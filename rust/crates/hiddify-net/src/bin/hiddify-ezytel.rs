//! Standalone Ezytel CLI & `h2c` gRPC server (`hiddify-core/cmd/cmd_ezytel.go`).
//!
//! Subcommands:
//!   - `info`
//!   - `messages`
//!   - `proxy-image`
//!   - `parse-channels`
//!   - `serve`

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use hiddify_net::ezytel::{
    serve_ezytel_grpc_on_listener, ChannelInfoRequest, ChannelMessagesRequest, EzytelService,
    ParseChannelsRequest, ProxyImageRequest,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut raw_args: Vec<String> = env::args().skip(1).collect();
    if raw_args.first().map(|s| s.as_str()) == Some("ezytel") {
        raw_args.remove(0);
    }
    if raw_args.is_empty() || matches!(raw_args[0].as_str(), "-h" | "--help" | "help") {
        print_usage();
        return Ok(());
    }

    let mut cache_dir = String::new();
    let mut subcmd = String::new();
    let mut channel_id = String::new();
    let mut last_read: i64 = 0;
    let mut before: i64 = 0;
    let mut disable_inline_images = false;
    let mut json_out = false;
    let mut html_only = false;
    let mut hex_url = String::new();
    let mut output_file = String::new();
    let mut raw_list = String::new();
    let mut raw_file = String::new();
    let mut listen_addr = "127.0.0.1:17079".to_string();

    let mut i = 0usize;
    while i < raw_args.len() {
        let arg = &raw_args[i];
        match arg.as_str() {
            "info" | "messages" | "proxy-image" | "parse-channels" | "serve" if subcmd.is_empty() => {
                subcmd = arg.clone();
            }
            "--cache-dir" => {
                i += 1;
                cache_dir = raw_args.get(i).cloned().ok_or("--cache-dir requires a value")?;
            }
            "--channel" => {
                i += 1;
                channel_id = raw_args.get(i).cloned().ok_or("--channel requires a value")?;
            }
            "--last-read" => {
                i += 1;
                last_read = raw_args
                    .get(i)
                    .ok_or("--last-read requires an integer")?
                    .parse()
                    .map_err(|e| format!("invalid --last-read: {e}"))?;
            }
            "--before" => {
                i += 1;
                before = raw_args
                    .get(i)
                    .ok_or("--before requires an integer")?
                    .parse()
                    .map_err(|e| format!("invalid --before: {e}"))?;
            }
            "--disable-inline-images" => disable_inline_images = true,
            "--json" => json_out = true,
            "--html-only" => html_only = true,
            "--hex" => {
                i += 1;
                hex_url = raw_args.get(i).cloned().ok_or("--hex requires a value")?;
            }
            "--output" | "-o" => {
                i += 1;
                output_file = raw_args.get(i).cloned().ok_or("--output requires a path")?;
            }
            "--raw" => {
                i += 1;
                raw_list = raw_args.get(i).cloned().ok_or("--raw requires a value")?;
            }
            "--file" | "-f" => {
                i += 1;
                raw_file = raw_args.get(i).cloned().ok_or("--file requires a path")?;
            }
            "--listen" => {
                i += 1;
                listen_addr = raw_args.get(i).cloned().ok_or("--listen requires host:port")?;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }

    let svc = Arc::new(EzytelService::new(&cache_dir));

    match subcmd.as_str() {
        "info" => {
            if channel_id.is_empty() {
                return Err("--channel is required".to_string());
            }
            let resp = svc.get_channel_info(&ChannelInfoRequest {
                channel_id,
                last_read,
                disable_inline_images,
            })?;
            if json_out {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())?
                );
            } else {
                println!("name: {}", resp.name);
                println!("description: {}", resp.description);
                println!("avatar_path: {}", resp.avatar_path);
                println!("date: {}", resp.date);
                println!("date_str: {}", resp.date_str);
                println!("newmsg: {}", resp.newmsg);
                println!("last_post_id: {}", resp.last_post_id);
                println!("ok: {}", resp.ok);
            }
        }
        "messages" => {
            if channel_id.is_empty() {
                return Err("--channel is required".to_string());
            }
            let resp = svc.get_channel_messages(&ChannelMessagesRequest {
                channel_id,
                before,
                disable_inline_images,
            })?;
            if html_only {
                print!("{}", resp.html);
            } else if json_out {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())?
                );
            } else {
                println!("last_post_id: {}", resp.last_post_id);
                println!("channel_avatar: {}", resp.channel_avatar);
                println!("html_bytes: {}", resp.html.len());
                print!("{}", resp.html);
            }
        }
        "proxy-image" => {
            if hex_url.is_empty() {
                return Err("--hex is required".to_string());
            }
            let resp = svc.proxy_image(&ProxyImageRequest { hex_url })?;
            if json_out {
                let out = serde_json::json!({
                    "content_type": resp.content_type,
                    "cache_name": resp.cache_name,
                    "data": BASE64_STANDARD.encode(&resp.data),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
                );
            } else if !output_file.is_empty() {
                fs::write(&output_file, &resp.data).map_err(|e| e.to_string())?;
            } else {
                io::stdout()
                    .write_all(&resp.data)
                    .map_err(|e| e.to_string())?;
            }
        }
        "parse-channels" => {
            let mut raw = raw_list;
            if !raw_file.is_empty() {
                raw = fs::read_to_string(&raw_file).map_err(|e| e.to_string())?;
            }
            if raw.is_empty() {
                let mut stdin_buf = String::new();
                io::stdin()
                    .read_to_string(&mut stdin_buf)
                    .map_err(|e| e.to_string())?;
                raw = stdin_buf;
            }
            let resp = svc.parse_channels(&ParseChannelsRequest { raw })?;
            if json_out {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&resp).map_err(|e| e.to_string())?
                );
            } else {
                for id in resp.channel_ids {
                    println!("{id}");
                }
            }
        }
        "serve" => {
            let listener =
                TcpListener::bind(&listen_addr).map_err(|e| format!("listen {listen_addr}: {e}"))?;
            let bound = listener.local_addr().map_err(|e| e.to_string())?;
            eprintln!("ezytel gRPC listening on {bound}");
            let stop = install_signal_flag();
            serve_ezytel_grpc_on_listener(listener, svc, stop).map_err(|e| e.to_string())?;
        }
        _ => {
            print_usage();
            return Err("missing subcommand".to_string());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn install_signal_flag() -> Arc<AtomicBool> {
    static STOP: AtomicBool = AtomicBool::new(false);
    extern "C" fn on_sig(_sig: libc::c_int) {
        STOP.store(true, Ordering::Release);
    }
    unsafe {
        libc::signal(libc::SIGINT, on_sig as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_sig as *const () as libc::sighandler_t);
    }
    let out = Arc::new(AtomicBool::new(false));
    let out_bg = out.clone();
    std::thread::spawn(move || {
        while !STOP.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        out_bg.store(true, Ordering::Release);
    });
    out
}

#[cfg(not(unix))]
fn install_signal_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn print_usage() {
    println!(
        "Telegram public-channel viewer (EzytelService)\n\n\
         Usage:\n  \
           hiddify-ezytel [--cache-dir DIR] <info|messages|proxy-image|parse-channels|serve> [flags]"
    );
}
