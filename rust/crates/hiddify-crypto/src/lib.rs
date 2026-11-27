//! Independent cryptographic utilities for mTLS client identity management,
//! certificate pinning, unambiguous secret generation, and streaming SHA-256 verification.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::{
    STANDARD as BASE64_STANDARD, URL_SAFE as BASE64_URL_SAFE,
    URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD,
};
use base64::Engine;
use der::asn1::OctetString;
use der::pem::LineEnding;
use der::{DateTime as DerDateTime, Encode, EncodePem};
use rand::rngs::OsRng;
use rand::Rng;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spki::SubjectPublicKeyInfoOwned;
use thiserror::Error;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{ExtendedKeyUsage, SubjectAltName};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::{Time, Validity};

/// Default validity period in days for generated mTLS client identities (10 years).
pub const DEFAULT_VALIDITY_DAYS: u32 = 3650;

/// Default renewal window in seconds before certificate expiry (30 days).
pub const DEFAULT_RENEW_BEFORE_SECS: u64 = 30 * 86_400;

/// Default client certificate file name.
pub const CERTIFICATE_FILE: &str = "client.crt";

/// Default client private key file name (PKCS#8 PEM).
pub const PRIVATE_KEY_FILE: &str = "client.key";

/// Unambiguous alphabet omitting visually similar characters (`0`/`O`, `1`/`l`/`I`).
const SECRET_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

/// Full 62-character ASCII alphanumeric alphabet used for local core gRPC secrets.
const ALPHANUMERIC_ALPHABET: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// OID `1.3.6.1.5.5.7.3.2` (`id-kp-clientAuth`).
const OID_KP_CLIENT_AUTH: der::oid::ObjectIdentifier =
    der::oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.2");

/// OID `1.3.6.1.5.5.7.3.1` (`id-kp-serverAuth`).
const OID_KP_SERVER_AUTH: der::oid::ObjectIdentifier =
    der::oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.1");

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("RSA key generation failed: {0}")]
    KeyGen(String),
    #[error("PKCS#8 PEM encoding failed: {0}")]
    Pkcs8Encode(String),
    #[error("X.509 certificate build failed: {0}")]
    CertBuild(String),
    #[error("Invalid PEM certificate: {0}")]
    InvalidPem(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Self-signed client certificate and PKCS#8 private key pair in PEM format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MtlsIdentity {
    pub certificate_pem: String,
    pub private_key_pem: String,
}

impl MtlsIdentity {
    /// Generates a new 2048-bit RSA keypair and self-signed X.509 v3 client certificate
    /// with `CN=HiddifyClient` and `ExtendedKeyUsage::ClientAuth`.
    pub fn generate() -> Result<Self, CryptoError> {
        Self::generate_with_options("HiddifyClient", DEFAULT_VALIDITY_DAYS, false, &[])
    }

    /// Generates an RSA-2048 identity with custom CN, validity days, optional `ServerAuth`, and SANs.
    pub fn generate_with_options(
        common_name: &str,
        validity_days: u32,
        include_server_auth: bool,
        sans: &[&str],
    ) -> Result<Self, CryptoError> {
        let mut rng = OsRng;
        let private_key =
            RsaPrivateKey::new(&mut rng, 2048).map_err(|e| CryptoError::KeyGen(e.to_string()))?;
        let public_key = private_key.to_public_key();

        let private_key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| CryptoError::Pkcs8Encode(e.to_string()))?
            .to_string();

        let signing_key = SigningKey::<Sha256>::new(private_key);
        let spki = SubjectPublicKeyInfoOwned::from_key(public_key)
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;

        let subject = Name::from_str(&format!("CN={common_name}"))
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(1_700_000_000))
            .as_secs();
        let end_secs = now_secs.saturating_add(u64::from(validity_days) * 86_400);

        let not_before = DerDateTime::from_unix_duration(Duration::from_secs(now_secs))
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;
        let not_after = DerDateTime::from_unix_duration(Duration::from_secs(end_secs))
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;
        let validity = Validity {
            not_before: Time::GeneralTime(der::asn1::GeneralizedTime::from_date_time(not_before)),
            not_after: Time::GeneralTime(der::asn1::GeneralizedTime::from_date_time(not_after)),
        };

        let mut serial_bytes = [0u8; 16];
        rng.fill(&mut serial_bytes);
        serial_bytes[0] &= 0x7F;
        if serial_bytes.iter().all(|&b| b == 0) {
            serial_bytes[15] = 1;
        }
        let serial = SerialNumber::new(&serial_bytes)
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;

        let mut builder = CertificateBuilder::new(
            Profile::Leaf {
                issuer: subject.clone(),
                enable_key_agreement: false,
                enable_key_encipherment: true,
            },
            serial,
            validity,
            subject,
            spki,
            &signing_key,
        )
        .map_err(|e| CryptoError::CertBuild(e.to_string()))?;

        let mut eku_oids = vec![OID_KP_CLIENT_AUTH];
        if include_server_auth {
            eku_oids.push(OID_KP_SERVER_AUTH);
        }
        builder
            .add_extension(&ExtendedKeyUsage(eku_oids))
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;

        if !sans.is_empty() {
            let mut names = Vec::new();
            for &san in sans {
                if let Ok(ip) = san.parse::<std::net::IpAddr>() {
                    let octets = match ip {
                        std::net::IpAddr::V4(v4) => OctetString::new(v4.octets().to_vec()),
                        std::net::IpAddr::V6(v6) => OctetString::new(v6.octets().to_vec()),
                    }
                    .map_err(|e| CryptoError::CertBuild(e.to_string()))?;
                    names.push(GeneralName::IpAddress(octets));
                } else if let Ok(ia5) = der::asn1::Ia5String::new(san) {
                    names.push(GeneralName::DnsName(ia5));
                }
            }
            if !names.is_empty() {
                builder
                    .add_extension(&SubjectAltName(names))
                    .map_err(|e| CryptoError::CertBuild(e.to_string()))?;
            }
        }

        let cert = builder
            .build()
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;
        let certificate_pem = cert
            .to_pem(LineEnding::LF)
            .map_err(|e| CryptoError::CertBuild(e.to_string()))?;

        Ok(Self {
            certificate_pem,
            private_key_pem,
        })
    }

    /// Checks whether the PEM certificate and PKCS#8 private key are structurally valid
    /// and `notAfter` is strictly after `now + renew_before_secs`.
    pub fn is_usable(&self, renew_before_secs: u64) -> bool {
        is_identity_usable(&self.certificate_pem, &self.private_key_pem, renew_before_secs)
    }

    /// Loads an existing usable identity from `directory`, or generates and atomically writes
    /// a fresh identity (`client.crt` and `client.key`).
    pub fn load_or_create(directory: &Path) -> Result<Self, CryptoError> {
        let cert_path = directory.join(CERTIFICATE_FILE);
        let key_path = directory.join(PRIVATE_KEY_FILE);

        if cert_path.exists() && key_path.exists() {
            if let (Ok(certificate_pem), Ok(private_key_pem)) = (
                fs::read_to_string(&cert_path),
                fs::read_to_string(&key_path),
            ) {
                let existing = Self {
                    certificate_pem,
                    private_key_pem,
                };
                if existing.is_usable(DEFAULT_RENEW_BEFORE_SECS) {
                    return Ok(existing);
                }
            }
        }

        let identity = Self::generate()?;
        fs::create_dir_all(directory)?;
        atomic_write(&key_path, &identity.private_key_pem)?;
        atomic_write(&cert_path, &identity.certificate_pem)?;
        Ok(identity)
    }
}

fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let mut tmp_os = path.as_os_str().to_os_string();
    tmp_os.push(".tmp");
    let tmp_path = std::path::PathBuf::from(tmp_os);
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, path)
}

/// Validates that `certificate_pem` and `private_key_pem` contain PEM headers and
/// the X.509 certificate expires strictly after `now + renew_before_secs`.
pub fn is_identity_usable(
    certificate_pem: &str,
    private_key_pem: &str,
    renew_before_secs: u64,
) -> bool {
    if !certificate_pem.contains("-----BEGIN CERTIFICATE-----")
        || !private_key_pem.contains("-----BEGIN PRIVATE KEY-----")
    {
        return false;
    }

    let der_bytes = match pem_to_der(certificate_pem) {
        Ok(b) if !b.is_empty() => b,
        _ => return false,
    };

    let Ok((_, cert)) = x509_parser::parse_x509_certificate(&der_bytes) else {
        return false;
    };

    let not_after_ts = cert.validity().not_after.timestamp();
    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let threshold = now_ts.saturating_add(renew_before_secs as i64);

    not_after_ts > threshold
}

/// Strips `-----BEGIN CERTIFICATE-----` and `-----END CERTIFICATE-----` markers and all whitespace.
pub fn pem_body(pem: &str) -> String {
    pem.replace("-----BEGIN CERTIFICATE-----", "")
        .replace("-----END CERTIFICATE-----", "")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Decodes a PEM-encoded X.509 certificate into raw DER bytes.
pub fn pem_to_der(pem: &str) -> Result<Vec<u8>, CryptoError> {
    if !pem.contains("-----BEGIN CERTIFICATE-----") {
        return Err(CryptoError::InvalidPem(
            "Certificate must contain BEGIN CERTIFICATE header".to_string(),
        ));
    }
    let body = pem_body(pem);
    BASE64_STANDARD
        .decode(body.as_bytes())
        .map_err(|e| CryptoError::InvalidPem(e.to_string()))
}

/// Mirrors `MTLSChannelCredentials._isLoopbackHost`:
/// splits `host` on `:`, takes the first segment, removes `[` and `]`, and checks loopback names.
pub fn is_loopback_host(host: &str) -> bool {
    let first = host.split(':').next().unwrap_or("");
    let name: String = first.chars().filter(|&c| c != '[' && c != ']').collect();
    name == "127.0.0.1" || name == "localhost" || name == "::1"
}

/// Verifies a presented server certificate against the pinned server certificate
/// (exact DER match first, falling back to normalized PEM body match) for a loopback host.
pub fn verify_pinned_certificate(
    presented_der: &[u8],
    presented_pem: &str,
    expected_server_pem: &str,
    host: &str,
) -> bool {
    if !is_loopback_host(host) {
        return false;
    }
    let Ok(expected_der) = pem_to_der(expected_server_pem) else {
        return false;
    };
    if !presented_der.is_empty() && presented_der == expected_der.as_slice() {
        return true;
    }
    let expected_body = pem_body(expected_server_pem);
    let presented_body = pem_body(presented_pem);
    !presented_body.is_empty() && presented_body == expected_body
}

/// Generates an unambiguous proxy secret grouped into 4-character dash-separated blocks
/// (e.g. `Kp7x-Q2mZ` for `length = 8`), matching `generateProxySecret` in Dart.
pub fn generate_proxy_secret(length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    let mut rng = OsRng;
    let raw: Vec<u8> = (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..SECRET_ALPHABET.len());
            SECRET_ALPHABET[idx]
        })
        .collect();

    raw.chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("-")
}

/// Checks whether a release asset digest string has the exact `sha256:<64 hex chars>` format.
pub fn is_valid_sha256_digest(digest: &str) -> bool {
    if digest.len() != 71 || !digest.starts_with("sha256:") {
        return false;
    }
    digest[7..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Computes the lowercase hex SHA-256 of a file using a fixed 64 KiB streaming buffer.
pub fn sha256_file_hex(path: &Path) -> Result<(u64, String), CryptoError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65_536];
    let mut total_bytes: u64 = 0;

    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        total_bytes += read as u64;
        hasher.update(&buf[..read]);
    }

    let digest_bytes = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest_bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Ok((total_bytes, hex))
}

/// Verifies that `path` matches `expected_size` (if `expected_size > 0`) and `expected_digest` (`sha256:...`).
pub fn verify_file_sha256(path: &Path, expected_size: u64, expected_digest: &str) -> bool {
    if !is_valid_sha256_digest(expected_digest) {
        return false;
    }
    if expected_size > 0 {
        let Ok(meta) = fs::metadata(path) else {
            return false;
        };
        if meta.len() != expected_size {
            return false;
        }
    }
    let Ok((actual_size, actual_hex)) = sha256_file_hex(path) else {
        return false;
    };
    if expected_size > 0 && actual_size != expected_size {
        return false;
    }
    actual_hex == expected_digest[7..].to_ascii_lowercase()
}

/// Generates a CSPRNG numeric PIN of `length` digits (`0..=9`), matching `PhoneImportServer._newToken`.
pub fn generate_numeric_pin(length: usize) -> String {
    let mut rng = OsRng;
    (0..length)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect()
}

/// Generates `byte_len` CSPRNG random bytes encoded with standard URL-safe Base64,
/// matching `LanFileServer._newToken` (`base64Url.encode(18 random bytes)`).
pub fn generate_url_token(byte_len: usize) -> String {
    let mut rng = OsRng;
    let mut bytes = vec![0u8; byte_len];
    rng.fill(&mut bytes[..]);
    BASE64_URL_SAFE.encode(&bytes)
}

/// Generates a CSPRNG alphanumeric secret of `length` characters (`[a-zA-Z0-9]`),
/// upgrading `CoreInterfaceDesktop.generateRandomPassword` to OS CSPRNG.
pub fn generate_alphanumeric_secret(length: usize) -> String {
    let mut rng = OsRng;
    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..ALPHANUMERIC_ALPHABET.len());
            char::from(ALPHANUMERIC_ALPHABET[idx])
        })
        .collect()
}

/// Compares `path` against `"/{token}"` in constant time (`PhoneImportServer._isOwnPath`).
pub fn verify_secret_path(path: &str, token: &str) -> bool {
    let expected = format!("/{token}");
    let p_bytes = path.as_bytes();
    let e_bytes = expected.as_bytes();
    if p_bytes.len() != e_bytes.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&a, &b) in p_bytes.iter().zip(e_bytes.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// VLESS X25519 (`mlkem768x25519plus`) server decryption / client encryption config pair,
/// matching `hiddify-core/cmd/cmd_vlessenc.go`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlessEncryptionPair {
    pub decryption: String,
    pub encryption: String,
}

/// Derives a clamped X25519 VLESS encryption/decryption pair from a 32-byte private key seed.
pub fn generate_vless_encryption_from_seed(mut private_key: [u8; 32]) -> VlessEncryptionPair {
    private_key[0] &= 248;
    private_key[31] &= 127;
    private_key[31] |= 64;

    let public_key = x25519_dalek::x25519(private_key, x25519_dalek::X25519_BASEPOINT_BYTES);
    let server_key = BASE64_URL_SAFE_NO_PAD.encode(private_key);
    let client_key = BASE64_URL_SAFE_NO_PAD.encode(public_key);

    VlessEncryptionPair {
        decryption: format!("mlkem768x25519plus.native.600s.{server_key}"),
        encryption: format!("mlkem768x25519plus.native.0rtt.{client_key}"),
    }
}

/// Generates a fresh CSPRNG X25519 VLESS encryption/decryption pair.
pub fn generate_vless_encryption_pair() -> VlessEncryptionPair {
    let mut seed = [0u8; 32];
    OsRng.fill(&mut seed);
    generate_vless_encryption_from_seed(seed)
}

#[allow(dead_code)]
fn _assert_der_encode_trait<T: Encode>() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_created_once_and_reused_until_corrupted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = MtlsIdentity::load_or_create(dir.path()).expect("first load_or_create");
        let second = MtlsIdentity::load_or_create(dir.path()).expect("second load_or_create");

        assert!(first.certificate_pem.contains("-----BEGIN CERTIFICATE-----"));
        assert!(first.private_key_pem.contains("-----BEGIN PRIVATE KEY-----"));
        assert_eq!(first, second);
        assert!(dir.path().join(CERTIFICATE_FILE).exists());
        assert!(dir.path().join(PRIVATE_KEY_FILE).exists());

        // Corrupting the private key triggers automatic replacement
        fs::write(dir.path().join(PRIVATE_KEY_FILE), "garbage").expect("write garbage");
        let third = MtlsIdentity::load_or_create(dir.path()).expect("third load_or_create");
        assert_ne!(first.certificate_pem, third.certificate_pem);
        assert!(third.is_usable(DEFAULT_RENEW_BEFORE_SECS));
    }

    #[test]
    fn pinned_certificate_verification_matches_mtls_rules() {
        let server = MtlsIdentity::generate_with_options(
            "127.0.0.1",
            365,
            true,
            &["127.0.0.1", "localhost"],
        )
        .expect("server cert");
        let der = pem_to_der(&server.certificate_pem).expect("der");

        assert!(verify_pinned_certificate(
            &der,
            &server.certificate_pem,
            &server.certificate_pem,
            "127.0.0.1:17078"
        ));
        // Even if DER bytes differ due to OS re-encoding, matching PEM body succeeds
        assert!(verify_pinned_certificate(
            b"re-encoded-der",
            &server.certificate_pem,
            &server.certificate_pem,
            "localhost"
        ));
        // Non-loopback host is rejected
        assert!(!verify_pinned_certificate(
            &der,
            &server.certificate_pem,
            &server.certificate_pem,
            "example.com:443"
        ));
    }

    #[test]
    fn proxy_secret_format_and_alphabet() {
        let s8 = generate_proxy_secret(8);
        assert_eq!(s8.len(), 9);
        assert_eq!(&s8[4..5], "-");

        let s10 = generate_proxy_secret(10);
        assert_eq!(s10.len(), 12);
        assert_eq!(&s10[4..5], "-");
        assert_eq!(&s10[9..10], "-");

        for _ in 0..200 {
            let s = generate_proxy_secret(8);
            assert!(!s.chars().any(|c| matches!(c, '0' | '1' | 'O' | 'I' | 'l')));
        }
    }

    #[test]
    fn sha256_digest_verification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("payload.bin");
        fs::write(&file, b"trusted update").expect("write");
        let valid = "sha256:3e815a6f9d8c44b0c2fc5d26f77545bb0d4378f1693d3dc425e654261f8780fc";

        assert!(verify_file_sha256(&file, 14, valid));
        assert!(verify_file_sha256(&file, 0, &valid.to_uppercase().replace("SHA256:", "sha256:")));
        assert!(!verify_file_sha256(&file, 15, valid));
        assert!(!verify_file_sha256(&file, 14, "md5:3e815a6f9d8c44b0c2fc5d26f77545bb0d4378f1693d3dc425e654261f8780fc"));
    }

    #[test]
    fn tokens_and_constant_time_secret_path() {
        let pin = generate_numeric_pin(6);
        assert_eq!(pin.len(), 6);
        assert!(pin.bytes().all(|b| b.is_ascii_digit()));

        let url_tok = generate_url_token(18);
        assert_eq!(url_tok.len(), 24);

        let alpha = generate_alphanumeric_secret(100);
        assert_eq!(alpha.len(), 100);
        assert!(alpha.bytes().all(|b| b.is_ascii_alphanumeric()));

        assert!(verify_secret_path(&format!("/{pin}"), &pin));
        assert!(!verify_secret_path("/000000x", "000000"));
        assert!(!verify_secret_path("/", &pin));
    }

    #[test]
    fn vless_encryption_pair_matches_rfc7748_and_go_vlessenc() {
        // RFC 7748 Section 6.1 test vector scalar:
        let seed: [u8; 32] = [
            0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2,
            0x66, 0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5,
            0x1d, 0xb9, 0x2c, 0x2a,
        ];
        let pair = generate_vless_encryption_from_seed(seed);
        assert!(pair.decryption.starts_with("mlkem768x25519plus.native.600s."));
        assert!(pair.encryption.starts_with("mlkem768x25519plus.native.0rtt."));
        let pub_b64 = pair
            .encryption
            .strip_prefix("mlkem768x25519plus.native.0rtt.")
            .unwrap();
        let pub_bytes = BASE64_URL_SAFE_NO_PAD.decode(pub_b64).unwrap();
        // RFC 7748 expected public key for scalar above:
        let expected_pub: [u8; 32] = [
            0x85, 0x20, 0xf0, 0x09, 0x89, 0x30, 0xa7, 0x54, 0x74, 0x8b, 0x7d, 0xdc, 0xb4, 0x3e,
            0xf7, 0x5a, 0x0d, 0xbf, 0x3a, 0x0d, 0x26, 0x38, 0x1a, 0xf4, 0xeb, 0xa4, 0xa9, 0x8e,
            0xaa, 0x9b, 0x4e, 0x6a,
        ];
        assert_eq!(pub_bytes.as_slice(), &expected_pub);

        let random_pair = generate_vless_encryption_pair();
        assert_ne!(random_pair.decryption, random_pair.encryption);
    }
}
