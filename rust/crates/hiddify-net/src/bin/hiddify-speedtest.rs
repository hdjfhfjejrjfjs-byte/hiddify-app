//! Standalone `hiddify-speedtest` CLI (`cmd/cmd_speedtest.go` equivalent).

use std::env;
use std::process;

use hiddify_net::speedtest::{run_speedtest, SpeedtestOptions};

fn print_usage() {
    eprintln!(
        "Usage: hiddify-speedtest [flags]\n\n\
         Run speedtest.net download/upload/ping/jitter through a proxy.\n\n\
         Flags:\n  \
           --proxy <url>      proxy URL (socks://[user:pass@]host:port)\n  \
           --distance <km>    target server distance in km (default 2000)\n  \
           --no-download      disable download test\n  \
           --no-upload        disable upload test\n  \
           --no-ping          disable ping test\n  \
           --no-jitter        disable jitter output\n  \
           -v, --verbose      debug logs (-vv for more detail)"
    );
}

fn main() {
    let mut opts = SpeedtestOptions::default();
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "--no-download" => opts.no_download = true,
            "--no-upload" => opts.no_upload = true,
            "--no-ping" => opts.no_ping = true,
            "--no-jitter" => opts.no_jitter = true,
            "-v" | "--verbose" => opts.verbose = opts.verbose.saturating_add(1),
            "-vv" => opts.verbose = opts.verbose.saturating_add(2),
            "--proxy" => {
                i += 1;
                opts.proxy_addr = args.get(i).cloned().unwrap_or_default();
            }
            "--distance" => {
                i += 1;
                let raw = args.get(i).map(String::as_str).unwrap_or("2000");
                match raw.parse::<f64>() {
                    Ok(km) => opts.target_distance_km = km,
                    Err(e) => {
                        eprintln!("invalid --distance {raw}: {e}");
                        process::exit(1);
                    }
                }
            }
            arg if arg.starts_with("--proxy=") => {
                opts.proxy_addr = arg["--proxy=".len()..].to_string();
            }
            arg if arg.starts_with("--distance=") => {
                let raw = &arg["--distance=".len()..];
                match raw.parse::<f64>() {
                    Ok(km) => opts.target_distance_km = km,
                    Err(e) => {
                        eprintln!("invalid --distance {raw}: {e}");
                        process::exit(1);
                    }
                }
            }
            other => {
                eprintln!("unknown flag: {other}");
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    let report = run_speedtest(&opts);
    for step in &report.steps {
        let mark = if step.ok { "✓" } else { "✗" };
        if !step.value.is_empty() {
            println!("{}: {} {}", step.name, mark, step.value);
        } else {
            println!("{}: {}", step.name, mark);
        }
    }
    if let Some(err) = report.fatal_error {
        eprintln!("{err}");
        process::exit(1);
    }
}
