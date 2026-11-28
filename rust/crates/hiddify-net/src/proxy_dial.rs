//! Pure-Rust SOCKS5 (TCP `CONNECT` + UDP `UDP ASSOCIATE`, RFC 1928 / RFC 1929) and
//! local HTTP proxy (`CONNECT` tunnel + `Proxy-Authorization: Basic`) dialers.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use url::Url;

const DIAL_STEP_MAX: Duration = Duration::from_secs(10);

fn remaining_io_timeout(deadline: Instant) -> io::Result<Duration> {
    let now = Instant::now();
    if now >= deadline {
        Err(io::Error::new(io::ErrorKind::TimedOut, "i/o timeout"))
    } else {
        Ok(deadline - now)
    }
}

pub fn connect_tcp_with_deadline(host: &str, port: u16, deadline: Instant) -> io::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("lookup {host}: no such host"),
        ));
    }
    let mut last_err: Option<io::Error> = None;
    for addr in addrs {
        let rem = remaining_io_timeout(deadline)?;
        match TcpStream::connect_timeout(&addr, rem.min(DIAL_STEP_MAX)) {
            Ok(stream) => {
                let rw = remaining_io_timeout(deadline)?;
                let _ = stream.set_read_timeout(Some(rw));
                let _ = stream.set_write_timeout(Some(rw));
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "connect failed")))
}

// ============================================================================
// Local HTTP Mixed Proxy (`127.0.0.1:<port>` with optional `hiddify:<secret>`)
// ============================================================================

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalHttpProxySpec {
    pub port: u16,
    pub secret: String,
}

impl LocalHttpProxySpec {
    pub fn is_enabled(&self) -> bool {
        self.port > 0
    }

    pub fn proxy_authorization_header(&self) -> Option<String> {
        if self.secret.is_empty() {
            None
        } else {
            let token = BASE64_STANDARD.encode(format!("hiddify:{}", self.secret));
            Some(format!("Basic {token}"))
        }
    }
}

/// Establishes an HTTP `CONNECT host:port HTTP/1.1` tunnel through `127.0.0.1:<proxy.port>`.
pub fn dial_http_proxy_connect(
    proxy: &LocalHttpProxySpec,
    target_host: &str,
    target_port: u16,
    user_agent: &str,
    deadline: Instant,
) -> io::Result<TcpStream> {
    let mut stream = connect_tcp_with_deadline("127.0.0.1", proxy.port, deadline)?;
    let authority = if target_host.contains(':') && !target_host.starts_with('[') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };

    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if !user_agent.is_empty() {
        req.push_str(&format!("User-Agent: {user_agent}\r\n"));
    }
    if let Some(auth) = proxy.proxy_authorization_header() {
        req.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    req.push_str("\r\n");

    stream.write_all(req.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    if reader.read_line(&mut status_line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "proxy closed connection during CONNECT",
        ));
    }
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    loop {
        let _ = remaining_io_timeout(deadline)?;
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    if !(200..300).contains(&code) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("proxy CONNECT failed with HTTP {code}"),
        ));
    }

    let inner = reader.into_inner();
    let rw = remaining_io_timeout(deadline)?;
    let _ = inner.set_read_timeout(Some(rw));
    let _ = inner.set_write_timeout(Some(rw));
    Ok(inner)
}

// ============================================================================
// SOCKS5 Client (TCP CONNECT & UDP ASSOCIATE, RFC 1928 + RFC 1929)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socks5Spec {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Socks5Spec {
    /// Parses `socks://[user:pass@]host:port`, `socks5://...`, or bare `host:port`
    /// (`cmd_health.go:newHealthDialer`).
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("empty proxy address".to_string());
        }
        let normalized = if trimmed.contains("://") {
            trimmed.to_string()
        } else {
            format!("socks://{trimmed}")
        };
        let url = Url::parse(&normalized).map_err(|e| format!("parse proxy URL: {e}"))?;
        match url.scheme() {
            "socks" | "socks5" | "socks5h" => {}
            other => return Err(format!("parse proxy URL: unsupported scheme {other}")),
        }
        let host = url
            .host_str()
            .ok_or_else(|| "parse proxy URL: missing host".to_string())?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let port = url
            .port()
            .ok_or_else(|| "parse proxy URL: missing port".to_string())?;
        let username = if url.username().is_empty() {
            None
        } else {
            Some(
                percent_encoding::percent_decode_str(url.username())
                    .decode_utf8_lossy()
                    .into_owned(),
            )
        };
        let password = url.password().map(|p| {
            percent_encoding::percent_decode_str(p)
                .decode_utf8_lossy()
                .into_owned()
        });
        Ok(Self {
            host,
            port,
            username,
            password,
        })
    }
}

fn socks5_authenticate(stream: &mut TcpStream, spec: &Socks5Spec) -> io::Result<()> {
    let has_auth = spec.username.is_some() || spec.password.is_some();
    if has_auth {
        stream.write_all(&[0x05, 0x02, 0x00, 0x02])?;
    } else {
        stream.write_all(&[0x05, 0x01, 0x00])?;
    }
    stream.flush()?;

    let mut method = [0u8; 2];
    stream.read_exact(&mut method)?;
    if method[0] != 0x05 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid SOCKS5 version"));
    }
    match method[1] {
        0x00 => Ok(()),
        0x02 => {
            let user = spec.username.as_deref().unwrap_or("");
            let pass = spec.password.as_deref().unwrap_or("");
            if user.len() > 255 || pass.len() > 255 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "SOCKS5 credentials exceed 255 bytes",
                ));
            }
            let mut auth_pkt = Vec::with_capacity(3 + user.len() + pass.len());
            auth_pkt.push(0x01);
            auth_pkt.push(user.len() as u8);
            auth_pkt.extend_from_slice(user.as_bytes());
            auth_pkt.push(pass.len() as u8);
            auth_pkt.extend_from_slice(pass.as_bytes());
            stream.write_all(&auth_pkt)?;
            stream.flush()?;

            let mut auth_resp = [0u8; 2];
            stream.read_exact(&mut auth_resp)?;
            if auth_resp[1] != 0x00 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "SOCKS5 authentication failed",
                ));
            }
            Ok(())
        }
        0xFF => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SOCKS5 no acceptable authentication method",
        )),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SOCKS5 unsupported auth method 0x{other:02x}"),
        )),
    }
}

fn encode_socks5_target(buf: &mut Vec<u8>, host: &str, port: u16) -> io::Result<()> {
    let clean_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = clean_host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v4) => {
                buf.push(0x01);
                buf.extend_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                buf.push(0x04);
                buf.extend_from_slice(&v6.octets());
            }
        }
    } else {
        if clean_host.len() > 255 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SOCKS5 domain name too long",
            ));
        }
        buf.push(0x03);
        buf.push(clean_host.len() as u8);
        buf.extend_from_slice(clean_host.as_bytes());
    }
    buf.extend_from_slice(&port.to_be_bytes());
    Ok(())
}

fn read_socks5_reply(stream: &mut TcpStream) -> io::Result<(Option<IpAddr>, u16)> {
    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr)?;
    if hdr[0] != 0x05 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid SOCKS5 reply version"));
    }
    if hdr[1] != 0x00 {
        let kind = match hdr[1] {
            0x02 => io::ErrorKind::PermissionDenied,
            0x03 | 0x04 => io::ErrorKind::HostUnreachable,
            0x05 => io::ErrorKind::ConnectionRefused,
            _ => io::ErrorKind::Other,
        };
        return Err(io::Error::new(
            kind,
            format!("SOCKS5 request rejected with code 0x{:02x}", hdr[1]),
        ));
    }
    let ip = match hdr[3] {
        0x01 => {
            let mut b = [0u8; 4];
            stream.read_exact(&mut b)?;
            Some(IpAddr::V4(Ipv4Addr::from(b)))
        }
        0x04 => {
            let mut b = [0u8; 16];
            stream.read_exact(&mut b)?;
            Some(IpAddr::V6(Ipv6Addr::from(b)))
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len)?;
            let mut dom = vec![0u8; len[0] as usize];
            stream.read_exact(&mut dom)?;
            None
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("SOCKS5 invalid ATYP 0x{other:02x}"),
            ))
        }
    };
    let mut port_bytes = [0u8; 2];
    stream.read_exact(&mut port_bytes)?;
    Ok((ip, u16::from_be_bytes(port_bytes)))
}

/// Dials a TCP stream to `target_host:target_port` via SOCKS5 `CONNECT`.
pub fn dial_socks5_tcp(
    spec: &Socks5Spec,
    target_host: &str,
    target_port: u16,
    deadline: Instant,
) -> io::Result<TcpStream> {
    let mut stream = connect_tcp_with_deadline(&spec.host, spec.port, deadline)?;
    socks5_authenticate(&mut stream, spec)?;

    let mut req = Vec::with_capacity(32);
    req.extend_from_slice(&[0x05, 0x01, 0x00]); // VER=5, CMD=CONNECT, RSV=0
    encode_socks5_target(&mut req, target_host, target_port)?;
    stream.write_all(&req)?;
    stream.flush()?;

    let _ = read_socks5_reply(&mut stream)?;
    let rw = remaining_io_timeout(deadline)?;
    let _ = stream.set_read_timeout(Some(rw));
    let _ = stream.set_write_timeout(Some(rw));
    Ok(stream)
}

/// Connected UDP-like datagram channel (either direct `UdpSocket` or SOCKS5 `UDP ASSOCIATE`).
pub enum DatagramChannel {
    Direct {
        socket: UdpSocket,
    },
    Socks5 {
        _control: TcpStream,
        socket: UdpSocket,
        relay_addr: SocketAddr,
        target_host: String,
        target_port: u16,
    },
}

impl DatagramChannel {
    pub fn dial_direct(target_host: &str, target_port: u16, deadline: Instant) -> io::Result<Self> {
        let _ = remaining_io_timeout(deadline)?;
        let remote = (target_host, target_port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no UDP address"))?;
        let bind_addr = if remote.is_ipv6() {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
        };
        let socket = UdpSocket::bind(bind_addr)?;
        socket.connect(remote)?;
        let rw = remaining_io_timeout(deadline)?;
        socket.set_read_timeout(Some(rw))?;
        socket.set_write_timeout(Some(rw))?;
        Ok(Self::Direct { socket })
    }

    pub fn dial_socks5(
        spec: &Socks5Spec,
        target_host: &str,
        target_port: u16,
        deadline: Instant,
    ) -> io::Result<Self> {
        let mut control = connect_tcp_with_deadline(&spec.host, spec.port, deadline)?;
        let peer_ip = control.peer_addr()?.ip();
        socks5_authenticate(&mut control, spec)?;

        // VER=5, CMD=UDP ASSOCIATE (0x03), RSV=0, ATYP=IPv4 0.0.0.0:0
        control.write_all(&[0x05, 0x03, 0x00, 0x01, 0, 0, 0, 0, 0, 0])?;
        control.flush()?;

        let (bnd_ip_opt, bnd_port) = read_socks5_reply(&mut control)?;
        let relay_ip = match bnd_ip_opt {
            Some(ip) if !ip.is_unspecified() => ip,
            _ => peer_ip,
        };
        let relay_addr = SocketAddr::new(relay_ip, bnd_port);
        let bind_addr = if relay_addr.is_ipv6() {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
        };
        let socket = UdpSocket::bind(bind_addr)?;
        socket.connect(relay_addr)?;
        let rw = remaining_io_timeout(deadline)?;
        socket.set_read_timeout(Some(rw))?;
        socket.set_write_timeout(Some(rw))?;

        Ok(Self::Socks5 {
            _control: control,
            socket,
            relay_addr,
            target_host: target_host.to_string(),
            target_port,
        })
    }

    pub fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Direct { socket } | Self::Socks5 { socket, .. } => socket.set_read_timeout(dur),
        }
    }

    pub fn set_write_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Direct { socket } | Self::Socks5 { socket, .. } => socket.set_write_timeout(dur),
        }
    }

    pub fn send(&self, payload: &[u8]) -> io::Result<usize> {
        match self {
            Self::Direct { socket } => socket.send(payload),
            Self::Socks5 {
                socket,
                relay_addr: _,
                target_host,
                target_port,
                ..
            } => {
                let mut pkt = Vec::with_capacity(payload.len() + 32);
                pkt.extend_from_slice(&[0x00, 0x00, 0x00]); // RSV=0x0000, FRAG=0x00
                encode_socks5_target(&mut pkt, target_host, *target_port)?;
                pkt.extend_from_slice(payload);
                socket.send(&pkt)?;
                Ok(payload.len())
            }
        }
    }

    pub fn recv(&self, out: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Direct { socket } => socket.recv(out),
            Self::Socks5 { socket, .. } => {
                let mut buf = vec![0u8; out.len() + 300];
                let n = socket.recv(&mut buf)?;
                if n < 7 || buf[2] != 0x00 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid SOCKS5 UDP datagram header",
                    ));
                }
                let hdr_len = match buf[3] {
                    0x01 => 4 + 4 + 2,
                    0x04 => 4 + 16 + 2,
                    0x03 => {
                        if n < 5 {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "truncated SOCKS5 UDP domain",
                            ));
                        }
                        4 + 1 + (buf[4] as usize) + 2
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unsupported SOCKS5 UDP ATYP",
                        ))
                    }
                };
                if n < hdr_len {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "truncated SOCKS5 UDP header",
                    ));
                }
                let data_len = (n - hdr_len).min(out.len());
                out[..data_len].copy_from_slice(&buf[hdr_len..hdr_len + data_len]);
                Ok(data_len)
            }
        }
    }
}
