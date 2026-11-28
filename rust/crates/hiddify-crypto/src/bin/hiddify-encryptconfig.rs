//! Standalone build-time Psiphon config encryptor (`hiddify-core/cmd/internal/encryptconfig`).
//!
//! Reads `HIDDIFY_SECRET_KEY` (base64 AES-256 key) and `PSIPHON_CONFIG` (JSON or base64 JSON)
//! from the environment and prints `base64(nonce || AES-256-GCM("henc:" || json))` to stdout.

use std::env;
use std::io::{self, Write};
use std::process;

use hiddify_crypto::encrypt_henc_config;

fn main() {
    let key = env::var("HIDDIFY_SECRET_KEY").unwrap_or_default();
    let config = env::var("PSIPHON_CONFIG").unwrap_or_default();
    match encrypt_henc_config(&key, &config) {
        Ok(encrypted) => {
            let _ = io::stdout().write_all(encrypted.as_bytes());
            let _ = io::stdout().flush();
        }
        Err(err) => {
            eprintln!("encryptconfig: {err}");
            process::exit(1);
        }
    }
}
