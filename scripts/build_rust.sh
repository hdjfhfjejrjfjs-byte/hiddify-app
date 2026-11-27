#!/usr/bin/env bash
set -euo pipefail

# Prevent macOS from writing AppleDouble (._*) resource-fork files on external volumes (ITMS-90255).
export COPYFILE_DISABLE=1

# Cross-platform build & packaging script for Hiddify Rust modules (`libhiddify_rust`).
# Targets:
#   host     - Build release shared lib for the current host OS/arch
#   ios      - Build iOS Dynamic Framework XCFramework (`ios/Frameworks/HiddifyRust.xcframework`)
#              Optional 2nd arg / IOS_ARCH: arm64 (device-only) | all (device + simulator, default)
#   macos    - Build macOS dylib (`macos/Frameworks/libhiddify_rust.dylib`)
#              Optional 2nd arg / MACOS_ARCH: arm64 | x86_64 | universal (default)
#   android  - Build Android JNI shared libs (`android/app/src/main/jniLibs/<abi>/libhiddify_rust.so`)
#   linux    - Build Linux shared lib (`rust/dist/linux/libhiddify_rust.so`)
#   test     - Run full `cargo test --workspace`

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="${ROOT_DIR}/rust"
HEADER_FILE="${RUST_DIR}/include/hiddify_rust.h"
export PATH="${HOME}/.cargo/bin:${PATH}"

clean_appledouble_and_perms() {
    local target_path="$1"
    if [[ -d "${target_path}" ]]; then
        find "${target_path}" -name '._*' -delete 2>/dev/null || true
    fi
    chmod -R a+rX "${target_path}" 2>/dev/null || true
}

ensure_target() {
    local target="$1"
    if command -v rustup >/dev/null 2>&1; then
        if ! rustup target list --installed | grep -qx "${target}"; then
            echo "==> Installing Rust target ${target}..."
            rustup target add "${target}"
        fi
    fi
}

create_ios_framework_bundle() {
    local dylib_src="$1"
    local platform_name="$2" # iPhoneOS | iPhoneSimulator
    local stage_root="$3"
    local fw_dir="${stage_root}/HiddifyRust.framework"

    rm -rf "${fw_dir}"
    mkdir -p "${fw_dir}/Headers" "${fw_dir}/Modules"

    cp "${dylib_src}" "${fw_dir}/HiddifyRust"
    chmod 0755 "${fw_dir}/HiddifyRust"
    install_name_tool -id "@rpath/HiddifyRust.framework/HiddifyRust" "${fw_dir}/HiddifyRust"

    cp "${HEADER_FILE}" "${fw_dir}/Headers/hiddify_rust.h"
    cat > "${fw_dir}/Modules/module.modulemap" <<'EOF'
framework module HiddifyRust {
    umbrella header "hiddify_rust.h"
    export *
    module * { export * }
}
EOF

    cat > "${fw_dir}/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>HiddifyRust</string>
    <key>CFBundleIdentifier</key>
    <string>app.hiddify.com.HiddifyRust</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>HiddifyRust</string>
    <key>CFBundlePackageType</key>
    <string>FMWK</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundleSupportedPlatforms</key>
    <array>
        <string>${platform_name}</string>
    </array>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>MinimumOSVersion</key>
    <string>15.0</string>
</dict>
</plist>
EOF
    plutil -convert binary1 "${fw_dir}/Info.plist" 2>/dev/null || true
    clean_appledouble_and_perms "${fw_dir}"
}

build_host() {
    echo "==> Building libhiddify_rust (release) for host..."
    cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi
    local uname_s
    uname_s="$(uname -s)"
    if [[ "${uname_s}" == "Darwin" ]]; then
        mkdir -p "${ROOT_DIR}/macos/Frameworks"
        local out_dylib="${ROOT_DIR}/macos/Frameworks/libhiddify_rust.dylib"
        cp "${RUST_DIR}/target/release/libhiddify_rust.dylib" "${out_dylib}"
        chmod 0755 "${out_dylib}"
        install_name_tool -id "@rpath/libhiddify_rust.dylib" "${out_dylib}" || true
        clean_appledouble_and_perms "${out_dylib}"
        echo "==> Installed ${out_dylib}"
    elif [[ "${uname_s}" == "Linux" ]]; then
        mkdir -p "${RUST_DIR}/dist/linux"
        cp "${RUST_DIR}/target/release/libhiddify_rust.so" "${RUST_DIR}/dist/linux/libhiddify_rust.so"
        chmod 0755 "${RUST_DIR}/dist/linux/libhiddify_rust.so"
        echo "==> Installed rust/dist/linux/libhiddify_rust.so"
    fi
}

build_ios() {
    local ios_arch="${1:-${IOS_ARCH:-all}}"
    case "${ios_arch}" in
        arm64|device)
            echo "==> Building HiddifyRust.xcframework (arm64 device-only) for iOS..."
            ensure_target aarch64-apple-ios

            IPHONEOS_DEPLOYMENT_TARGET=15.0 \
                cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target aarch64-apple-ios

            local stage_dev="${RUST_DIR}/target/xcframework-stage/iphoneos"
            create_ios_framework_bundle \
                "${RUST_DIR}/target/aarch64-apple-ios/release/libhiddify_rust.dylib" \
                "iPhoneOS" \
                "${stage_dev}"

            local out_xcf="${ROOT_DIR}/ios/Frameworks/HiddifyRust.xcframework"
            rm -rf "${out_xcf}"
            mkdir -p "${ROOT_DIR}/ios/Frameworks"
            xcodebuild -create-xcframework \
                -framework "${stage_dev}/HiddifyRust.framework" \
                -output "${out_xcf}"
            clean_appledouble_and_perms "${out_xcf}"
            echo "==> Created device-only ${out_xcf} (arm64)"
            ;;
        all|universal)
            echo "==> Building HiddifyRust.xcframework (device + simulator) for iOS..."
            ensure_target aarch64-apple-ios
            ensure_target aarch64-apple-ios-sim

            IPHONEOS_DEPLOYMENT_TARGET=15.0 \
                cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target aarch64-apple-ios
            IPHONEOS_DEPLOYMENT_TARGET=15.0 \
                cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target aarch64-apple-ios-sim

            local sim_dylib="${RUST_DIR}/target/aarch64-apple-ios-sim/release/libhiddify_rust.dylib"
            if rustup target list --installed 2>/dev/null | grep -qx "x86_64-apple-ios"; then
                IPHONEOS_DEPLOYMENT_TARGET=15.0 \
                    cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target x86_64-apple-ios
                mkdir -p "${RUST_DIR}/target/ios-sim-universal"
                sim_dylib="${RUST_DIR}/target/ios-sim-universal/libhiddify_rust.dylib"
                lipo -create \
                    "${RUST_DIR}/target/aarch64-apple-ios-sim/release/libhiddify_rust.dylib" \
                    "${RUST_DIR}/target/x86_64-apple-ios/release/libhiddify_rust.dylib" \
                    -output "${sim_dylib}"
            fi

            local stage_dev="${RUST_DIR}/target/xcframework-stage/iphoneos"
            local stage_sim="${RUST_DIR}/target/xcframework-stage/iphonesimulator"
            create_ios_framework_bundle \
                "${RUST_DIR}/target/aarch64-apple-ios/release/libhiddify_rust.dylib" \
                "iPhoneOS" \
                "${stage_dev}"
            create_ios_framework_bundle \
                "${sim_dylib}" \
                "iPhoneSimulator" \
                "${stage_sim}"

            local out_xcf="${ROOT_DIR}/ios/Frameworks/HiddifyRust.xcframework"
            rm -rf "${out_xcf}"
            mkdir -p "${ROOT_DIR}/ios/Frameworks"
            xcodebuild -create-xcframework \
                -framework "${stage_dev}/HiddifyRust.framework" \
                -framework "${stage_sim}/HiddifyRust.framework" \
                -output "${out_xcf}"
            clean_appledouble_and_perms "${out_xcf}"
            echo "==> Created ${out_xcf} (device + simulator)"
            ;;
        *)
            echo "ERROR: Unsupported iOS arch '${ios_arch}' (expected: arm64 | all)" >&2
            exit 1
            ;;
    esac
}

build_macos() {
    local macos_arch="${1:-${MACOS_ARCH:-universal}}"
    mkdir -p "${ROOT_DIR}/macos/Frameworks"
    local out_dylib="${ROOT_DIR}/macos/Frameworks/libhiddify_rust.dylib"

    case "${macos_arch}" in
        arm64)
            echo "==> Building arm64 libhiddify_rust.dylib for macOS..."
            ensure_target aarch64-apple-darwin
            MACOSX_DEPLOYMENT_TARGET=12.0 \
                cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target aarch64-apple-darwin
            cp -f "${RUST_DIR}/target/aarch64-apple-darwin/release/libhiddify_rust.dylib" "${out_dylib}"
            ;;
        x86_64)
            echo "==> Building x86_64 libhiddify_rust.dylib for macOS..."
            ensure_target x86_64-apple-darwin
            MACOSX_DEPLOYMENT_TARGET=12.0 \
                cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target x86_64-apple-darwin
            cp -f "${RUST_DIR}/target/x86_64-apple-darwin/release/libhiddify_rust.dylib" "${out_dylib}"
            ;;
        universal|all)
            echo "==> Building Universal2 libhiddify_rust.dylib for macOS..."
            ensure_target aarch64-apple-darwin
            ensure_target x86_64-apple-darwin

            MACOSX_DEPLOYMENT_TARGET=12.0 \
                cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target aarch64-apple-darwin
            MACOSX_DEPLOYMENT_TARGET=12.0 \
                cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target x86_64-apple-darwin

            lipo -create \
                "${RUST_DIR}/target/aarch64-apple-darwin/release/libhiddify_rust.dylib" \
                "${RUST_DIR}/target/x86_64-apple-darwin/release/libhiddify_rust.dylib" \
                -output "${out_dylib}"
            ;;
        *)
            echo "ERROR: Unsupported macOS arch '${macos_arch}' (expected: arm64 | x86_64 | universal)" >&2
            exit 1
            ;;
    esac

    chmod 0755 "${out_dylib}"
    install_name_tool -id "@rpath/libhiddify_rust.dylib" "${out_dylib}"
    clean_appledouble_and_perms "${out_dylib}"
    echo "==> Created ${out_dylib} ($(lipo -archs "${out_dylib}"))"
}

build_android() {
    echo "==> Building libhiddify_rust.so for Android ABIs..."
    local jni_root="${ROOT_DIR}/android/app/src/main/jniLibs"
    ensure_target aarch64-linux-android
    ensure_target armv7-linux-androideabi
    ensure_target x86_64-linux-android

    if command -v cargo-ndk >/dev/null 2>&1; then
        (cd "${RUST_DIR}" && cargo ndk \
            -t arm64-v8a \
            -t armeabi-v7a \
            -t x86_64 \
            -o "${jni_root}" \
            build --release -p hiddify-ffi)
    else
        local ndk_root="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
        if [[ -z "${ndk_root}" || ! -d "${ndk_root}" ]]; then
            echo "ERROR: Set ANDROID_NDK_HOME or install cargo-ndk ('cargo install cargo-ndk') to cross-compile Android targets." >&2
            exit 1
        fi
        local host_tag
        host_tag="$(ls "${ndk_root}/toolchains/llvm/prebuilt" | head -n 1)"
        local tc_bin="${ndk_root}/toolchains/llvm/prebuilt/${host_tag}/bin"
        local api=21

        CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${tc_bin}/aarch64-linux-android${api}-clang" \
            cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target aarch64-linux-android
        CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="${tc_bin}/armv7a-linux-androideabi${api}-clang" \
            cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target armv7-linux-androideabi
        CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="${tc_bin}/x86_64-linux-android${api}-clang" \
            cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target x86_64-linux-android

        mkdir -p "${jni_root}/arm64-v8a" "${jni_root}/armeabi-v7a" "${jni_root}/x86_64"
        cp "${RUST_DIR}/target/aarch64-linux-android/release/libhiddify_rust.so" "${jni_root}/arm64-v8a/"
        cp "${RUST_DIR}/target/armv7-linux-androideabi/release/libhiddify_rust.so" "${jni_root}/armeabi-v7a/"
        cp "${RUST_DIR}/target/x86_64-linux-android/release/libhiddify_rust.so" "${jni_root}/x86_64/"
    fi
    echo "==> Installed Android JNI libraries in ${jni_root}"
}

build_linux() {
    echo "==> Building libhiddify_rust.so for Linux..."
    local target="${LINUX_RUST_TARGET:-}"
    if [[ -z "${target}" ]]; then
        case "$(uname -m)" in
            aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
            *) target="x86_64-unknown-linux-gnu" ;;
        esac
    fi
    ensure_target "${target}"
    cargo build --manifest-path "${RUST_DIR}/Cargo.toml" --release -p hiddify-ffi --target "${target}"
    mkdir -p "${RUST_DIR}/dist/linux"
    cp "${RUST_DIR}/target/${target}/release/libhiddify_rust.so" "${RUST_DIR}/dist/linux/libhiddify_rust.so"
    chmod 0755 "${RUST_DIR}/dist/linux/libhiddify_rust.so"
    echo "==> Created ${RUST_DIR}/dist/linux/libhiddify_rust.so"
}

run_tests() {
    echo "==> Running Rust workspace tests..."
    cargo test --manifest-path "${RUST_DIR}/Cargo.toml" --workspace
}

ACTION="${1:-host}"
TARGET_ARCH="${2:-}"
case "${ACTION}" in
    host)    build_host ;;
    ios)     build_ios "${TARGET_ARCH}" ;;
    macos)   build_macos "${TARGET_ARCH}" ;;
    android) build_android ;;
    linux)   build_linux ;;
    test)    run_tests ;;
    *)
        echo "Usage: $0 {host|ios [arm64|all]|macos [arm64|x86_64|universal]|android|linux|test}" >&2
        exit 1
        ;;
esac
