#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Hiddify macOS (.app) & iOS (.ipa) 一键构建脚本
# 功能：
#   1. 自动检测构建环境（macOS、Xcode、Homebrew、Flutter SDK、CocoaPods、基础工具）
#   2. 自动修复环境问题（版本不匹配自动安装指定 Flutter、缺失/损坏 CocoaPods 自动修复、
#      UTF-8 locale 修复、核心库自动下载、依赖与代码生成自动补齐、pod 自动重试等）
#   3. 编译 macOS .app（可选生成 .dmg）和 iOS .ipa（自动识别签名环境或回退无签名打包）
#   4. 幂等构建：环境与依赖已就绪时自动跳过重复安装（pub get / 代码生成 / pod install /
#      precache / 核心库下载），仅在缺失或版本变化时执行（--force-deps / --force-libs 可强制）
# ==============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$SCRIPT_DIR"

# --- 默认配置 ---
PLATFORM="all"            # all | macos | ios
CHANNEL="prod"            # prod | dev
SIGN_MODE="auto"          # auto | unsigned | signed | developer-id | app-store
EXPORT_OPTIONS=""         # 自定义 iOS exportOptions.plist 路径
OUTPUT_DIR="$ROOT_DIR/build_output"
IOS_IPA_OVERRIDE=""       # 自定义 iOS .ipa 输出/安装路径
INSTALL_MODE="auto"       # auto | always | never
INSTALL_ONLY=false
TROLLSTORE_MODE=false
DEVICE_ID="${UDID:-${DEVICE_UDID:-}}"
CUSTOM_BUNDLE_ID="${BUNDLE_ID:-}"
CUSTOM_APP_NAME="${APP_DISPLAY_NAME:-}"
PROVISION_PATH="${PLUMESIGN_PROVISION:-}"
PLUMESIGN_BIN="${PLUMESIGN:-}"
BUILD_DMG=false
SKIP_CODEGEN=false
FORCE_LIBS=false
FORCE_DEPS=false
DO_CLEAN=false
CHECK_ONLY=false
SENTRY_DSN="${SENTRY_DSN:-}"
FLUTTER_BIN_DIR=""
FLUTTER_VER=""

START_TS="$(date +%s)"

# --- 颜色与日志 ---
if [[ -t 1 ]]; then
  C_RESET='\033[0m'
  C_BLUE='\033[1;34m'
  C_GREEN='\033[1;32m'
  C_YELLOW='\033[1;33m'
  C_RED='\033[1;31m'
  C_CYAN='\033[1;36m'
else
  C_RESET='' C_BLUE='' C_GREEN='' C_YELLOW='' C_RED='' C_CYAN=''
fi

log_step() { printf "\n${C_CYAN}==> %s${C_RESET}\n" "$*"; }
log_info() { printf "${C_BLUE}[INFO]${C_RESET} %s\n" "$*"; }
log_ok()   { printf "${C_GREEN}[OK]${C_RESET}   %s\n" "$*"; }
log_fix()  { printf "${C_YELLOW}[FIX]${C_RESET}  %s\n" "$*"; }
log_skip() { printf "${C_CYAN}[SKIP]${C_RESET} %s\n" "$*"; }
log_warn() { printf "${C_YELLOW}[WARN]${C_RESET} %s\n" "$*"; }
log_err()  { printf "${C_RED}[ERR]${C_RESET}  %s\n" "$*" >&2; }

die() {
  log_err "$@"
  exit 1
}

usage() {
  cat <<'EOF'
用法: ./build.sh [选项]

选项:
  --platform <all|macos|ios>     构建目标平台（默认: all，同时编译 macOS .app 和 iOS .ipa）
  --channel <prod|dev>           构建渠道（默认: prod，对应 lib/main_prod.dart 与版本 core 库）
  --sign <mode>                  签名模式（默认: auto）
                                   auto         : 自动检测本机证书与配置，无证书则自动无签名/ad-hoc构建
                                   unsigned     : 强制无签名/ad-hoc构建（macOS .app + iOS 无签名 .ipa）
                                   signed       : 强制签名构建（macOS 用 developer-id，iOS 用 exportOptions.plist）
                                   developer-id : macOS 使用 Developer ID 签名（iOS 跟随 auto/signed）
                                   app-store    : macOS 使用 App Store 签名，iOS 使用 App Store 导出
  --export-options <plist>       指定 iOS 签名导出配置文件（默认: ios/exportOptions.plist）
  --output <dir|ipa>             产物输出目录（默认: ./build_output；若以 .ipa 结尾则作为 iOS IPA 路径）
  --ipa <path>                   指定 iOS .ipa 输出路径（或配合 --install-only 指定待安装的 IPA）
  --install                      iOS 打包完成后签名并安装到已连接 USB 的真机
  --trollstore                   TrollStore / TrollStore Lite：强制无签名编译，并用 ldid 伪签 VPN 与 App Group 权限（Runner 与 HiddifyPacketTunnel.appex 缺一即失败），再通过 USB 推送到真机 Downloads/Hiddify.ipa；若免密 SSH 可用则自动调用 trollstorehelper 安装
  --no-install                   仅编译打包，跳过 iOS 真机签名与安装
  --install-only                 跳过编译，直接将已有 iOS .ipa 签名并安装到 USB 真机
  --device, --udid <id>          目标 iOS 真机 UDID 或 usbmuxd DeviceID（默认首个 USB 设备；支持环境变量 UDID / DEVICE_UDID）
  --bundle-id <id>               重签安装时覆盖 Bundle Identifier（plumesign --custom-identifier；支持 BUNDLE_ID）
  --name <name>                  重签安装时覆盖应用显示名（plumesign --custom-name；支持 APP_DISPLAY_NAME）
  --provision <path>             重签安装时嵌入指定描述文件（plumesign --provision；支持 PLUMESIGN_PROVISION）
  --plumesign <path>             指定 plumesign 可执行文件路径（支持环境变量 PLUMESIGN）
  --dmg                          macOS 编译完成后额外生成 .dmg 安装包
  --skip-codegen                 强制跳过 build_runner / slang 代码生成（生成文件缺失时仍会自动执行）
  --force-libs                   强制重新下载 hiddify-core 原生库
  --force-deps                   强制重新执行 pub get、代码生成与 pod install（忽略缓存检测）
  --clean                        构建前执行 flutter clean
  --check-only                   仅检测并自动修复构建环境，不执行编译
  -h, --help                     显示本帮助信息

示例:
  ./build.sh                                         # 自动修复环境并编译 macOS .app + iOS .ipa（若连有 USB 真机则自动签名安装）
  ./build.sh --platform macos --dmg                  # 仅编译 macOS .app 并生成 .dmg
  ./build.sh --platform ios --sign unsigned          # 编译嵌入权限声明的 iOS .ipa，并自动用 plumesign 签名安装到 USB 真机
  ./build.sh --platform ios --trollstore             # 强制 ldid 伪签 VPN/App Group，推送到 TrollStore 真机
  ./build.sh --platform ios --no-install             # 仅编译打包 iOS .ipa，不安装到真机
  ./build.sh --install-only                          # 跳过编译，直接将已有 .ipa 签名安装到 USB 真机
  ./build.sh --check-only                            # 仅检查并修复环境
EOF
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --platform)
        PLATFORM="${2:-}"
        shift 2
        ;;
      --channel)
        CHANNEL="${2:-}"
        shift 2
        ;;
      --sign)
        SIGN_MODE="${2:-}"
        shift 2
        ;;
      --export-options)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        EXPORT_OPTIONS="${2:-}"
        shift 2
        ;;
      --output)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        if [[ "${2:-}" == *.ipa ]]; then
          IOS_IPA_OVERRIDE="${2}"
          OUTPUT_DIR="$(dirname "${2}")"
        else
          OUTPUT_DIR="${2:-}"
        fi
        shift 2
        ;;
      --ipa)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        IOS_IPA_OVERRIDE="${2:-}"
        shift 2
        ;;
      --install)
        INSTALL_MODE="always"
        shift
        ;;
      --trollstore)
        TROLLSTORE_MODE=true
        if [[ "$INSTALL_MODE" == "auto" ]]; then
          INSTALL_MODE="always"
        fi
        shift
        ;;
      --no-install)
        INSTALL_MODE="never"
        shift
        ;;
      --install-only)
        INSTALL_ONLY=true
        INSTALL_MODE="always"
        shift
        ;;
      --device|--udid)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        DEVICE_ID="${2:-}"
        shift 2
        ;;
      --bundle-id)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        CUSTOM_BUNDLE_ID="${2:-}"
        shift 2
        ;;
      --name)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        CUSTOM_APP_NAME="${2:-}"
        shift 2
        ;;
      --provision)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        PROVISION_PATH="${2:-}"
        shift 2
        ;;
      --plumesign)
        [[ $# -ge 2 ]] || die "$1 需要参数"
        PLUMESIGN_BIN="${2:-}"
        shift 2
        ;;
      --dmg)
        BUILD_DMG=true
        shift
        ;;
      --skip-codegen)
        SKIP_CODEGEN=true
        shift
        ;;
      --force-libs)
        FORCE_LIBS=true
        shift
        ;;
      --force-deps)
        FORCE_DEPS=true
        shift
        ;;
      --clean)
        DO_CLEAN=true
        shift
        ;;
      --check-only)
        CHECK_ONLY=true
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        die "未知参数: $1（使用 --help 查看用法）"
        ;;
    esac
  done

  case "$PLATFORM" in
    all|macos|ios) ;;
    *) die "无效的 --platform 参数: ${PLATFORM}（可选: all, macos, ios）" ;;
  esac

  case "$CHANNEL" in
    prod|dev) ;;
    *) die "无效的 --channel 参数: ${CHANNEL}（可选: prod, dev）" ;;
  esac

  case "$SIGN_MODE" in
    auto|unsigned|signed|developer-id|app-store) ;;
    *) die "无效的 --sign 参数: ${SIGN_MODE}（可选: auto, unsigned, signed, developer-id, app-store）" ;;
  esac

  # TrollStore 只认 Mach-O 里 ldid 伪签的 entitlements。放在解析末尾，这样无论
  # --sign 写在 --trollstore 之前还是之后，都不会改走 app-store 导出。
  if [[ "$TROLLSTORE_MODE" == true ]]; then
    if [[ "$SIGN_MODE" != "unsigned" && "$SIGN_MODE" != "auto" ]]; then
      log_warn "--trollstore 只认 ldid 伪签的 entitlements，已忽略 --sign ${SIGN_MODE}，改走 unsigned"
    fi
    SIGN_MODE="unsigned"
  fi
}

# --- 工具函数 ---
version_ge() {
  # 比较版本号 $1 >= $2（纯 POSIX/awk，不依赖 sort -V）
  awk -v v1="$1" -v v2="$2" '
    BEGIN {
      n1 = split(v1, a, ".")
      n2 = split(v2, b, ".")
      n = (n1 > n2 ? n1 : n2)
      for (i = 1; i <= n; i++) {
        x = (i <= n1 ? a[i] + 0 : 0)
        y = (i <= n2 ? b[i] + 0 : 0)
        if (x > y) exit 0
        if (x < y) exit 1
      }
      exit 0
    }
  '
}

version_major_minor_eq() {
  local m1 m2
  m1="$(printf '%s' "$1" | awk -F. '{print $1 "." $2}')"
  m2="$(printf '%s' "$2" | awk -F. '{print $1 "." $2}')"
  [[ "$m1" == "$m2" ]]
}

get_flutter_version_of_bin() {
  local bin="$1"
  [[ -x "$bin" ]] || return 1
  local sdk_root v=""
  sdk_root="$(cd "$(dirname "$bin")/.." && pwd)"

  # 1) 官方 SDK 内置 version 文件（极快且不触发联网）
  if [[ -f "$sdk_root/version" ]]; then
    v="$(tr -d '[:space:]' < "$sdk_root/version")"
    if [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
      printf '%s\n' "${BASH_REMATCH[0]}"
      return 0
    fi
  fi

  # 2) FVM / 新版 SDK 的 flutter.version.json（无需启动 flutter 工具）
  if [[ -f "$sdk_root/bin/cache/flutter.version.json" ]]; then
    v="$(sed -n 's/.*"frameworkVersion": *"\([0-9][0-9.]*\)".*/\1/p' "$sdk_root/bin/cache/flutter.version.json" | head -n 1)"
    if [[ -n "$v" ]]; then
      printf '%s\n' "$v"
      return 0
    fi
  fi

  # 3) 兜底：直接询问 flutter 工具（禁止版本检查，避免联网）
  "$bin" --no-version-check --version --suppress-analytics 2>/dev/null | awk 'NR==1 {print $2}' | sed -E 's/[^0-9.].*$//'
}

prepend_path() {
  local dir="$1"
  [[ -d "$dir" ]] || return 0
  case ":$PATH:" in
    *":$dir:"*) ;;
    *) export PATH="$dir:$PATH" ;;
  esac
}

append_path() {
  local dir="$1"
  [[ -d "$dir" ]] || return 0
  case ":$PATH:" in
    *":$dir:"*) ;;
    *) export PATH="$PATH:$dir" ;;
  esac
}

# --- 幂等缓存辅助：内容哈希 + 戳记，环境未变化时跳过重复安装 ---
hash_files() {
  # 对给定文件的内容做稳定哈希（不存在的文件直接忽略）
  local f
  for f in "$@"; do
    if [[ -f "$f" ]]; then
      shasum -a 256 "$f"
    fi
  done | shasum -a 256 | awk '{print $1}'
}

stamp_write() {
  local stamp="$1" value="$2"
  mkdir -p "$(dirname "$stamp")"
  printf '%s' "$value" > "$stamp"
}

stamp_matches() {
  local stamp="$1" value="$2"
  [[ -f "$stamp" ]] || return 1
  [[ "$(cat "$stamp")" == "$value" ]]
}

# ==============================================================================
# 阶段 1：基础系统与工具链检测 / 自动修复
# ==============================================================================
init_base_env() {
  log_step "1/6 检测系统与基础环境"

  if [[ "$(uname -s)" != "Darwin" ]]; then
    die "编译 macOS 与 iOS 客户端必须在 macOS 系统上运行（当前系统: $(uname -s)）"
  fi

  ARCH="$(uname -m)"
  MACOS_VER="$(sw_vers -productVersion)"
  log_info "系统: macOS $MACOS_VER ($ARCH)"

  # 强制 UTF-8，避免 CocoaPods / Ruby 在非 UTF-8 环境下崩溃
  if [[ "${LANG:-}" != *UTF-8* ]] || [[ "${LC_ALL:-}" != *UTF-8* ]]; then
    log_fix "修复 Locale 为 en_US.UTF-8（防止 CocoaPods 编码异常）"
    export LANG="en_US.UTF-8"
    export LC_ALL="en_US.UTF-8"
  fi

  # 预加载常见路径
  prepend_path "/opt/homebrew/bin"
  prepend_path "/opt/homebrew/sbin"
  prepend_path "/usr/local/bin"
  append_path "$HOME/.pub-cache/bin"
  append_path "$HOME/develop/flutter/bin"
  append_path "$HOME/flutter/bin"

  # 如果 Homebrew 已安装但未进入环境，自动加载 shellenv
  if [[ -x "/opt/homebrew/bin/brew" ]]; then
    eval "$(/opt/homebrew/bin/brew shellenv)"
  elif [[ -x "/usr/local/bin/brew" ]]; then
    eval "$(/usr/local/bin/brew shellenv)"
  fi

  # 解析项目元信息
  cd "$ROOT_DIR"
  APP_VERSION="$(grep '^version:' pubspec.yaml | head -n 1 | sed -E 's/^version:[[:space:]]*//' | tr -d "\"'")"
  REQUIRED_FLUTTER="$(sed -n '/^environment:/,/^[a-zA-Z0-9_]*:/p' pubspec.yaml | awk -F: '/flutter:/ {gsub(/[^0-9.]/, "", $2); print $2; exit}')"
  CORE_VERSION="$(awk -F= '/^core\.version=/ {gsub(/[[:space:]]/, "", $2); print $2; exit}' dependencies.properties)"
  APP_COMMIT="$(git rev-parse --short=7 HEAD 2>/dev/null || echo 'unknown')"

  [[ -n "$REQUIRED_FLUTTER" ]] || die "无法从 pubspec.yaml 解析所需 Flutter 版本"
  [[ -n "$CORE_VERSION" ]] || die "无法从 dependencies.properties 解析 core.version"

  if [[ "$CHANNEL" == "prod" ]]; then
    BUILD_TARGET="lib/main_prod.dart"
  else
    BUILD_TARGET="lib/main.dart"
  fi

  log_info "应用版本: $APP_VERSION | Commit: $APP_COMMIT | 渠道: $CHANNEL ($BUILD_TARGET)"
  log_info "目标平台: $PLATFORM | 签名模式: $SIGN_MODE | 需求 Flutter: >= $REQUIRED_FLUTTER | Core: v$CORE_VERSION"
}

ensure_homebrew() {
  if command -v brew >/dev/null 2>&1; then
    log_ok "Homebrew: $(brew --version | head -n 1)"
    return 0
  fi

  log_fix "未检测到 Homebrew，正在自动安装..."
  NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  if [[ -x "/opt/homebrew/bin/brew" ]]; then
    eval "$(/opt/homebrew/bin/brew shellenv)"
  elif [[ -x "/usr/local/bin/brew" ]]; then
    eval "$(/usr/local/bin/brew shellenv)"
  fi
  command -v brew >/dev/null 2>&1 || die "Homebrew 安装失败，请手动安装后重试"
  log_ok "Homebrew 安装完成"
}

ensure_basic_tools() {
  local missing=()
  local tool
  for tool in git curl tar unzip zip rsync make; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      missing+=("$tool")
    fi
  done
  if [[ ${#missing[@]} -gt 0 ]]; then
    ensure_homebrew
    log_fix "自动安装缺失基础工具: ${missing[*]}"
    brew install "${missing[@]}"
  fi
  log_ok "基础 CLI 工具完整 (git/curl/tar/unzip/zip/rsync/make)"
}

ensure_xcode() {
  log_step "2/6 检测 Xcode 与苹果 SDK 环境"

  local dev_dir=""
  dev_dir="$(xcode-select -p 2>/dev/null || true)"

  if [[ -z "$dev_dir" || "$dev_dir" == *CommandLineTools* ]]; then
    if [[ -d "/Applications/Xcode.app/Contents/Developer" ]]; then
      log_fix "当前 xcode-select 指向 '${dev_dir:-空}'，正在尝试切换至完整 Xcode..."
      if sudo -n xcode-select --switch /Applications/Xcode.app/Contents/Developer 2>/dev/null; then
        log_ok "已切换至 /Applications/Xcode.app/Contents/Developer"
      elif [[ -t 0 ]]; then
        sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
      else
        die "请先执行: sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer"
      fi
    else
      die "未找到完整的 /Applications/Xcode.app，请先从 App Store 或 Apple Developer 安装 Xcode"
    fi
  fi

  local xcode_ver_out=""
  if ! xcode_ver_out="$(xcodebuild -version 2>&1)"; then
    if [[ "$xcode_ver_out" == *license* ]]; then
      log_fix "Xcode 许可协议未同意，正在尝试自动接受..."
      if sudo -n xcodebuild -license accept 2>/dev/null; then
        xcode_ver_out="$(xcodebuild -version 2>&1)"
      elif [[ -t 0 ]]; then
        sudo xcodebuild -license accept
        xcode_ver_out="$(xcodebuild -version 2>&1)"
      else
        die "请先执行: sudo xcodebuild -license accept"
      fi
    else
      die "xcodebuild 不可用: $xcode_ver_out"
    fi
  fi
  log_ok "$(printf '%s' "$xcode_ver_out" | tr '\n' ' ')"

  # 检查 Xcode 首次启动组件（某些新版 Xcode 会返回 69，即使 SDK 已正常，非致命处理）
  if ! xcodebuild -checkFirstLaunchStatus >/dev/null 2>&1; then
    log_warn "Xcode checkFirstLaunchStatus 返回非 0，尝试执行 xcodebuild -runFirstLaunch..."
    if xcodebuild -runFirstLaunch >/dev/null 2>&1; then
      log_ok "Xcode 首次启动组件初始化完成"
    else
      log_warn "xcodebuild -runFirstLaunch 未能无密码完成（若后续编译报组件缺失，请手动执行 sudo xcodebuild -runFirstLaunch）"
    fi
  fi

  # 检查目标平台 SDK
  local sdks
  sdks="$(xcodebuild -showsdks 2>/dev/null || true)"
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
    if ! printf '%s\n' "$sdks" | grep -q 'macosx'; then
      die "未检测到 macOS SDK，请检查 Xcode 安装"
    fi
    log_ok "已就绪 SDK: $(printf '%s\n' "$sdks" | grep -m1 'macosx' | xargs)"
  fi

  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]]; then
    if ! printf '%s\n' "$sdks" | grep -q 'iphoneos'; then
      log_fix "未检测到 iOS 平台 SDK，正在尝试通过 xcodebuild 自动下载 iOS 平台..."
      xcodebuild -downloadPlatform iOS || die "自动下载 iOS SDK 失败，请在 Xcode -> Settings -> Components 中手动安装 iOS 平台"
      sdks="$(xcodebuild -showsdks 2>/dev/null || true)"
      printf '%s\n' "$sdks" | grep -q 'iphoneos' || die "iOS SDK 仍不可用"
    fi
    log_ok "已就绪 SDK: $(printf '%s\n' "$sdks" | grep -m1 'iphoneos' | xargs)"
  fi
}

# ==============================================================================
# 阶段 2：Flutter SDK 检测与自动修复
# ==============================================================================
is_compatible_flutter() {
  local ver="$1"
  [[ -n "$ver" ]] || return 1
  # 满足 pubspec.yaml ^3.38.5：>= 3.38.5 且主版本为 3
  local req_major cur_major
  req_major="${REQUIRED_FLUTTER%%.*}"
  cur_major="${ver%%.*}"
  [[ "$cur_major" == "$req_major" ]] && version_ge "$ver" "$REQUIRED_FLUTTER"
}

find_existing_compatible_flutter() {
  local candidates=()
  local p

  # 1. 优先精确匹配的 FVM / develop 目录
  candidates+=(
    "$HOME/fvm/versions/$REQUIRED_FLUTTER/bin/flutter"
    "$HOME/develop/flutter-$REQUIRED_FLUTTER/bin/flutter"
    "$HOME/develop/flutter/bin/flutter"
    "$HOME/flutter/bin/flutter"
    "$ROOT_DIR/.fvm/flutter_sdk/bin/flutter"
  )

  # 2. 扫描 ~/fvm/versions 下所有版本
  if [[ -d "$HOME/fvm/versions" ]]; then
    for p in "$HOME/fvm/versions"/*/bin/flutter; do
      [[ -x "$p" ]] && candidates+=("$p")
    done
  fi

  # 3. 当前 PATH 中的 flutter
  if command -v flutter >/dev/null 2>&1; then
    candidates+=("$(command -v flutter)")
  fi

  local ver best_bin="" best_ver=""
  for p in "${candidates[@]}"; do
    [[ -x "$p" ]] || continue
    ver="$(get_flutter_version_of_bin "$p" || true)"
    [[ -n "$ver" ]] || continue
    if [[ "$ver" == "$REQUIRED_FLUTTER" ]]; then
      printf '%s\n' "$(dirname "$p")"
      return 0
    fi
    if is_compatible_flutter "$ver"; then
      if [[ -z "$best_ver" ]] || version_ge "$ver" "$best_ver"; then
        best_ver="$ver"
        best_bin="$(dirname "$p")"
      fi
    fi
  done

  if [[ -n "$best_bin" ]]; then
    printf '%s\n' "$best_bin"
    return 0
  fi
  return 1
}

install_flutter_sdk() {
  FLUTTER_BIN_DIR=""
  # 优先使用已安装的 fvm（不污染全局其他版本）
  if command -v fvm >/dev/null 2>&1; then
    log_fix "检测到 FVM，正在自动安装 Flutter $REQUIRED_FLUTTER..."
    if fvm install "$REQUIRED_FLUTTER" --no-setup </dev/null; then
      if [[ -x "$HOME/fvm/versions/$REQUIRED_FLUTTER/bin/flutter" ]]; then
        FLUTTER_BIN_DIR="$HOME/fvm/versions/$REQUIRED_FLUTTER/bin"
        return 0
      fi
    fi
    log_warn "FVM 安装 Flutter $REQUIRED_FLUTTER 失败，回退至官方归档下载..."
  fi

  local install_parent="$HOME/develop"
  local install_dir="$install_parent/flutter-$REQUIRED_FLUTTER"
  mkdir -p "$install_parent"

  local arch_suffix=""
  [[ "$ARCH" == "arm64" ]] && arch_suffix="_arm64"
  local zip_name="flutter_macos${arch_suffix}_${REQUIRED_FLUTTER}-stable.zip"
  local url="https://storage.googleapis.com/flutter_infra_release/releases/stable/macos/${zip_name}"
  local tmp_dir
  tmp_dir="$(mktemp -d)"

  log_fix "正在下载官方 Flutter SDK ($REQUIRED_FLUTTER, $ARCH)..."
  if curl -fL --retry 3 --retry-delay 3 -o "$tmp_dir/$zip_name" "$url"; then
    log_info "正在解压 Flutter SDK 到 $install_dir..."
    unzip -q "$tmp_dir/$zip_name" -d "$tmp_dir/extract"
    rm -rf "$install_dir"
    mv "$tmp_dir/extract/flutter" "$install_dir"
    rm -rf "$tmp_dir"
    if [[ -x "$install_dir/bin/flutter" ]]; then
      FLUTTER_BIN_DIR="$install_dir/bin"
      return 0
    fi
  fi

  rm -rf "$tmp_dir"
  log_warn "官方归档下载失败，尝试通过 git clone Flutter tag $REQUIRED_FLUTTER..."
  rm -rf "$install_dir"
  git clone --branch "$REQUIRED_FLUTTER" --depth 1 https://github.com/flutter/flutter.git "$install_dir"
  git config --global --add safe.directory "$install_dir" || true
  [[ -x "$install_dir/bin/flutter" ]] || die "Flutter SDK 自动安装失败"
  FLUTTER_BIN_DIR="$install_dir/bin"
}

ensure_flutter() {
  log_step "3/6 检测 Flutter SDK 与 CocoaPods"

  FLUTTER_BIN_DIR=""
  FLUTTER_VER=""
  if FLUTTER_BIN_DIR="$(find_existing_compatible_flutter)"; then
    prepend_path "$FLUTTER_BIN_DIR"
    FLUTTER_VER="$(get_flutter_version_of_bin "$FLUTTER_BIN_DIR/flutter" || echo unknown)"
    log_ok "找到兼容 Flutter SDK: $FLUTTER_VER ($FLUTTER_BIN_DIR)"
  else
    if command -v flutter >/dev/null 2>&1; then
      log_warn "当前 PATH 中的 Flutter 版本 ($(get_flutter_version_of_bin "$(command -v flutter)" || echo unknown)) 不满足 >= ${REQUIRED_FLUTTER}"
    else
      log_warn "未检测到兼容的 Flutter SDK（需求 >= ${REQUIRED_FLUTTER}）"
    fi
    install_flutter_sdk
    prepend_path "$FLUTTER_BIN_DIR"
    FLUTTER_VER="$(get_flutter_version_of_bin "$FLUTTER_BIN_DIR/flutter" || echo unknown)"
    log_ok "Flutter SDK 已安装就绪: $FLUTTER_BIN_DIR"
  fi

  # 确保 Flutter 工具链初始化并开启 macOS 桌面支持
  export FLUTTER_SUPPRESS_ANALYTICS=true
  flutter --no-version-check --version --suppress-analytics >/dev/null
  flutter config --enable-macos-desktop --no-analytics >/dev/null 2>&1 || true

  local precache_flags=()
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
    precache_flags+=(--macos)
  fi
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]]; then
    precache_flags+=(--ios)
  fi

  # Flutter 版本未变化且所需平台的引擎产物仍在缓存中时，跳过重复的 precache 检查
  local sdk_root cache_root precache_stamp precache_key artifacts_ready=true flag
  sdk_root="$(cd "$FLUTTER_BIN_DIR/.." && pwd)"
  cache_root="$sdk_root/bin/cache"
  precache_stamp="$ROOT_DIR/.dart_tool/.build_sh_precache_stamp"
  precache_key="flutter=$FLUTTER_VER"
  for flag in "${precache_flags[@]}"; do
    case "$flag" in
      --macos)
        [[ -d "$cache_root/artifacts/engine/darwin-x64" ]] || artifacts_ready=false
        [[ -d "$cache_root/artifacts/engine/darwin-x64-release" ]] || artifacts_ready=false
        ;;
      --ios)
        [[ -d "$cache_root/artifacts/engine/ios" ]] || artifacts_ready=false
        [[ -d "$cache_root/artifacts/engine/ios-release" ]] || artifacts_ready=false
        ;;
    esac
  done

  if [[ "$FORCE_DEPS" != true && "$artifacts_ready" == true ]] && stamp_matches "$precache_stamp" "$precache_key"; then
    log_skip "Flutter 引擎产物已就绪（$FLUTTER_VER: ${precache_flags[*]}），跳过 flutter precache"
  else
    log_info "检查并预缓存 Flutter 引擎产物 (${precache_flags[*]})..."
    flutter precache "${precache_flags[@]}"
    stamp_write "$precache_stamp" "$precache_key"
  fi

  log_ok "$(flutter --no-version-check --version --suppress-analytics | head -n 1)"
  log_ok "$(dart --version 2>&1)"
}

ensure_cocoapods() {
  local pod_ok=false
  if command -v pod >/dev/null 2>&1; then
    local pod_ver=""
    if pod_ver="$(pod --version 2>/dev/null)" && [[ -n "$pod_ver" ]]; then
      log_ok "CocoaPods: $pod_ver ($(command -v pod))"
      pod_ok=true
    else
      log_warn "检测到 pod 命令存在但运行异常（常见于系统 Ruby 版本不兼容），将通过 Homebrew 自动修复..."
    fi
  else
    log_warn "未检测到 CocoaPods"
  fi

  if [[ "$pod_ok" == false ]]; then
    ensure_homebrew
    log_fix "通过 Homebrew 自动安装/修复 CocoaPods..."
    if brew list cocoapods >/dev/null 2>&1; then
      brew reinstall cocoapods || brew upgrade cocoapods || true
      brew link --overwrite cocoapods || true
    else
      brew install cocoapods
      brew link --overwrite cocoapods || true
    fi
    hash -r
    local pod_ver=""
    pod_ver="$(pod --version 2>/dev/null || true)"
    [[ -n "$pod_ver" ]] || die "CocoaPods 自动修复失败，请手动检查 brew install cocoapods"
    log_ok "CocoaPods 已修复就绪: $pod_ver ($(command -v pod))"
  fi
}

# ==============================================================================
# 阶段 3：项目依赖、代码生成与 hiddify-core 原生库
# ==============================================================================
pub_get_needed() {
  # 返回 0 表示需要执行 pub get；返回 1 表示依赖已是最新
  [[ -f "$ROOT_DIR/.dart_tool/package_config.json" ]] || return 0
  # ios/macos 的 Generated.xcconfig 由 pub get 生成，缺失时必须重新执行
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]]; then
    [[ -f "$ROOT_DIR/ios/Flutter/Generated.xcconfig" ]] || return 0
  fi
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
    [[ -f "$ROOT_DIR/macos/Flutter/ephemeral/Flutter-Generated.xcconfig" ]] || return 0
  fi
  local want
  want="sdk=$FLUTTER_VER@$FLUTTER_BIN_DIR;$(hash_files "$ROOT_DIR/pubspec.yaml" "$ROOT_DIR/pubspec.lock" "$ROOT_DIR/.flutter-plugins-dependencies")"
  stamp_matches "$ROOT_DIR/.dart_tool/.build_sh_pub_stamp" "$want" && return 1
  return 0
}

run_pub_get() {
  log_info "执行 flutter pub get..."
  if ! flutter pub get; then
    log_fix "flutter pub get 失败，尝试清理 .dart_tool 并重试..."
    rm -rf .dart_tool/package_config.json .dart_tool/flutter_build
    flutter pub get || die "flutter pub get 失败，请检查网络连接或 pub 镜像配置"
  fi
  stamp_write "$ROOT_DIR/.dart_tool/.build_sh_pub_stamp" \
    "sdk=$FLUTTER_VER@$FLUTTER_BIN_DIR;$(hash_files "$ROOT_DIR/pubspec.yaml" "$ROOT_DIR/pubspec.lock" "$ROOT_DIR/.flutter-plugins-dependencies")"
  log_ok "Flutter 依赖获取完成"
}

codegen_input_hash() {
  # 哈希所有影响生成代码的输入：Dart 源码、资源文件、build/pubspec 配置
  local files_hash
  files_hash="$(
    {
      find "$ROOT_DIR/lib" -type f -name '*.dart' ! -name '*.g.dart' ! -name '*.freezed.dart' -print
      find "$ROOT_DIR/assets" -type f -print
      printf '%s\n' "$ROOT_DIR/build.yaml" "$ROOT_DIR/pubspec.yaml"
      if [[ -f "$ROOT_DIR/pubspec.lock" ]]; then
        printf '%s\n' "$ROOT_DIR/pubspec.lock"
      fi
    } | LC_ALL=C sort | tr '\n' '\0' | xargs -0 shasum -a 256 | shasum -a 256 | awk '{print $1}'
  )"
  printf 'sdk=%s@%s;%s' "$FLUTTER_VER" "$FLUTTER_BIN_DIR" "$files_hash"
}

codegen_outputs_exist() {
  [[ -f "$ROOT_DIR/lib/gen/translations.g.dart" ]] || return 1
  [[ -n "$(find "$ROOT_DIR/lib" -name '*.g.dart' -type f -print -quit 2>/dev/null)" ]]
}

run_codegen() {
  local input_hash="$1"
  log_info "运行 build_runner 生成 Dart 代码..."
  dart run build_runner build --delete-conflicting-outputs
  log_info "运行 slang 生成多语言代码..."
  dart run slang
  stamp_write "$ROOT_DIR/.dart_tool/.build_sh_codegen_stamp" "$input_hash"
  log_ok "代码生成与翻译生成完成"
}

prepare_flutter_project() {
  log_step "4/6 准备 Flutter 依赖与代码生成"
  cd "$ROOT_DIR"

  if [[ "$DO_CLEAN" == true ]]; then
    log_info "执行 flutter clean..."
    flutter clean
  fi

  if [[ "$FORCE_DEPS" == true ]]; then
    log_fix "--force-deps: 强制重新执行 flutter pub get"
    run_pub_get
  elif pub_get_needed; then
    run_pub_get
  else
    log_skip "Flutter 依赖已是最新（pubspec 未变化），跳过 flutter pub get（--force-deps 可强制）"
  fi

  local need_codegen=true
  local codegen_hash=""
  if [[ "$SKIP_CODEGEN" == true ]]; then
    if codegen_outputs_exist; then
      need_codegen=false
      log_skip "已指定 --skip-codegen 且生成文件存在，跳过代码生成"
    else
      log_warn "指定了 --skip-codegen，但未检测到生成文件，自动执行代码生成..."
    fi
  elif [[ "$FORCE_DEPS" != true ]] && codegen_outputs_exist; then
    codegen_hash="$(codegen_input_hash)"
    if stamp_matches "$ROOT_DIR/.dart_tool/.build_sh_codegen_stamp" "$codegen_hash"; then
      need_codegen=false
      log_skip "Dart 代码生成结果已是最新，跳过 build_runner/slang（--force-deps 可强制）"
    fi
  fi

  if [[ "$need_codegen" == true ]]; then
    [[ -n "$codegen_hash" ]] || codegen_hash="$(codegen_input_hash)"
    run_codegen "$codegen_hash"
  fi
}

download_core_tarball() {
  local filename="$1"
  local dest_dir="$2"
  local primary_url fallback_url

  if [[ "$CHANNEL" == "prod" ]]; then
    primary_url="https://github.com/hiddify/hiddify-core/releases/download/v${CORE_VERSION}/${filename}"
    fallback_url="https://github.com/hiddify/hiddify-core/releases/download/draft/${filename}"
  else
    primary_url="https://github.com/hiddify/hiddify-core/releases/download/draft/${filename}"
    fallback_url="https://github.com/hiddify/hiddify-core/releases/download/v${CORE_VERSION}/${filename}"
  fi

  mkdir -p "$dest_dir"
  local tmp_tar
  tmp_tar="$(mktemp /tmp/hiddify-core-XXXXXX.tar.gz)"

  log_info "下载 $filename ..."
  if curl -fL --retry 5 --retry-delay 3 --retry-all-errors -o "$tmp_tar" "$primary_url"; then
    tar -xzf "$tmp_tar" -C "$dest_dir"
    rm -f "$tmp_tar"
    return 0
  fi

  log_warn "主地址下载失败 ($primary_url)，尝试备用地址 ($fallback_url)..."
  if curl -fL --retry 3 --retry-delay 3 --retry-all-errors -o "$tmp_tar" "$fallback_url"; then
    tar -xzf "$tmp_tar" -C "$dest_dir"
    rm -f "$tmp_tar"
    return 0
  fi

  rm -f "$tmp_tar"
  die "无法下载 ${filename}，请检查网络或代理设置"
}

prepare_macos_core_libs() {
  local stamp="$ROOT_DIR/.dart_tool/.build_sh_core_macos_stamp"
  local want="core $CHANNEL $CORE_VERSION"
  local need_download=false
  if [[ "$FORCE_LIBS" == true ]]; then
    need_download=true
  elif [[ ! -f "$ROOT_DIR/hiddify-core/bin/hiddify-core.dylib" ]] \
    || [[ ! -d "$ROOT_DIR/macos/Frameworks/HiddifyTunnelCore.xcframework" ]]; then
    need_download=true
  elif stamp_matches "$stamp" "$want"; then
    log_skip "macOS 核心库已是最新（${want}），跳过下载（--force-libs 可强制更新）"
  elif [[ -f "$stamp" ]]; then
    log_fix "macOS 核心库版本戳记不匹配（$(cat "$stamp") -> ${want}），重新下载..."
    need_download=true
  else
    # 旧版本脚本下载过核心库但没有版本戳记：补齐戳记，避免重复下载
    stamp_write "$stamp" "$want"
    log_skip "macOS 核心库已存在，补写版本戳记（${want}），跳过下载"
  fi

  if [[ "$need_download" == true ]]; then
    log_fix "准备 macOS 原生核心库 (hiddify-core.dylib + HiddifyTunnelCore.xcframework)..."
    mkdir -p "$ROOT_DIR/hiddify-core/bin" "$ROOT_DIR/macos/Frameworks"
    download_core_tarball "hiddify-lib-macos.tar.gz" "$ROOT_DIR/hiddify-core/bin"
    [[ -d "$ROOT_DIR/hiddify-core/bin/HiddifyTunnelCore.xcframework" ]] \
      || die "解压的 hiddify-lib-macos.tar.gz 中缺少 HiddifyTunnelCore.xcframework"
    rm -rf "$ROOT_DIR/macos/Frameworks/HiddifyTunnelCore.xcframework"
    mv "$ROOT_DIR/hiddify-core/bin/HiddifyTunnelCore.xcframework" "$ROOT_DIR/macos/Frameworks/"
    touch "$ROOT_DIR/macos/Frameworks/HiddifyTunnelCore.xcframework/.downloaded"
    stamp_write "$stamp" "$want"
  fi

  # macos/scripts/embed-core-helper.sh 需要 hiddify-core/cmd/launcher/main.c
  if [[ ! -f "$ROOT_DIR/hiddify-core/cmd/launcher/main.c" ]]; then
    local core_rev
    core_rev="$(git -C "$ROOT_DIR" rev-parse HEAD:hiddify-core 2>/dev/null || echo 'main')"
    log_fix "补全 hiddify-core/cmd/launcher/main.c (commit: ${core_rev:0:12})..."
    mkdir -p "$ROOT_DIR/hiddify-core/cmd/launcher"
    curl -fL --retry 5 --retry-delay 3 --retry-all-errors \
      -o "$ROOT_DIR/hiddify-core/cmd/launcher/main.c" \
      "https://raw.githubusercontent.com/hiddify/hiddify-core/${core_rev}/cmd/launcher/main.c" \
      || die "下载 hiddify-core/cmd/launcher/main.c 失败"
  fi

  log_ok "macOS 核心库与 Helper 源码就绪"
}

prepare_ios_core_libs() {
  local stamp="$ROOT_DIR/.dart_tool/.build_sh_core_ios_stamp"
  local want="core $CHANNEL $CORE_VERSION"
  local need_download=false
  if [[ "$FORCE_LIBS" == true ]]; then
    need_download=true
  elif [[ ! -d "$ROOT_DIR/ios/Frameworks/HiddifyCore.xcframework" ]]; then
    need_download=true
  elif stamp_matches "$stamp" "$want"; then
    log_skip "iOS 核心库已是最新（${want}），跳过下载（--force-libs 可强制更新）"
  elif [[ -f "$stamp" ]]; then
    log_fix "iOS 核心库版本戳记不匹配（$(cat "$stamp") -> ${want}），重新下载..."
    need_download=true
  else
    # 旧版本脚本下载过核心库但没有版本戳记：补齐戳记，避免重复下载
    stamp_write "$stamp" "$want"
    log_skip "iOS 核心库已存在，补写版本戳记（${want}），跳过下载"
  fi

  if [[ "$need_download" == true ]]; then
    log_fix "准备 iOS 原生核心库 (HiddifyCore.xcframework)..."
    mkdir -p "$ROOT_DIR/ios/Frameworks"
    rm -rf "$ROOT_DIR/ios/Frameworks/HiddifyCore.xcframework"
    download_core_tarball "hiddify-lib-ios.tar.gz" "$ROOT_DIR/ios/Frameworks"
    [[ -d "$ROOT_DIR/ios/Frameworks/HiddifyCore.xcframework" ]] \
      || die "解压的 hiddify-lib-ios.tar.gz 中缺少 HiddifyCore.xcframework"
    stamp_write "$stamp" "$want"
  fi
  log_ok "iOS 核心库就绪"
}

prepare_core_libs() {
  log_step "5/6 检测与准备 hiddify-core 原生库"
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
    prepare_macos_core_libs
  fi
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]]; then
    prepare_ios_core_libs
  fi
}

# ==============================================================================
# CocoaPods 安装与自愈封装
# ==============================================================================
pods_config_hash() {
  # Pods 配置的内容哈希：Podfile / Podfile.lock / Generated.xcconfig / 插件清单
  local platform_dir="$1"
  local gen=""
  if [[ "$platform_dir" == "ios" ]]; then
    gen="$ROOT_DIR/ios/Flutter/Generated.xcconfig"
  else
    gen="$ROOT_DIR/macos/Flutter/ephemeral/Flutter-Generated.xcconfig"
  fi
  hash_files "$ROOT_DIR/$platform_dir/Podfile" "$ROOT_DIR/$platform_dir/Podfile.lock" \
    "$gen" "$ROOT_DIR/.flutter-plugins-dependencies"
}

pods_up_to_date() {
  # 返回 0 表示 Pods 与当前配置一致，可跳过 pod install
  local platform_dir="$1"
  local lock="$ROOT_DIR/$platform_dir/Podfile.lock"
  local pods_dir="$ROOT_DIR/$platform_dir/Pods"
  local manifest="$pods_dir/Manifest.lock"

  [[ -d "$pods_dir" && -f "$lock" && -f "$manifest" ]] || return 1
  # CocoaPods 自身的判定：Pods/Manifest.lock 必须与 Podfile.lock 完全一致
  cmp -s "$lock" "$manifest" || return 1

  # 用内容哈希而不是 mtime：flutter build 会重写 Generated.xcconfig，
  # 但内容未变化时不应重复执行 pod install
  local stamp="$ROOT_DIR/.dart_tool/.build_sh_pods_${platform_dir}_stamp"
  local want
  want="$(pods_config_hash "$platform_dir")"
  if [[ -f "$stamp" ]]; then
    if stamp_matches "$stamp" "$want"; then
      return 0
    fi
    return 1
  fi
  # 已有 Pods 且 Manifest 与 lock 一致：视为已安装，补写戳记，避免重复 pod install
  stamp_write "$stamp" "$want"
  return 0
}

run_pod_install() {
  local platform_dir="$1"

  if [[ "$FORCE_DEPS" != true ]] && pods_up_to_date "$platform_dir"; then
    log_skip "$platform_dir Pods 已是最新（Podfile.lock 与 Manifest.lock 一致），跳过 pod install（--force-deps 可强制）"
    return 0
  fi

  log_info "在 $platform_dir 执行 pod install..."
  (
    cd "$ROOT_DIR/$platform_dir"
    if pod install; then
      exit 0
    fi
    log_fix "$platform_dir pod install 失败，尝试更新仓库索引 (pod install --repo-update)..."
    if pod install --repo-update; then
      exit 0
    fi
    log_fix "$platform_dir pod install 仍失败，清理 Pods 与 Podfile.lock 缓存后重试..."
    rm -rf Pods
    pod install --repo-update
  ) || die "$platform_dir CocoaPods 依赖安装失败"
  stamp_write "$ROOT_DIR/.dart_tool/.build_sh_pods_${platform_dir}_stamp" \
    "$(pods_config_hash "$platform_dir")"
  log_ok "$platform_dir CocoaPods 依赖安装完成"
}

# ==============================================================================
# 阶段 4：构建 macOS (.app / 可选 .dmg)
# ==============================================================================
resolve_macos_sign_mode() {
  case "$SIGN_MODE" in
    unsigned)
      echo "unsigned"
      ;;
    developer-id|signed)
      echo "developer-id"
      ;;
    app-store)
      echo "app-store"
      ;;
    auto)
      if [[ -f "$ROOT_DIR/macos/Runner/Configs/NetworkExtension.local.xcconfig" ]] \
        && security find-identity -v -p codesigning 2>/dev/null | grep -q "Developer ID Application"; then
        echo "developer-id"
      else
        echo "unsigned"
      fi
      ;;
  esac
}

build_macos() {
  log_step "6/6 [macOS] 开始编译 macOS 客户端 (.app)"
  cd "$ROOT_DIR"

  local effective_sign
  effective_sign="$(resolve_macos_sign_mode)"
  log_info "macOS 签名方式: $effective_sign"

  local extra_defines=()
  local saved_xcconfig="${XCODE_XCCONFIG_FILE:-}"

  case "$effective_sign" in
    unsigned)
      export XCODE_XCCONFIG_FILE="$ROOT_DIR/macos/Runner/Configs/Unsigned.xcconfig"
      ;;
    developer-id)
      unset XCODE_XCCONFIG_FILE || true
      if ! security find-identity -v -p codesigning 2>/dev/null | grep -q "Developer ID Application"; then
        die "未在本机钥匙串检测到 Developer ID Application 证书，请改用 --sign unsigned 或导入证书"
      fi
      ;;
    app-store)
      export XCODE_XCCONFIG_FILE="$ROOT_DIR/macos/Runner/Configs/AppStore.xcconfig"
      extra_defines+=(--dart-define "release=app-store")
      ;;
  esac

  # 校验核心框架存在
  bash "$ROOT_DIR/macos/scripts/build-admin-bgservice-core.sh"

  log_info "执行 flutter build macos --release..."
  local build_cmd=(
    flutter build macos --release
    --target "$BUILD_TARGET"
    --dart-define "sentry_dsn=$SENTRY_DSN"
    --dart-define "commit=$APP_COMMIT"
  )
  if [[ ${#extra_defines[@]} -gt 0 ]]; then
    build_cmd+=("${extra_defines[@]}")
  fi

  if ! "${build_cmd[@]}"; then
    log_fix "首次 macOS 构建失败，尝试修复 macos Pods 依赖并重试..."
    if [[ -f "$ROOT_DIR/macos/Flutter/ephemeral/Flutter-Generated.xcconfig" ]]; then
      run_pod_install "macos"
    fi
    "${build_cmd[@]}" || die "macOS 客户端编译失败"
  fi

  # 恢复外部环境变量
  if [[ -n "$saved_xcconfig" ]]; then
    export XCODE_XCCONFIG_FILE="$saved_xcconfig"
  else
    unset XCODE_XCCONFIG_FILE || true
  fi

  local built_app="$ROOT_DIR/build/macos/Build/Products/Release/Hiddify.app"
  [[ -d "$built_app" ]] || die "未找到编译产物: $built_app"

  mkdir -p "$OUTPUT_DIR"
  local out_app="$OUTPUT_DIR/Hiddify.app"
  rm -rf "$out_app"
  ditto "$built_app" "$out_app"
  log_ok "macOS .app 编译成功: $out_app"

  if [[ "$BUILD_DMG" == true ]]; then
    local out_dmg="$OUTPUT_DIR/Hiddify-macOS-${APP_VERSION}.dmg"
    local stage_dir
    stage_dir="$(mktemp -d)"
    ditto "$out_app" "$stage_dir/Hiddify.app"
    ln -s /Applications "$stage_dir/Applications"
    hdiutil create -volname "Hiddify" -srcfolder "$stage_dir" -format UDZO -ov "$out_dmg" >/dev/null
    rm -rf "$stage_dir"
    log_ok "macOS .dmg 打包成功: $out_dmg"
  fi
}

# ==============================================================================
# 阶段 5：构建 iOS (.ipa) 与真机签名安装 (plumesign)
# ==============================================================================
BUILT_IOS_IPA=""
RESOLVED_DEVICE_ID=""
RESOLVED_DEVICE_UDID=""

read_ios_pbx_value() {
  local key="$1"
  local pbx="$ROOT_DIR/ios/Runner.xcodeproj/project.pbxproj"
  [[ -f "$pbx" ]] || return 0
  sed -n "s/^[[:space:]]*${key} = \([^;]*\);.*/\1/p" "$pbx" | head -n 1 | tr -d '[:space:]"'"'"
}

read_ios_base_bundle_id() {
  local app_plist="$1"
  local base_id=""
  if [[ -f "$app_plist" ]]; then
    base_id="$(plutil -extract BASE_BUNDLE_IDENTIFIER raw -o - "$app_plist" 2>/dev/null || true)"
    if [[ -z "$base_id" || "$base_id" == *'$('* ]]; then
      base_id="$(plutil -extract CFBundleIdentifier raw -o - "$app_plist" 2>/dev/null || true)"
    fi
  fi
  if [[ -z "$base_id" || "$base_id" == *'$('* ]]; then
    base_id="$(awk -F= '/^[[:space:]]*BASE_BUNDLE_IDENTIFIER[[:space:]]*=/ {gsub(/[[:space:]]/, "", $2); print $2; exit}' "$ROOT_DIR/ios/Base.xcconfig" 2>/dev/null || true)"
  fi
  printf '%s\n' "${base_id:-apple.hiddify.com}"
}

verify_ios_app() {
  local app="$1"
  [[ -d "$app" ]] || die "未找到 iOS .app 产物: $app"
  [[ -f "$app/Info.plist" ]] || die "Info.plist 缺失: $app/Info.plist"
  plutil -lint "$app/Info.plist" >/dev/null || die "Info.plist 格式无效: $app/Info.plist"

  local exe_name exe_path
  exe_name="$(plutil -extract CFBundleExecutable raw -o - "$app/Info.plist" 2>/dev/null || basename "$app" .app)"
  exe_path="$app/$exe_name"
  [[ -f "$exe_path" ]] || die "主可执行文件缺失: $exe_path"
  [[ -x "$exe_path" ]] || log_warn "可执行文件缺少可执行权限标记: $exe_path"

  local expected_short_ver="${APP_VERSION%%+*}"
  local got_ver
  got_ver="$(plutil -extract CFBundleShortVersionString raw -o - "$app/Info.plist" 2>/dev/null || true)"
  if [[ -n "$got_ver" && -n "$expected_short_ver" && "$got_ver" != "$expected_short_ver" ]]; then
    log_warn "Info.plist CFBundleShortVersionString ($got_ver) 与 pubspec.yaml ($expected_short_ver) 不一致"
  fi
}

strip_single_bundle_signature() {
  local bundle="$1"
  [[ -d "$bundle" ]] || return 0
  codesign --remove-signature "$bundle" >/dev/null 2>&1 || codesign --remove "$bundle" >/dev/null 2>&1 || true
  rm -rf "$bundle/_CodeSignature" "$bundle/embedded.mobileprovision" "$bundle"/.ldid.*
}

strip_ios_app_signatures() {
  local app="$1"
  local sub
  if [[ -d "$app/Frameworks" ]]; then
    for sub in "$app/Frameworks"/*; do
      [[ -e "$sub" ]] || continue
      if [[ -d "$sub" ]]; then
        strip_single_bundle_signature "$sub"
      else
        codesign --remove-signature "$sub" >/dev/null 2>&1 || true
      fi
    done
  fi
  if [[ -d "$app/PlugIns" ]]; then
    for sub in "$app/PlugIns"/*.appex; do
      [[ -d "$sub" ]] || continue
      strip_single_bundle_signature "$sub"
    done
  fi
  strip_single_bundle_signature "$app"
}

render_ios_entitlements() {
  local src_plist="$1"
  local dst_plist="$2"
  local base_bundle_id="$3"
  local bundle_id="$4"
  local team_id="$5"
  local strip_paid_vpn="${6:-false}"

  [[ -f "$src_plist" ]] || die "未找到 entitlements 文件: $src_plist"
  python3 - "$src_plist" "$dst_plist" "$base_bundle_id" "$bundle_id" "$team_id" "$strip_paid_vpn" <<'PY'
import plistlib, sys

src, dst, base_id, bundle_id, team_id, strip_vpn = sys.argv[1:7]
raw = open(src, "rb").read().decode("utf-8")
raw = raw.replace("$(BASE_BUNDLE_IDENTIFIER)", base_id).replace("${BASE_BUNDLE_IDENTIFIER}", base_id)
data = plistlib.loads(raw.encode("utf-8"))

if team_id and bundle_id:
    data["application-identifier"] = f"{team_id}.{bundle_id}"
    data["com.apple.developer.team-identifier"] = team_id
    data["keychain-access-groups"] = [f"{team_id}.{base_id}"]

if strip_vpn == "true":
    data.pop("com.apple.developer.networking.networkextension", None)
    data.pop("com.apple.developer.networking.vpn.api", None)

with open(dst, "wb") as f:
    plistlib.dump(data, f, fmt=plistlib.FMT_XML, sort_keys=True)
PY

  plutil -lint "$dst_plist" >/dev/null || die "生成的 entitlements plist 无效: $dst_plist"
}

sign_bundle_ad_hoc() {
  local bundle="$1"
  local entitlements="${2:-}"
  if command -v ldid >/dev/null 2>&1; then
    local target="$bundle"
    # Procursus ldid 若直接传入 .app 目录会递归把主包的 entitlements 覆盖到 PlugIns/*.appex 和 Frameworks/* 上；
    # 因此对所有 .app / .appex / .framework 均精确指向其自身 Mach-O 主可执行文件单独签名
    if [[ -d "$bundle" ]]; then
      local bin_name=""
      if [[ -f "$bundle/Info.plist" ]]; then
        bin_name="$(plutil -extract CFBundleExecutable raw -o - "$bundle/Info.plist" 2>/dev/null || true)"
      fi
      if [[ -z "$bin_name" ]]; then
        bin_name="$(basename "$bundle")"
        bin_name="${bin_name%.*}"
      fi
      target="$bundle/$bin_name"
      [[ -f "$target" ]] || return 0
    fi
    if [[ -n "$entitlements" ]]; then
      ldid -S"$entitlements" "$target"
    else
      ldid -S "$target"
    fi
  else
    if [[ "${TROLLSTORE_MODE:-false}" == true ]]; then
      die "TrollStore 模式需要 ldid 将 VPN 与 App Group 权限伪签进二进制（请执行 brew install ldid）"
    fi
    if [[ -n "$entitlements" ]]; then
      codesign --force --sign - \
        --entitlements "$entitlements" \
        --generate-entitlement-der \
        --timestamp=none "$bundle"
    else
      codesign --force --sign - --timestamp=none "$bundle"
    fi
  fi
}

verify_bundle_entitlements() {
  local bundle="$1"
  local exe_name exe_path dump
  exe_name="$(plutil -extract CFBundleExecutable raw -o - "$bundle/Info.plist" 2>/dev/null || true)"
  if [[ -z "$exe_name" ]]; then
    exe_name="$(basename "$bundle")"
    exe_name="${exe_name%.*}"
  fi
  exe_path="$bundle/$exe_name"
  if [[ "${TROLLSTORE_MODE:-false}" == true && ! -f "$exe_path" ]]; then
    die "TrollStore 模式下找不到可执行文件 $exe_path，无法校验权限"
  fi
  if [[ "${TROLLSTORE_MODE:-false}" == true ]] && ! command -v ldid >/dev/null 2>&1; then
    die "TrollStore 模式需要 ldid 校验 entitlements（请执行 brew install ldid）"
  fi
  if command -v ldid >/dev/null 2>&1; then
    dump="$(ldid -e "$exe_path" 2>/dev/null || true)"
  else
    dump="$(codesign -d --entitlements :- "$exe_path" 2>/dev/null || codesign -d --entitlements - "$exe_path" 2>/dev/null || true)"
  fi
  # 巨魔安装不会补上描述文件里没有的权限，缺了 VPN 或 App Group 就必须让构建失败。
  if [[ "${TROLLSTORE_MODE:-false}" == true ]]; then
    local key
    for key in \
      com.apple.security.application-groups \
      com.apple.developer.networking.networkextension \
      packet-tunnel-provider \
      com.apple.developer.networking.vpn.api \
      allow-vpn
    do
      [[ "$dump" == *"$key"* ]] || die "TrollStore 模式下 $exe_path 缺少权限 $key"
    done
  fi
  if [[ "$dump" != *com.apple.security.application-groups* ]]; then
    log_warn "未能在 $exe_path 中读回 com.apple.security.application-groups 权限声明"
  elif [[ "$dump" == *'$(BASE_BUNDLE_IDENTIFIER)'* ]]; then
    die "$exe_path 中的 BASE_BUNDLE_IDENTIFIER 未展开: $dump"
  fi
}

embed_ios_ad_hoc_entitlements() {
  local app="$1"
  local strip_paid_vpn="${2:-false}"
  local base_bundle_id team_id
  base_bundle_id="$(read_ios_base_bundle_id "$app/Info.plist")"
  team_id="$(awk -F= '/^[[:space:]]*DEVELOPMENT_TEAM[[:space:]]*=/ {gsub(/[[:space:]]/, "", $2); print $2; exit}' "$ROOT_DIR/ios/Base.xcconfig" 2>/dev/null || true)"
  team_id="${team_id:-M7Q8ASP66Z}"

  local ent_dir
  ent_dir="$(mktemp -d)"

  local runner_ent="$ent_dir/Runner.entitlements"
  render_ios_entitlements "$ROOT_DIR/ios/Runner/Runner.entitlements" "$runner_ent" "$base_bundle_id" "$base_bundle_id" "$team_id" "$strip_paid_vpn"

  local tunnel_ent="$ent_dir/HiddifyPacketTunnel.entitlements"
  if [[ -f "$ROOT_DIR/ios/HiddifyPacketTunnel/HiddifyPacketTunnel.entitlements" ]]; then
    render_ios_entitlements "$ROOT_DIR/ios/HiddifyPacketTunnel/HiddifyPacketTunnel.entitlements" "$tunnel_ent" "$base_bundle_id" "${base_bundle_id}.HiddifyPacketTunnel" "$team_id" "$strip_paid_vpn"
  fi

  if [[ "${TROLLSTORE_MODE:-false}" == true && "$strip_paid_vpn" != true ]]; then
    [[ -d "$app/PlugIns/HiddifyPacketTunnel.appex" ]] || die "TrollStore 模式下缺少 PlugIns/HiddifyPacketTunnel.appex，无法嵌入 VPN 权限"
    [[ -f "$tunnel_ent" ]] || die "TrollStore 模式下未能生成 HiddifyPacketTunnel entitlements"
  fi

  strip_ios_app_signatures "$app"

  # 由内向外依次对所有 Frameworks、PlugIns(.appex) 和主包进行 Ad-hoc 签名（兼容 TrollStore Lite / Dopamine / codesign）
  if [[ -d "$app/Frameworks" ]]; then
    local fw
    for fw in "$app/Frameworks"/*; do
      [[ -e "$fw" ]] && sign_bundle_ad_hoc "$fw"
    done
  fi

  if [[ -d "$app/PlugIns" ]]; then
    local appex
    for appex in "$app/PlugIns"/*.appex; do
      [[ -d "$appex" ]] || continue
      if [[ "$(basename "$appex")" == "HiddifyPacketTunnel.appex" && -f "$tunnel_ent" ]]; then
        sign_bundle_ad_hoc "$appex" "$tunnel_ent"
        verify_bundle_entitlements "$appex"
      else
        sign_bundle_ad_hoc "$appex"
      fi
    done
  fi

  sign_bundle_ad_hoc "$app" "$runner_ent"
  verify_bundle_entitlements "$app"

  rm -rf "$ent_dir"
}

has_ios_signing_ready() {
  local plist="$1"
  [[ -f "$plist" ]] || return 1
  # 检查钥匙串是否有 Apple Distribution 或 iPhone Distribution 证书
  security find-identity -v -p codesigning 2>/dev/null | grep -Eq "Apple Distribution|iPhone Distribution" || return 1
  # 检查本机是否存在匹配的 provisioning profile
  local profiles_dir="$HOME/Library/MobileDevice/Provisioning Profiles"
  local xcode_profiles_dir="$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles"
  local has_profile=false
  if [[ -d "$profiles_dir" ]] && [[ -n "$(ls -A "$profiles_dir" 2>/dev/null)" ]]; then
    has_profile=true
  elif [[ -d "$xcode_profiles_dir" ]] && [[ -n "$(ls -A "$xcode_profiles_dir" 2>/dev/null)" ]]; then
    has_profile=true
  fi
  [[ "$has_profile" == true ]]
}

resolve_ios_sign_mode() {
  # 即使调用方没把 SIGN_MODE 改成 unsigned（例如本机已有 Distribution 证书的 auto），
  # TrollStore 也必须走无签名 + ldid，不能导出 app-store IPA。
  if [[ "${TROLLSTORE_MODE:-false}" == true ]]; then
    echo "unsigned"
    return
  fi
  local plist="${EXPORT_OPTIONS:-$ROOT_DIR/ios/exportOptions.plist}"
  case "$SIGN_MODE" in
    unsigned)
      echo "unsigned"
      ;;
    signed|app-store)
      echo "signed"
      ;;
    developer-id|auto)
      if has_ios_signing_ready "$plist"; then
        echo "signed"
      else
        echo "unsigned"
      fi
      ;;
  esac
}

package_unsigned_ipa() {
  local app_path="$1"
  local out_ipa="$2"
  verify_ios_app "$app_path"

  local stage_dir
  stage_dir="$(mktemp -d)"
  mkdir -p "$stage_dir/Payload"
  local staged_app="$stage_dir/Payload/$(basename "$app_path")"
  ditto "$app_path" "$staged_app"

  log_info "清理旧签名并嵌入 Ad-hoc Entitlements (Runner + HiddifyPacketTunnel)..."
  embed_ios_ad_hoc_entitlements "$staged_app" false
  verify_ios_app "$staged_app"

  mkdir -p "$(dirname "$out_ipa")"
  rm -f "$out_ipa"
  (
    cd "$stage_dir"
    # 使用 -y 保留符号链接，标准 IPA zip 结构为 Payload/<App>.app
    zip -qry "$out_ipa" Payload
  )
  rm -rf "$stage_dir"
  [[ -s "$out_ipa" ]] || die "生成 IPA 文件失败: $out_ipa"
}

resolve_plumesign_bin() {
  local candidates=()
  if [[ -n "$PLUMESIGN_BIN" ]]; then
    candidates+=("$PLUMESIGN_BIN")
  fi
  candidates+=(
    "/Applications/Impactor.app/Contents/MacOS/plumesign"
    "$HOME/Applications/Impactor.app/Contents/MacOS/plumesign"
  )
  if command -v plumesign >/dev/null 2>&1; then
    candidates+=("$(command -v plumesign)")
  fi
  local parent1 parent2 parent3
  parent1="$(cd "$ROOT_DIR/.." 2>/dev/null && pwd || true)"
  parent2="$(cd "$ROOT_DIR/../.." 2>/dev/null && pwd || true)"
  parent3="$(cd "$ROOT_DIR/../../.." 2>/dev/null && pwd || true)"
  local base
  for base in "$parent1" "$parent2" "$parent3"; do
    [[ -n "$base" ]] || continue
    candidates+=(
      "$base/Impactor/dist/Impactor.app/Contents/MacOS/plumesign"
      "$base/Impactor/package/macos/Impactor.app/Contents/MacOS/plumesign"
      "$base/Impactor/target/release/plumesign"
    )
  done

  local c
  for c in "${candidates[@]}"; do
    if [[ -n "$c" && -x "$c" ]]; then
      printf '%s\n' "$c"
      return 0
    fi
  done
  return 1
}

list_usb_ios_devices() {
  command -v python3 >/dev/null 2>&1 || return 1
  python3 - <<'PY' 2>/dev/null
import plistlib, socket, struct, sys

try:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect("/var/run/usbmuxd")
except OSError:
    sys.exit(1)

body = plistlib.dumps({
    "MessageType": "ListDevices",
    "ClientVersionString": "1",
    "ProgName": "build.sh",
})
sock.sendall(struct.pack("<IIII", 16 + len(body), 1, 8, 1) + body)

def recv_exact(n):
    chunks = []
    while n > 0:
        chunk = sock.recv(n)
        if not chunk:
            raise EOFError("usbmuxd closed connection")
        chunks.append(chunk)
        n -= len(chunk)
    return b"".join(chunks)

length = struct.unpack("<I", recv_exact(16)[:4])[0]
reply = plistlib.loads(recv_exact(length - 16))
found = 0
for dev in reply.get("DeviceList", []):
    props = dev.get("Properties", {})
    if props.get("ConnectionType") != "USB":
        continue
    # SerialNumber 为 USB 设备的 UDID；plumesign --udid 接收 usbmuxd 的数字 DeviceID
    print("%s\t%s" % (props.get("SerialNumber", ""), dev.get("DeviceID", "")))
    found += 1
if not found:
    sys.exit(1)
PY
}

resolve_ios_device() {
  command -v python3 >/dev/null 2>&1 || die "未找到 python3（枚举 usbmuxd USB 设备所需）"
  local devices
  devices="$(list_usb_ios_devices)" || die "未检测到通过 USB 连接的 iOS 设备（请连接数据线或使用 --no-install）"

  RESOLVED_DEVICE_ID=""
  RESOLVED_DEVICE_UDID=""

  if [[ -z "$DEVICE_ID" ]]; then
    local first_line
    first_line="${devices%%$'\n'*}"
    RESOLVED_DEVICE_UDID="${first_line%%$'\t'*}"
    RESOLVED_DEVICE_ID="${first_line#*$'\t'}"
    [[ -n "$RESOLVED_DEVICE_ID" ]] || die "无法从 usbmuxd 响应解析 DeviceID"
    log_info "未指定 --device/--udid，自动选用首个 USB 设备: UDID=$RESOLVED_DEVICE_UDID (DeviceID=$RESOLVED_DEVICE_ID)"
  else
    # 纯数字直接作为 usbmuxd DeviceID；UDID 字符串则映射为对应的数字 DeviceID
    case "$DEVICE_ID" in
      *[!0-9]*)
        local match_line
        match_line="$(printf '%s\n' "$devices" | awk -F'\t' -v s="$DEVICE_ID" \
          'tolower($1) == tolower(s) { print $0; exit }')"
        if [[ -n "$match_line" ]]; then
          RESOLVED_DEVICE_UDID="${match_line%%$'\t'*}"
          RESOLVED_DEVICE_ID="${match_line#*$'\t'}"
        else
          log_warn "未在当前 USB 设备列表中匹配到 $DEVICE_ID，将原样传递给 plumesign"
          RESOLVED_DEVICE_UDID="$DEVICE_ID"
          RESOLVED_DEVICE_ID="$DEVICE_ID"
        fi
        ;;
      *)
        RESOLVED_DEVICE_ID="$DEVICE_ID"
        RESOLVED_DEVICE_UDID="$(printf '%s\n' "$devices" | awk -F'\t' -v d="$DEVICE_ID" '$2 == d { print $1; exit }')"
        ;;
    esac
  fi
}

check_ios_device_os() {
  local target device_os
  target="$(read_ios_pbx_value IPHONEOS_DEPLOYMENT_TARGET)"
  [[ -n "$target" ]] || return 0
  command -v ideviceinfo >/dev/null 2>&1 || return 0
  if [[ -n "$RESOLVED_DEVICE_UDID" ]]; then
    device_os="$(ideviceinfo -u "$RESOLVED_DEVICE_UDID" -k ProductVersion 2>/dev/null | tr -d '[:space:]' || true)"
  else
    device_os="$(ideviceinfo -k ProductVersion 2>/dev/null | tr -d '[:space:]' || true)"
  fi
  [[ -n "$device_os" ]] || return 0
  if ! version_ge "$device_os" "$target"; then
    log_warn "当前设备系统 iOS ${device_os} 低于最低部署要求 iOS ${target}，installd 可能会拒绝安装"
  else
    log_ok "目标真机系统版本: iOS ${device_os}（满足最低要求 >= ${target}）"
  fi
}

repackage_ipa_for_free_apple_id() {
  local src_ipa="$1"
  local dst_ipa="$2"
  local tmp_stage
  tmp_stage="$(mktemp -d)"
  unzip -q "$src_ipa" -d "$tmp_stage"
  local app_bundle
  app_bundle="$(find "$tmp_stage/Payload" -maxdepth 1 -name '*.app' -type d | head -n 1)"
  [[ -n "$app_bundle" ]] || { rm -rf "$tmp_stage"; die "IPA 结构异常，缺少 Payload/*.app: $src_ipa"; }

  # 移除免费个人账号不支持的 PacketTunnel AppEx 与付费专属 VPN/NetworkExtension 权限，保留 App Group
  rm -rf "$app_bundle/PlugIns/HiddifyPacketTunnel.appex"
  embed_ios_ad_hoc_entitlements "$app_bundle" true

  rm -f "$dst_ipa"
  (
    cd "$tmp_stage"
    zip -qry "$dst_ipa" Payload
  )
  rm -rf "$tmp_stage"
}

find_default_ios_ipa() {
  if [[ -n "$IOS_IPA_OVERRIDE" ]]; then
    printf '%s\n' "$IOS_IPA_OVERRIDE"
    return 0
  fi
  if [[ -n "$BUILT_IOS_IPA" && -f "$BUILT_IOS_IPA" ]]; then
    printf '%s\n' "$BUILT_IOS_IPA"
    return 0
  fi
  if [[ "$TROLLSTORE_MODE" == true && -f "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa" ]]; then
    printf '%s\n' "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa"
    return 0
  fi
  if [[ -f "$OUTPUT_DIR/Hiddify-iOS.ipa" && -f "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa" ]]; then
    if [[ "$OUTPUT_DIR/Hiddify-iOS.ipa" -nt "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa" ]]; then
      printf '%s\n' "$OUTPUT_DIR/Hiddify-iOS.ipa"
    else
      printf '%s\n' "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa"
    fi
    return 0
  fi
  [[ -f "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa" ]] && { printf '%s\n' "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa"; return 0; }
  [[ -f "$OUTPUT_DIR/Hiddify-iOS.ipa" ]] && { printf '%s\n' "$OUTPUT_DIR/Hiddify-iOS.ipa"; return 0; }
  printf '%s\n' "$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa"
}

verify_trollstore_app() {
  local app="$1"
  local appex="$app/PlugIns/HiddifyPacketTunnel.appex"
  [[ -d "$appex" ]] || die "TrollStore 模式下缺少 PlugIns/HiddifyPacketTunnel.appex，IPA 没有 VPN 扩展"
  verify_bundle_entitlements "$app"
  verify_bundle_entitlements "$appex"
}

verify_trollstore_ipa() {
  local ipa_path="$1"
  local stage app
  stage="$(mktemp -d)"
  if ! unzip -q "$ipa_path" -d "$stage"; then
    rm -rf "$stage"
    die "无法解压 IPA 以校验 entitlements: $ipa_path"
  fi
  app="$(find "$stage/Payload" -maxdepth 1 -name '*.app' -type d | head -n 1)"
  if [[ -z "$app" ]]; then
    rm -rf "$stage"
    die "IPA 结构异常，缺少 Payload/*.app: $ipa_path"
  fi
  # 校验失败会直接退出，避免把没有 VPN 权限的包推到手机上。
  verify_trollstore_app "$app"
  rm -rf "$stage"
}

install_ios_ipa_trollstore() {
  local ipa_path="$1"
  [[ -f "$ipa_path" ]] || die "未找到待安装的 IPA 文件: $ipa_path"
  log_info "安装前校验 Runner 与 HiddifyPacketTunnel.appex 的 VPN / App Group 权限..."
  verify_trollstore_ipa "$ipa_path"

  log_step "通过 TrollStore / TrollStore Lite 模式推送 IPA 到越狱/巨魔真机"
  resolve_ios_device
  check_ios_device_os

  command -v afcclient >/dev/null 2>&1 || die "未找到 afcclient（请执行 brew install libimobiledevice）"

  local remote_rel="/Downloads/Hiddify.ipa"
  local remote_abs="/var/mobile/Media/Downloads/Hiddify.ipa"
  local afc_args=()
  [[ -n "$RESOLVED_DEVICE_UDID" ]] && afc_args+=(-u "$RESOLVED_DEVICE_UDID")

  afcclient "${afc_args[@]}" mkdir /Downloads >/dev/null 2>&1 || true
  afcclient "${afc_args[@]}" rm "$remote_rel" >/dev/null 2>&1 || true
  log_info "正在通过 USB AFC 传输 $ipa_path -> 真机 $remote_abs ..."
  afcclient "${afc_args[@]}" put "$ipa_path" "$remote_rel"
  log_ok "IPA 已传输至真机路径: $remote_abs"

  # 若本机已配置免密 SSH 到越狱设备的 root 账户，自动通过 trollstorehelper 执行静默安装
  local ssh_installed=false
  if command -v iproxy >/dev/null 2>&1; then
    local local_ssh_port=22288
    local iproxy_args=()
    [[ -n "$RESOLVED_DEVICE_UDID" ]] && iproxy_args+=(-u "$RESOLVED_DEVICE_UDID")
    iproxy "${iproxy_args[@]}" "${local_ssh_port}:22" >/dev/null 2>&1 &
    local iproxy_pid=$!
    sleep 0.4
    if ssh -p "$local_ssh_port" -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5 \
        root@127.0.0.1 \
        "HELPER=\$(find /var/jb/Applications /Applications /var/containers/Bundle/Application -maxdepth 3 -name 'trollstorehelper' 2>/dev/null | head -n 1); [ -x \"\$HELPER\" ] && \"\$HELPER\" install force custom '$remote_abs'"; then
      ssh_installed=true
    fi
    kill "$iproxy_pid" >/dev/null 2>&1 || true
    wait "$iproxy_pid" 2>/dev/null || true
  fi

  if [[ "$ssh_installed" == true ]]; then
    log_ok "已通过 SSH 调用 trollstorehelper 自动完成 Hiddify 巨魔安装（含完整 VPN/PacketTunnel 权限）！"
  else
    log_info "现在请直接在 iPhone 上打开 TrollStore Lite -> 点击右上角 '+' -> Install IPA File -> 选择 Downloads/Hiddify.ipa（或通过 Filza 打开 $remote_abs 用 TrollStore Lite 安装）"
  fi
}

install_ios_ipa() {
  local ipa_path="${1:-$(find_default_ios_ipa)}"
  [[ -f "$ipa_path" ]] || die "未找到待安装的 IPA 文件: $ipa_path（请先编译或通过 --ipa 指定路径）"

  if [[ "$TROLLSTORE_MODE" == true ]]; then
    install_ios_ipa_trollstore "$ipa_path"
    return 0
  fi

  local plumesign=""
  if ! plumesign="$(resolve_plumesign_bin)"; then
    if [[ -n "$PLUMESIGN_BIN" ]]; then
      die "指定的 plumesign 不可执行: $PLUMESIGN_BIN"
    fi
    die "未找到 plumesign（请安装 Impactor.app 或通过 PLUMESIGN / --plumesign 指定路径）"
  fi

  log_step "使用 plumesign 签名并安装到 iOS 真机"
  log_info "IPA 源文件: $ipa_path"
  log_info "签名工具: $plumesign"

  resolve_ios_device
  check_ios_device_os

  # plumesign 会原地改写签名包，复制临时副本以保持输出目录中的原始 IPA 纯净
  local sign_dir
  sign_dir="$(mktemp -d)"
  local work_ipa="$sign_dir/Hiddify.ipa"
  cp -f "$ipa_path" "$work_ipa"

  local args=(
    sign
    --package "$work_ipa"
    --apple-id
    --register-and-install
    --udid "$RESOLVED_DEVICE_ID"
  )
  [[ -n "$CUSTOM_BUNDLE_ID" ]] && args+=(--custom-identifier "$CUSTOM_BUNDLE_ID")
  [[ -n "$CUSTOM_APP_NAME" ]] && args+=(--custom-name "$CUSTOM_APP_NAME")
  if [[ -n "$PROVISION_PATH" ]]; then
    [[ -f "$PROVISION_PATH" ]] || { rm -rf "$sign_dir"; die "指定的描述文件不存在: $PROVISION_PATH"; }
    args+=(--provision "$PROVISION_PATH")
  else
    log_info "未指定 --provision：将由 plumesign 通过 Apple ID 自动注册 App ID / App Group 并签发描述文件"
  fi

  local sign_log="$sign_dir/plumesign.log"
  log_info "正在签名并安装到设备 DeviceID=$RESOLVED_DEVICE_ID (${RESOLVED_DEVICE_UDID:-unknown})..."
  if RUST_LOG="${RUST_LOG:-info}" "$plumesign" "${args[@]}" 2>&1 | tee "$sign_log"; then
    rm -rf "$sign_dir"
    log_ok "已安装 Hiddify ($APP_VERSION) 到设备 ${RESOLVED_DEVICE_UDID:-DeviceID $RESOLVED_DEVICE_ID}"
    return 0
  fi

  # 若因免费个人 Apple ID 不支持 NetworkExtension / Personal VPN 权限、并发创建 App Group 冲突或达 App ID 配额导致失败，
  # 自动在临时副本上移除付费专属 VPN 权限与 PacketTunnel.appex（保留 App Group），再重试一次安装
  if [[ -z "$PROVISION_PATH" ]] && grep -Eiq "capabilit|entitlement|networkextension|vpn|app.?id|0xe8008016|maxQuantity|limit|addApplicationGroup|not available" "$sign_log"; then
    log_fix "检测到当前 Apple ID 可能为免费个人账号（不支持 NetworkExtension/VPN 扩展权限或扩展注册冲突），正在剥离付费专属扩展并仅保留 App Group 重试安装..."
    cp -f "$ipa_path" "$work_ipa.orig"
    repackage_ipa_for_free_apple_id "$work_ipa.orig" "$work_ipa"
    if RUST_LOG="${RUST_LOG:-info}" "$plumesign" "${args[@]}"; then
      rm -rf "$sign_dir"
      log_ok "已以免费 Apple ID 兼容模式安装 Hiddify ($APP_VERSION) 到设备 ${RESOLVED_DEVICE_UDID:-DeviceID $RESOLVED_DEVICE_ID}"
      return 0
    fi
  fi

  rm -rf "$sign_dir"
  die "plumesign 签名或真机安装失败"
}

maybe_install_ios_ipa() {
  local ipa_path="$1"
  case "$INSTALL_MODE" in
    never)
      log_skip "已指定 --no-install，跳过真机签名与安装"
      return 0
      ;;
    always)
      install_ios_ipa "$ipa_path"
      ;;
    auto)
      if ! list_usb_ios_devices >/dev/null 2>&1; then
        log_skip "未检测到 USB 连接的 iOS 设备，跳过自动安装（连接真机后可执行 ./build.sh --install-only 安装）"
        return 0
      fi
      if ! resolve_plumesign_bin >/dev/null 2>&1; then
        log_skip "检测到 USB iOS 设备但未找到 plumesign，跳过自动安装（可通过 PLUMESIGN 指定路径）"
        return 0
      fi
      install_ios_ipa "$ipa_path"
      ;;
  esac
}

build_ios() {
  log_step "6/6 [iOS] 开始编译 iOS 客户端 (.ipa)"
  cd "$ROOT_DIR"

  local export_plist="${EXPORT_OPTIONS:-$ROOT_DIR/ios/exportOptions.plist}"
  local effective_sign
  effective_sign="$(resolve_ios_sign_mode)"
  if [[ "$TROLLSTORE_MODE" == true ]]; then
    [[ "$effective_sign" == "unsigned" ]] || die "内部错误: TrollStore 模式必须走 unsigned 打包，当前为 ${effective_sign}"
    command -v ldid >/dev/null 2>&1 || die "TrollStore 模式需要 ldid 将 VPN 与 App Group 权限伪签进二进制（请执行 brew install ldid）"
    log_info "TrollStore 模式：强制无签名编译，并用 ldid 伪签 VPN 与 App Group 权限（任一缺失则失败）"
  fi
  log_info "iOS 签名方式: $effective_sign"

  # 预先确保 ios/Flutter/Generated.xcconfig 存在并安装 Pods
  if [[ ! -f "$ROOT_DIR/ios/Flutter/Generated.xcconfig" ]]; then
    flutter pub get
  fi
  run_pod_install "ios"

  mkdir -p "$OUTPUT_DIR"

  if [[ "$effective_sign" == "signed" ]]; then
    [[ -f "$export_plist" ]] || die "未找到 iOS exportOptions.plist: $export_plist"
    log_info "执行 flutter build ipa --release --export-options-plist $export_plist ..."
    flutter build ipa --release \
      --target "$BUILD_TARGET" \
      --export-options-plist "$export_plist" \
      --dart-define "sentry_dsn=$SENTRY_DSN" \
      --dart-define "commit=$APP_COMMIT" \
      || die "已签名 iOS IPA 编译失败"

    local built_ipa
    built_ipa="$(find "$ROOT_DIR/build/ios/ipa" -maxdepth 1 -name '*.ipa' -type f | head -n 1)"
    [[ -n "$built_ipa" ]] || die "未在 build/ios/ipa 中找到生成的 .ipa 文件"

    local out_ipa="${IOS_IPA_OVERRIDE:-$OUTPUT_DIR/Hiddify-iOS.ipa}"
    mkdir -p "$(dirname "$out_ipa")"
    cp -f "$built_ipa" "$out_ipa"
    BUILT_IOS_IPA="$out_ipa"
    log_ok "已签名 iOS .ipa 编译成功: $out_ipa"
  else
    log_info "本机未配置 iOS 签名证书/描述文件（或指定了 unsigned），执行无签名编译并嵌入 Ad-hoc Entitlements..."
    flutter build ios --release --no-codesign \
      --target "$BUILD_TARGET" \
      --dart-define "sentry_dsn=$SENTRY_DSN" \
      --dart-define "commit=$APP_COMMIT" \
      || die "无签名 iOS 编译失败"

    local built_app="$ROOT_DIR/build/ios/iphoneos/Runner.app"
    local out_ipa="${IOS_IPA_OVERRIDE:-$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa}"
    log_info "正在将 $built_app 打包为标准 .ipa ..."
    package_unsigned_ipa "$built_app" "$out_ipa"
    BUILT_IOS_IPA="$out_ipa"
    log_ok "无签名 iOS .ipa（已嵌入 Ad-hoc Entitlements）打包成功: ${out_ipa}"
  fi

  maybe_install_ios_ipa "$BUILT_IOS_IPA"
}

# ==============================================================================
# 汇总输出
# ==============================================================================
print_summary() {
  local end_ts elapsed
  end_ts="$(date +%s)"
  elapsed="$((end_ts - START_TS))"

  log_step "构建汇总（总耗时: ${elapsed}s）"
  if [[ "$CHECK_ONLY" == true ]]; then
    log_ok "环境检测与自动修复已全部通过！"
    return 0
  fi

  log_info "输出目录: $OUTPUT_DIR"
  if [[ -d "$OUTPUT_DIR" ]]; then
    local item size
    for item in "$OUTPUT_DIR"/*; do
      [[ -e "$item" ]] || continue
      size="$(du -sh "$item" | awk '{print $1}')"
      printf "  - %-40s (%s)\n" "$(basename "$item")" "$size"
    done
  fi
}

main() {
  parse_args "$@"

  if [[ "$INSTALL_ONLY" == true ]]; then
    export LANG="en_US.UTF-8"
    export LC_ALL="en_US.UTF-8"
    prepend_path "/opt/homebrew/bin"
    prepend_path "/opt/homebrew/sbin"
    prepend_path "/usr/local/bin"
    cd "$ROOT_DIR"
    APP_VERSION="$(grep '^version:' pubspec.yaml | head -n 1 | sed -E 's/^version:[[:space:]]*//' | tr -d "\"'")"
    install_ios_ipa "$(find_default_ios_ipa)"
    exit 0
  fi

  init_base_env
  ensure_basic_tools
  ensure_xcode
  ensure_flutter
  ensure_cocoapods

  if [[ "$CHECK_ONLY" == true ]]; then
    print_summary
    exit 0
  fi

  prepare_flutter_project
  prepare_core_libs

  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
    build_macos
  fi

  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]]; then
    build_ios
  fi

  print_summary
}

# 仅当直接执行时运行；被其他脚本 source 时只加载函数
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
