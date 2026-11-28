//! Standalone `hiddify-health` CLI (`cmd/cmd_health.go` equivalent).

use std::env;
use std::process;

use hiddify_net::health_check::{
    run_health_checks, HealthCheckOptions, DEFAULT_HEALTH_DNS_SERVER, DEFAULT_HEALTH_TARGET,
};

fn parse_duration_ms(raw: &str) -> Result<u64, String> {
    let s = raw.trim();
    if let Some(ms) = s.strip_suffix("ms") {
        ms.parse::<u64>().map_err(|e| e.to_string())
    } else if let Some(sec) = s.strip_suffix('s') {
        let v: f64 = sec.parse().map_err(|e| format!("{e}"))?;
        Ok((v * 1000.0) as u64)
    } else if let Some(min) = s.strip_suffix('m') {
        let v: f64 = min.parse().map_err(|e| format!("{e}"))?;
        Ok((v * 60_000.0) as u64)
    } else {
        s.parse::<u64>().map_err(|e| e.to_string())
    }
}

fn print_usage() {
    eprintln!(
        "Usage: hiddify-health [flags] [target]\n\n\
         Test DNS, HTTP, or QUIC through socks:// or direct dial.\n\n\
         Flags:\n  \
           --proxy <url>        proxy URL (socks://[user:pass@]host:port)\n  \
           --dns                test DNS\n  \
           --http               test HTTP over TCP\n  \
           --quic               test QUIC over UDP\n  \
           --dns-server <addr>  DNS server (default 1.1.1.1:53)\n  \
           --timeout <dur>      timeout per check (default 15s)\n  \
           -v, --verbose        debug logs (-vv for more detail)"
    );
}

fn main() {
    let mut opts = HealthCheckOptions {
        target: DEFAULT_HEALTH_TARGET.to_string(),
        proxy_addr: String::new(),
        check_dns: false,
        check_http: false,
        check_quic: false,
        dns_server: DEFAULT_HEALTH_DNS_SERVER.to_string(),
        timeout_ms: 15_000,
        verbose: 0,
        quic_endpoint_override: None,
    };

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "--dns" => opts.check_dns = true,
            "--http" => opts.check_http = true,
            "--quic" => opts.check_quic = true,
            "-v" | "--verbose" => opts.verbose = opts.verbose.saturating_add(1),
            "-vv" => opts.verbose = opts.verbose.saturating_add(2),
            "--proxy" => {
                i += 1;
                opts.proxy_addr = args.get(i).cloned().unwrap_or_default();
            }
            "--dns-server" => {
                i += 1;
                opts.dns_server = args.get(i).cloned().unwrap_or_default();
            }
            "--timeout" => {
                i += 1;
                let raw = args.get(i).map(String::as_str).unwrap_or("15s");
                match parse_duration_ms(raw) {
                    Ok(ms) => opts.timeout_ms = ms,
                    Err(e) => {
                        eprintln!("invalid --timeout {raw}: {e}");
                        process::exit(1);
                    }
                }
            }
            arg if arg.starts_with("--proxy=") => {
                opts.proxy_addr = arg["--proxy=".len()..].to_string();
            }
            arg if arg.starts_with("--dns-server=") => {
                opts.dns_server = arg["--dns-server=".len()..].to_string();
            }
            arg if arg.starts_with("--timeout=") => {
                let raw = &arg["--timeout=".len()..];
                match parse_duration_ms(raw) {
                    Ok(ms) => opts.timeout_ms = ms,
                    Err(e) => {
                        eprintln!("invalid --timeout {raw}: {e}");
                        process::exit(1);
                    }
                }
            }
            arg if !arg.starts_with('-') => {
                opts.target = arg.to_string();
            }
            other => {
                eprintln!("unknown flag: {other}");
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    if !opts.check_dns && !opts.check_http && !opts.check_quic {
        opts.check_http = true;
    }

    let report = run_health_checks(&opts);
    for r in &report.results {
        let mark = if r.ok { "✓" } else { "✗" };
        if r.ok && !r.formatted_duration.is_empty() {
            println!("{}: {} {}", r.name, mark, r.formatted_duration);
        } else {
            println!("{}: {}", r.name, mark);
        }
        if opts.verbose >= 2 {
            if let Some(ref err) = r.error {
                eprintln!("debug: {}: {}", r.name, err);
            }
        }
    }
    if opts.verbose >= 2 && report.ok {
        eprintln!("debug: total duration={}", report.formatted_total_duration);
    }
    if let Some(err) = report.fatal_error {
        eprintln!("{err}");
        process::exit(1);
    }
}
