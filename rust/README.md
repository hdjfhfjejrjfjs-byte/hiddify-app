# Hiddify Rust Workspace (`libhiddify_rust`)

Modular, pure-Rust domain libraries extracted from `hiddify-app` with a unified panic-safe C-ABI facade for **iOS, Android, macOS, and Linux**.

## Workspace Crates

| Crate | Type | Responsibility |
| :--- | :--- | :--- |
| `hiddify-crypto` | Standalone lib | RSA-2048 PKCS#8 + self-signed X.509 v3 mTLS identity generation & renewal checks, PEM/DER pinning verification, unambiguous CSPRNG proxy secrets, streaming SHA-256 file verification. |
| `hiddify-net` | Standalone lib | URL, port, port-range, IPv4/IPv6 CIDR, domain, `domain_suffix`, OS process name/path validators, IP privacy masking, and proxy tag / WARP scanner endpoint parsers. |
| `hiddify-config` | Standalone lib | Sing-box JSON proxy counting & extraction (`keep-top`), flag/country/protocol mapping, editable config filtering, `singleProxyConfig` transitive `detour` extraction, and `profileOverride` deep JSON merging (with `preserve_order`). |
| `hiddify-profile` | Standalone lib | Subscription comment/HTTP header merging, `subscription-userinfo` tolerant parser, RFC 5987 `Content-Disposition` decoding, 7-tier profile name resolution, proxy protocol detection, deep-link parsing, and ZIP config archive merging. |
| `hiddify-diagnostics` | Standalone lib | Telegram `<= 64`-byte base64url diagnostic payload encoding, Linux `/etc/os-release` & Windows build detection, Go/native crash excerpt extraction, GitHub changelog extraction, and bounded `LogRing`. |
| `hiddify-ffi` | `cdylib` + `staticlib` | Panic-safe (`catch_unwind`) C-ABI facade matching `include/hiddify_rust.h`. |

## Building & Testing

```bash
# Run all crate unit tests
./scripts/build_rust.sh test

# Build for current desktop host
./scripts/build_rust.sh host

# Cross-build per target platform
./scripts/build_rust.sh ios       # -> ios/Frameworks/HiddifyRust.xcframework
./scripts/build_rust.sh macos     # -> macos/Frameworks/libhiddify_rust.dylib
./scripts/build_rust.sh android   # -> android/app/src/main/jniLibs/<abi>/libhiddify_rust.so
./scripts/build_rust.sh linux     # -> rust/dist/linux/libhiddify_rust.so
```
