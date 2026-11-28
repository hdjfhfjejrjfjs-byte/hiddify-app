//! Standalone loopback self-signed certificate pair generator (`hiddify-core/cmd/cmd_gen_cert.go`).
//!
//! Generates `data/cert/server-{cert,key}.pem` and `data/cert/client-{cert,key}.pem` (with `--dir`
//! optional working directory override, skipping existing pairs unless `--force` is passed).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use hiddify_crypto::generate_certificate_file;

fn main() {
    let mut base_dir = PathBuf::from(".");
    let mut force = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" | "-d" => {
                let Some(val) = args.next() else {
                    eprintln!("gen-cert: --dir requires a path");
                    process::exit(2);
                };
                base_dir = PathBuf::from(val);
            }
            "--force" | "-f" => force = true,
            "--help" | "-h" => {
                println!("Usage: hiddify-gen-cert [--dir <path>] [--force]");
                return;
            }
            other => {
                eprintln!("gen-cert: unknown flag: {other}");
                process::exit(2);
            }
        }
    }

    let cert_dir = base_dir.join("cert");
    if let Err(err) = fs::create_dir_all(&cert_dir) {
        eprintln!("Error: {err}");
        process::exit(1);
    }

    let data_cert = base_dir.join("data").join("cert");
    let skip_if_exist = !force;
    for (cert_name, key_name) in [
        ("server-cert.pem", "server-key.pem"),
        ("client-cert.pem", "client-key.pem"),
    ] {
        if let Err(err) = generate_certificate_file(
            &data_cert.join(cert_name),
            &data_cert.join(key_name),
            skip_if_exist,
        ) {
            eprintln!("Error: {err}");
            process::exit(1);
        }
    }
}
