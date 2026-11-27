#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Hiddify macOS (.app) & iOS (.ipa) 交互式一键构建脚本
# 使用方式：
#   直接运行 ./build.sh（无需也不接受任何命令行参数），全程通过方向键 ↑/↓、空格与回车交互选择。
# 功能：
#   1. 交互式 TUI 向导：方向键选择构建平台、渠道、签名/巨魔模式、真机安装与增量/强制开关
#   2. 自动检测与自愈环境（macOS、Xcode、Homebrew、Flutter SDK、CocoaPods、Rust 工具链及架构）
#   3. 编译 macOS .app（可选 .dmg）和 iOS .ipa（自动识别签名、TrollStore 伪签或无签名打包）
#   4. 幂等增量构建：依赖、代码生成、Pods、hiddify-core 与 Rust 原生模块均按内容哈希缓存跳过
# ==============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$SCRIPT_DIR"

# --- 默认配置 ---
PLATFORM="all"            # all | macos | ios
MACOS_ARCH="arm64"        # arm64 | universal | x86_64
IOS_RUST_ARCH="arm64"     # arm64 | all
CHANNEL="prod"            # prod | dev
SIGN_MODE="auto"          # auto | unsigned | signed | developer-id | app-store
EXPORT_OPTIONS=""         # 自定义 iOS exportOptions.plist 路径
OUTPUT_DIR="$ROOT_DIR/build_output"
HIDDIFY_CORE_DIR="${HIDDIFY_CORE_DIR:-$ROOT_DIR/hiddify-core}"
export HIDDIFY_CORE_DIR
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
SKIP_RUST=false
FORCE_LIBS=false
FORCE_RUST=false
FORCE_DEPS=false
DO_CLEAN=false
CHECK_ONLY=false
SENTRY_DSN="${SENTRY_DSN:-}"
FLUTTER_BIN_DIR=""
FLUTTER_VER=""
RUSTC_VER=""
BUILT_IOS_IPA=""
BUILT_ARTIFACTS=()

START_TS="$(date +%s)"

# --- 颜色与日志 ---
if [[ -t 1 ]]; then
  C_RESET='\033[0m'
  C_BOLD='\033[1m'
  C_DIM='\033[2m'
  C_BLUE='\033[1;34m'
  C_GREEN='\033[1;32m'
  C_YELLOW='\033[1;33m'
  C_RED='\033[1;31m'
  C_CYAN='\033[1;36m'
else
  C_RESET='' C_BOLD='' C_DIM='' C_BLUE='' C_GREEN='' C_YELLOW='' C_RED='' C_CYAN=''
fi

log_step() { printf "\n${C_CYAN}==> %s${C_RESET}\n" "$*"; }
log_info() { printf "${C_BLUE}[INFO]${C_RESET} %s\n" "$*"; }
log_ok()   { printf "${C_GREEN}[OK]${C_RESET}   %s\n" "$*"; }
log_fix()  { printf "${C_YELLOW}[FIX]${C_RESET}  %s\n" "$*"; }
log_skip() { printf "${C_CYAN}[SKIP]${C_RESET} %s\n" "$*"; }
log_warn() { printf "${C_YELLOW}[WARN]${C_RESET} %s\n" "$*"; }
log_err()  { printf "${C_RED}[ERR]${C_RESET}  %s\n" "$*" >&2; }

TUI_IN="${TUI_TTY_IN:-/dev/tty}"
TUI_OUT="${TUI_TTY_OUT:-/dev/tty}"
TUI_FDS_OPEN=false
TUI_CURSOR_HIDDEN=false
TUI_RESULT=0
TUI_MULTI_SELECTED=()

tui_open_fds() {
  if [[ "$TUI_FDS_OPEN" != true ]]; then
    exec 3< "$TUI_IN"
    TUI_FDS_OPEN=true
  fi
}

tui_close_fds() {
  if [[ "$TUI_FDS_OPEN" == true ]]; then
    exec 3<&- 2>/dev/null || true
    TUI_FDS_OPEN=false
  fi
}

tui_show_cursor() {
  if [[ "$TUI_CURSOR_HIDDEN" == true ]]; then
    printf '\033[?25h' >> "$TUI_OUT" 2>/dev/null || true
    TUI_CURSOR_HIDDEN=false
  fi
}

tui_hide_cursor() {
  if [[ "$TUI_CURSOR_HIDDEN" != true ]]; then
    tui_open_fds
    printf '\033[?25l' >> "$TUI_OUT" 2>/dev/null || true
    TUI_CURSOR_HIDDEN=true
  fi
}

tui_cleanup() {
  tui_show_cursor
  tui_close_fds
}

trap 'tui_cleanup' EXIT INT TERM

die() {
  tui_cleanup
  log_err "$@"
  exit 1
}

record_built_artifact() {
  local raw_path="${1:-}"
  [[ -n "$raw_path" && -e "$raw_path" ]] || return 0

  # 去除末尾多余斜杠（兼容 Hiddify.app/ 等目录产物路径）
  while [[ "$raw_path" != "/" && "$raw_path" == */ ]]; do
    raw_path="${raw_path%/}"
  done

  local resolved_dir resolved_path
  resolved_dir="$(cd "$(dirname "$raw_path")" 2>/dev/null && pwd)" || return 0
  resolved_path="${resolved_dir}/$(basename "$raw_path")"

  local existing
  if [[ ${#BUILT_ARTIFACTS[@]} -gt 0 ]]; then
    for existing in "${BUILT_ARTIFACTS[@]}"; do
      if [[ "$existing" == "$resolved_path" ]] || [[ "$existing" -ef "$resolved_path" ]]; then
        return 0
      fi
    done
  fi

  BUILT_ARTIFACTS+=("$resolved_path")
}

# ==============================================================================
# 交互式方向键 TUI 引擎（兼容 macOS 默认 Bash 3.2）
# ==============================================================================
TUI_KEY=""

tui_read_key() {
  tui_open_fds
  local k="" c1="" c2=""
  if ! IFS= read -r -s -n 1 k <&3; then
    TUI_KEY="EOF"
    return 0
  fi
  if [[ -z "$k" ]]; then
    TUI_KEY="ENTER"
    return 0
  fi
  if [[ "$k" == $'\x1b' ]]; then
    # Bash 3.2 仅支持整数秒超时；方向键后续字节已在同一 fd 缓冲中，会瞬间返回
    if IFS= read -r -s -n 1 -t 1 c1 <&3; then
      if [[ "$c1" == "[" || "$c1" == "O" ]]; then
        if IFS= read -r -s -n 1 -t 1 c2 <&3; then
          case "$c2" in
            A) TUI_KEY="UP"; return 0 ;;
            B) TUI_KEY="DOWN"; return 0 ;;
            C) TUI_KEY="RIGHT"; return 0 ;;
            D) TUI_KEY="LEFT"; return 0 ;;
          esac
        fi
      fi
    fi
    TUI_KEY="ESC"
    return 0
  fi
  case "$k" in
    " ") TUI_KEY="SPACE" ;;
    $'\x7f'|$'\x08') TUI_KEY="BACK" ;;
    k|K) TUI_KEY="UP" ;;
    j|J) TUI_KEY="DOWN" ;;
    b|B) TUI_KEY="BACK" ;;
    a|A) TUI_KEY="ALL" ;;
    q|Q) TUI_KEY="QUIT" ;;
    [1-9]) TUI_KEY="NUM_$k" ;;
    *) TUI_KEY="OTHER" ;;
  esac
}

# 单选方向键菜单
# 参数: $1=标题, $2=默认索引(0起), $3=是否允许返回(true/false), 后续为选项列表
# 可选全局变量 TUI_NOTE: 非空时在标题下方额外显示一行说明提示（调用后自动清空）
# 返回: 0=确认(结果写入 TUI_RESULT), 2=返回上一步
TUI_NOTE=""
tui_select() {
  local title="$1"
  local cur="$2"
  local allow_back="$3"
  shift 3
  local options=("$@")
  local count=${#options[@]}
  local note="$TUI_NOTE"
  TUI_NOTE=""
  local total_lines=$((count + 3))
  [[ -n "$note" ]] && total_lines=$((total_lines + 1))
  local first_draw=true
  local i hint

  if [[ "$allow_back" == true ]]; then
    hint="↑/↓ 选择 · Enter 确认 · ←/b 返回上一步 · q 退出"
  else
    hint="↑/↓ 选择 · Enter 确认 · 数字键快选 · q 退出"
  fi

  tui_hide_cursor
  while true; do
    if [[ "$first_draw" == false ]]; then
      printf '\033[%dA\r' "$total_lines" >> "$TUI_OUT"
    fi
    first_draw=false

    printf "${C_CYAN}┌─${C_RESET} ${C_BOLD}%s${C_RESET}\033[K\n" "$title" >> "$TUI_OUT"
    if [[ -n "$note" ]]; then
      printf "${C_CYAN}│${C_RESET}  ${C_YELLOW}💡 %s${C_RESET}\033[K\n" "$note" >> "$TUI_OUT"
    fi
    printf "${C_CYAN}│${C_RESET}  ${C_DIM}%s${C_RESET}\033[K\n" "$hint" >> "$TUI_OUT"
    for ((i = 0; i < count; i++)); do
      if [[ $i -eq $cur ]]; then
        printf "${C_CYAN}│${C_RESET}  ${C_GREEN}❯ ● %s${C_RESET}\033[K\n" "${options[$i]}" >> "$TUI_OUT"
      else
        printf "${C_CYAN}│${C_RESET}    ○ %s\033[K\n" "${options[$i]}" >> "$TUI_OUT"
      fi
    done
    printf "${C_CYAN}└──────────────────────────────────────────────────────────${C_RESET}\033[K\n" >> "$TUI_OUT"

    tui_read_key
    case "$TUI_KEY" in
      UP)
        cur=$(((cur - 1 + count) % count))
        ;;
      DOWN)
        cur=$(((cur + 1) % count))
        ;;
      ENTER|RIGHT)
        printf '\033[%dA\r\033[J' "$total_lines" >> "$TUI_OUT"
        printf "${C_GREEN}✔${C_RESET} ${C_BOLD}%s${C_RESET} ${C_DIM}›${C_RESET} ${C_CYAN}%s${C_RESET}\n" \
          "$title" "${options[$cur]}" >> "$TUI_OUT"
        tui_show_cursor
        TUI_RESULT=$cur
        return 0
        ;;
      LEFT|BACK)
        if [[ "$allow_back" == true ]]; then
          # 擦除当前菜单及上一条已确认摘要行
          printf '\033[%dA\r\033[J' "$((total_lines + 1))" >> "$TUI_OUT"
          tui_show_cursor
          return 2
        fi
        ;;
      NUM_*)
        local num="${TUI_KEY#NUM_}"
        if [[ $num -ge 1 && $num -le $count ]]; then
          cur=$((num - 1))
        fi
        ;;
      QUIT|ESC|EOF)
        printf '\033[%dA\r\033[J' "$total_lines" >> "$TUI_OUT"
        tui_show_cursor
        log_warn "已取消操作并退出"
        exit 0
        ;;
    esac
  done
}

# 多选方向键开关面板
# 参数: $1=标题, $2=初始选中状态串(空格分隔 0/1，如 "0 1 0"), 后续为选项列表
# 返回: 0=确认(结果数组写入 TUI_MULTI_SELECTED), 2=返回上一步
tui_multiselect() {
  local title="$1"
  local init_flags="$2"
  shift 2
  local options=("$@")
  local count=${#options[@]}
  local total_lines=$((count + 3))
  local cur=0
  local first_draw=true
  local i

  TUI_MULTI_SELECTED=()
  local flag_arr=()
  if [[ -n "$init_flags" ]]; then
    read -r -a flag_arr <<< "$init_flags"
  fi
  for ((i = 0; i < count; i++)); do
    TUI_MULTI_SELECTED[$i]="${flag_arr[$i]:-0}"
  done

  tui_hide_cursor
  while true; do
    if [[ "$first_draw" == false ]]; then
      printf '\033[%dA\r' "$total_lines" >> "$TUI_OUT"
    fi
    first_draw=false

    printf "${C_CYAN}┌─${C_RESET} ${C_BOLD}%s${C_RESET}\033[K\n" "$title" >> "$TUI_OUT"
    printf "${C_CYAN}│${C_RESET}  ${C_DIM}↑/↓ 移动 · Space 勾选/取消 · a 全量重编 · Enter 下一步 · ←/b 返回 · q 退出${C_RESET}\033[K\n" >> "$TUI_OUT"
    for ((i = 0; i < count; i++)); do
      local mark="[ ]"
      local color=""
      if [[ "${TUI_MULTI_SELECTED[$i]}" == "1" ]]; then
        mark="[✔]"
        color="$C_GREEN"
      fi
      if [[ $i -eq $cur ]]; then
        printf "${C_CYAN}│${C_RESET}  ${C_CYAN}❯${C_RESET} ${color}${C_BOLD}%s %s${C_RESET}\033[K\n" "$mark" "${options[$i]}" >> "$TUI_OUT"
      else
        printf "${C_CYAN}│${C_RESET}    ${color}%s %s${C_RESET}\033[K\n" "$mark" "${options[$i]}" >> "$TUI_OUT"
      fi
    done
    printf "${C_CYAN}└──────────────────────────────────────────────────────────${C_RESET}\033[K\n" >> "$TUI_OUT"

    tui_read_key
    case "$TUI_KEY" in
      UP)
        cur=$(((cur - 1 + count) % count))
        ;;
      DOWN)
        cur=$(((cur + 1) % count))
        ;;
      SPACE)
        if [[ "${TUI_MULTI_SELECTED[$cur]}" == "1" ]]; then
          TUI_MULTI_SELECTED[$cur]="0"
        else
          TUI_MULTI_SELECTED[$cur]="1"
          tui_resolve_multiselect_conflicts "$cur" "${options[@]}"
        fi
        ;;
      ALL)
        # "a 全量重编" 仅切换正向构建/刷新项，自动排除反向 "跳过..." 与弹窗式 "自定义高级设置..."
        local any_positive_unset=false
        for ((i = 0; i < count; i++)); do
          case "${options[$i]}" in
            跳过*|自定义高级设置*) continue ;;
          esac
          [[ "${TUI_MULTI_SELECTED[$i]}" == "0" ]] && any_positive_unset=true
        done
        for ((i = 0; i < count; i++)); do
          case "${options[$i]}" in
            跳过*|自定义高级设置*)
              TUI_MULTI_SELECTED[$i]="0"
              ;;
            *)
              if [[ "$any_positive_unset" == true ]]; then
                TUI_MULTI_SELECTED[$i]="1"
              else
                TUI_MULTI_SELECTED[$i]="0"
              fi
              ;;
          esac
        done
        ;;
      NUM_*)
        local num="${TUI_KEY#NUM_}"
        if [[ $num -ge 1 && $num -le $count ]]; then
          local idx=$((num - 1))
          cur=$idx
          if [[ "${TUI_MULTI_SELECTED[$idx]}" == "1" ]]; then
            TUI_MULTI_SELECTED[$idx]="0"
          else
            TUI_MULTI_SELECTED[$idx]="1"
            tui_resolve_multiselect_conflicts "$idx" "${options[@]}"
          fi
        fi
        ;;
      ENTER)
        local picked_summary=""
        local short_label
        for ((i = 0; i < count; i++)); do
          if [[ "${TUI_MULTI_SELECTED[$i]}" == "1" ]]; then
            short_label="${options[$i]%% (*}"
            if [[ -z "$picked_summary" ]]; then
              picked_summary="$short_label"
            else
              picked_summary="$picked_summary, $short_label"
            fi
          fi
        done
        [[ -n "$picked_summary" ]] || picked_summary="默认增量构建"
        printf '\033[%dA\r\033[J' "$total_lines" >> "$TUI_OUT"
        printf "${C_GREEN}✔${C_RESET} ${C_BOLD}%s${C_RESET} ${C_DIM}›${C_RESET} ${C_CYAN}%s${C_RESET}\n" \
          "$title" "$picked_summary" >> "$TUI_OUT"
        tui_show_cursor
        return 0
        ;;
      LEFT|BACK)
        printf '\033[%dA\r\033[J' "$((total_lines + 1))" >> "$TUI_OUT"
        tui_show_cursor
        return 2
        ;;
      QUIT|ESC|EOF)
        printf '\033[%dA\r\033[J' "$total_lines" >> "$TUI_OUT"
        tui_show_cursor
        log_warn "已取消操作并退出"
        exit 0
        ;;
    esac
  done
}

# 当勾选某项时自动取消与其互斥的相反项（如"强制重编 Rust"与"跳过 Rust"、"强制刷新/clean"与"跳过代码生成"）
tui_resolve_multiselect_conflicts() {
  local toggled_idx="$1"
  shift
  local opts=("$@")
  local chosen="${opts[$toggled_idx]}"
  local j
  for ((j = 0; j < ${#opts[@]}; j++)); do
    [[ $j -eq $toggled_idx ]] && continue
    case "$chosen" in
      强制重编\ Rust*)
        [[ "${opts[$j]}" == 跳过\ Rust* ]] && TUI_MULTI_SELECTED[$j]="0"
        ;;
      跳过\ Rust*)
        [[ "${opts[$j]}" == 强制重编\ Rust* ]] && TUI_MULTI_SELECTED[$j]="0"
        ;;
      强制刷新依赖*|构建前执行\ flutter\ clean*)
        [[ "${opts[$j]}" == 跳过\ Dart* ]] && TUI_MULTI_SELECTED[$j]="0"
        ;;
      跳过\ Dart*)
        [[ "${opts[$j]}" == 强制刷新依赖* || "${opts[$j]}" == 构建前执行\ flutter\ clean* ]] && TUI_MULTI_SELECTED[$j]="0"
        ;;
    esac
  done
}

TUI_INPUT_VALUE=""
tui_prompt_input() {
  local label="$1"
  local default_val="${2:-}"
  tui_show_cursor
  if [[ -n "$default_val" ]]; then
    printf "${C_CYAN}  ▸ %s${C_RESET} ${C_DIM}[默认: %s]${C_RESET}: " "$label" "$default_val" >> "$TUI_OUT"
  else
    printf "${C_CYAN}  ▸ %s${C_RESET} ${C_DIM}[直接回车留空]${C_RESET}: " "$label" >> "$TUI_OUT"
  fi
  local line=""
  tui_open_fds
  IFS= read -r line <&3 || true
  # 剥离误按方向键/ESC 产生的控制序列与首尾空白
  line="$(printf '%s' "$line" | LC_ALL=C sed $'s/\x1b\\[[0-9;]*[A-Za-z~]//g; s/\x1b//g; s/^[[:space:]]*//; s/[[:space:]]*$//')"
  if [[ -z "$line" ]]; then
    TUI_INPUT_VALUE="$default_val"
  else
    case "$line" in
      "~/"*) line="${HOME}/${line#\~/}" ;;
      "~")   line="$HOME" ;;
    esac
    TUI_INPUT_VALUE="$line"
  fi
}

select_usb_device_interactive() {
  local devices_raw
  devices_raw="$(list_usb_ios_devices 2>/dev/null || true)"
  if [[ -z "$devices_raw" ]]; then
    return 0
  fi

  local udids=()
  local dev_ids=()
  local labels=("自动选用首个已连接的 USB 设备")
  local line u d
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    u="${line%%$'\t'*}"
    d="${line#*$'\t'}"
    udids+=("$u")
    dev_ids+=("$d")
    labels+=("真机 UDID: ${u} (DeviceID: ${d})")
  done <<< "$devices_raw"

  if [[ ${#udids[@]} -le 1 ]]; then
    DEVICE_ID=""
    return 0
  fi

  if tui_select "检测到多台 USB iOS 设备，请选择安装目标" 0 true "${labels[@]}"; then
    if [[ $TUI_RESULT -eq 0 ]]; then
      DEVICE_ID=""
    else
      DEVICE_ID="${udids[$((TUI_RESULT - 1))]}"
    fi
    return 0
  else
    return 2
  fi
}

prompt_advanced_settings() {
  local need_ios="$1"
  printf "\n${C_CYAN}  ── 高级路径与重签参数设置（直接回车保持默认）──${C_RESET}\n" >> "$TUI_OUT"
  tui_prompt_input "产物输出目录" "$OUTPUT_DIR"
  OUTPUT_DIR="$TUI_INPUT_VALUE"

  if [[ "$need_ios" == true ]]; then
    if [[ "$SIGN_MODE" == "signed" || "$SIGN_MODE" == "app-store" ]]; then
      tui_prompt_input "iOS ExportOptions.plist 路径" "${EXPORT_OPTIONS:-$ROOT_DIR/ios/exportOptions.plist}"
      EXPORT_OPTIONS="$TUI_INPUT_VALUE"
    fi
    if [[ "$TROLLSTORE_MODE" != true && "$INSTALL_MODE" != "never" ]]; then
      tui_prompt_input "自定义重签 Bundle ID" "$CUSTOM_BUNDLE_ID"
      CUSTOM_BUNDLE_ID="$TUI_INPUT_VALUE"
      tui_prompt_input "自定义应用显示名称" "$CUSTOM_APP_NAME"
      CUSTOM_APP_NAME="$TUI_INPUT_VALUE"
      tui_prompt_input "自定义 Provision 描述文件路径" "$PROVISION_PATH"
      PROVISION_PATH="$TUI_INPUT_VALUE"
      tui_prompt_input "指定 plumesign 可执行文件路径" "$PLUMESIGN_BIN"
      PLUMESIGN_BIN="$TUI_INPUT_VALUE"
    fi
  fi
  printf "\n" >> "$TUI_OUT"
}

run_install_only_wizard() {
  local step=1
  local ipa_candidates=()
  local ipa_labels=()
  local f

  for f in "$OUTPUT_DIR"/*.ipa "$ROOT_DIR"/*.ipa; do
    [[ -f "$f" ]] || continue
    ipa_candidates+=("$f")
    ipa_labels+=("$(basename "$f") (${f#$ROOT_DIR/})")
  done
  ipa_labels+=("手动输入其他 .ipa 文件路径...")

  local sel_ipa_idx=0
  local sel_mode_idx=0
  local sel_custom_idx=0

  while true; do
    case $step in
      1)
        if tui_select "选择待安装的 iOS .ipa 安装包" "$sel_ipa_idx" true "${ipa_labels[@]}"; then
          sel_ipa_idx=$TUI_RESULT
          if [[ $sel_ipa_idx -lt ${#ipa_candidates[@]} ]]; then
            IOS_IPA_OVERRIDE="${ipa_candidates[$sel_ipa_idx]}"
          else
            tui_prompt_input "请输入待安装的 .ipa 完整路径" "${IOS_IPA_OVERRIDE:-$OUTPUT_DIR/Hiddify-iOS-unsigned.ipa}"
            IOS_IPA_OVERRIDE="$TUI_INPUT_VALUE"
          fi
          step=2
        else
          return 2
        fi
        ;;
      2)
        TUI_NOTE="免费个人 Apple ID 无 NetworkExtension/VPN 权限会导致代理不可用！无 \$99 付费证书必须借助 TrollStore"
        if tui_select "选择真机安装方式" "$sel_mode_idx" true \
          "TrollStore / 巨魔模式推送安装 (推荐 · 免费账号必选，保留完整 VPN 代理权限)" \
          "付费开发者 Apple ID 重签安装 (plumesign · 需 \$99/年账号，免费账号将自动转巨魔)"; then
          sel_mode_idx=$TUI_RESULT
          if [[ $sel_mode_idx -eq 0 ]]; then
            TROLLSTORE_MODE=true
            SIGN_MODE="unsigned"
            step=4
          else
            TROLLSTORE_MODE=false
            step=3
          fi
        else
          step=1
        fi
        ;;
      3)
        if tui_select "重签安装参数设置" "$sel_custom_idx" true \
          "使用默认 Bundle ID 与自动描述文件安装 (推荐)" \
          "自定义 Bundle ID / 应用显示名 / 描述文件路径"; then
          sel_custom_idx=$TUI_RESULT
          if [[ $sel_custom_idx -eq 1 ]]; then
            tui_prompt_input "自定义重签 Bundle ID" "$CUSTOM_BUNDLE_ID"
            CUSTOM_BUNDLE_ID="$TUI_INPUT_VALUE"
            tui_prompt_input "自定义应用显示名称" "$CUSTOM_APP_NAME"
            CUSTOM_APP_NAME="$TUI_INPUT_VALUE"
            tui_prompt_input "自定义 Provision 描述文件路径" "$PROVISION_PATH"
            PROVISION_PATH="$TUI_INPUT_VALUE"
          fi
          step=4
        else
          step=2
        fi
        ;;
      4)
        if select_usb_device_interactive; then
          return 0
        else
          if [[ "$TROLLSTORE_MODE" == true ]]; then
            step=2
          else
            step=3
          fi
        fi
        ;;
    esac
  done
}

run_interactive_wizard() {
  if [[ ! -r "$TUI_IN" || ! -w "$TUI_OUT" ]]; then
    die "当前非交互式终端环境，请在终端中直接运行 ./build.sh 并使用方向键选择"
  fi
  tui_open_fds

  cd "$ROOT_DIR"
  local ver_preview commit_preview
  ver_preview="$(grep '^version:' pubspec.yaml 2>/dev/null | head -n 1 | sed -E 's/^version:[[:space:]]*//' | tr -d "\"'" || echo 'unknown')"
  commit_preview="$(git rev-parse --short=7 HEAD 2>/dev/null || echo 'unknown')"

  printf "\n${C_CYAN}╔══════════════════════════════════════════════════════════╗${C_RESET}\n" >> "$TUI_OUT"
  printf "${C_CYAN}║${C_RESET}  ${C_BOLD}Hiddify 一键构建控制台 (Flutter + Go Core + Rust FFI)${C_RESET}   ${C_CYAN}║${C_RESET}\n" >> "$TUI_OUT"
  printf "${C_CYAN}║${C_RESET}  ${C_DIM}版本: %-18s Commit: %-21s${C_RESET} ${C_CYAN}║${C_RESET}\n" "$ver_preview" "$commit_preview" >> "$TUI_OUT"
  printf "${C_CYAN}╚══════════════════════════════════════════════════════════╝${C_RESET}\n\n" >> "$TUI_OUT"

  local step=1
  local sel_task=0
  local sel_check_plat=0
  local sel_channel=0
  local sel_arch=0
  local sel_sign=0
  local sel_install=0
  local multi_state=""

  while true; do
    case $step in
      1)
        # 重置任务相关标志
        INSTALL_ONLY=false
        CHECK_ONLY=false
        TROLLSTORE_MODE=false
        BUILD_DMG=false
        SKIP_CODEGEN=false
        SKIP_RUST=false
        FORCE_LIBS=false
        FORCE_RUST=false
        FORCE_DEPS=false
        DO_CLEAN=false
        BUILT_IOS_IPA=""
        BUILT_ARTIFACTS=()

        tui_select "选择要执行的任务" "$sel_task" false \
          "全平台一键构建 (macOS .app + iOS .ipa)" \
          "仅构建 iOS 客户端 (.ipa 打包 / 真机安装)" \
          "仅构建 macOS 客户端 (.app / 可选 .dmg)" \
          "仅安装已有 iOS .ipa 到 USB 真机 (跳过编译)" \
          "仅检测并自动修复构建环境 (Xcode / Flutter / Pods / Rust)"
        sel_task=$TUI_RESULT

        case $sel_task in
          0) PLATFORM="all"; step=2 ;;
          1) PLATFORM="ios"; step=2 ;;
          2) PLATFORM="macos"; step=2 ;;
          3)
            PLATFORM="ios"
            INSTALL_ONLY=true
            INSTALL_MODE="always"
            if run_install_only_wizard; then
              step=6
            else
              step=1
            fi
            ;;
          4)
            CHECK_ONLY=true
            step=10
            ;;
        esac
        ;;

      10)
        if tui_select "选择要检测修复的目标平台环境" "$sel_check_plat" true \
          "全平台环境 (macOS + iOS + Rust 全架构)" \
          "仅 iOS 构建环境 (iOS SDK + Pods + Rust iOS)" \
          "仅 macOS 构建环境 (macOS SDK + Pods + Rust Universal2)"; then
          sel_check_plat=$TUI_RESULT
          case $sel_check_plat in
            0) PLATFORM="all" ;;
            1) PLATFORM="ios" ;;
            2) PLATFORM="macos" ;;
          esac
          step=6
        else
          step=1
        fi
        ;;

      2)
        TUI_NOTE="两个版本均已内置启用 Rust 原生加速模块！区别仅为正式发布环境与开发调试环境"
        if tui_select "选择构建渠道 (日常使用请直接回车选 prod)" "$sel_channel" true \
          "正式渠道 prod [含Rust] — 稳定版 Core + 关闭调试开销 (日常使用推荐)" \
          "开发渠道 dev  [含Rust] — Draft 测试版 Core + 开启调试日志 (排障测试用)"; then
          sel_channel=$TUI_RESULT
          if [[ $sel_channel -eq 0 ]]; then
            CHANNEL="prod"
          else
            CHANNEL="dev"
          fi
          step=25
        else
          step=1
        fi
        ;;

      25)
        if [[ "$PLATFORM" == "ios" ]]; then
          [[ $sel_arch -le 1 ]] || sel_arch=0
          TUI_NOTE="Go 核心 (HiddifyCore) 在 iOS 仅打包 arm64 真机切片；此处控制 Rust 模块是否跳过模拟器"
          if tui_select "选择 iOS Rust 模块构建架构" "$sel_arch" true \
            "arm64 真机单架构 — 跳过模拟器编译，提速一倍 (真机使用推荐)" \
            "arm64 + 模拟器全架构 — 同时包含 Xcode 模拟器切片"; then
            sel_arch=$TUI_RESULT
            if [[ $sel_arch -eq 0 ]]; then
              IOS_RUST_ARCH="arm64"
            else
              IOS_RUST_ARCH="all"
            fi
            step=3
          else
            step=2
          fi
        else
          [[ $sel_arch -le 2 ]] || sel_arch=0
          TUI_NOTE="单架构将自动裁剪 Go 内核与 App 二进制并单架构编译 Rust，全部协议与逻辑 100% 不变"
          if tui_select "选择目标架构 (单架构可使 macOS 核心体积减半 ~119MB)" "$sel_arch" true \
            "arm64 单架构 — Apple Silicon (M1-M4) / iOS 真机单架构 (体积减半 · 推荐)" \
            "universal 双架构 — macOS 通用包 (arm64+x86_64) / iOS 含模拟器" \
            "x86_64 单架构 — 仅 Intel Mac (iOS 仍为 arm64 真机)"; then
            sel_arch=$TUI_RESULT
            case $sel_arch in
              0) MACOS_ARCH="arm64"; IOS_RUST_ARCH="arm64" ;;
              1) MACOS_ARCH="universal"; IOS_RUST_ARCH="all" ;;
              2) MACOS_ARCH="x86_64"; IOS_RUST_ARCH="arm64" ;;
            esac
            step=3
          else
            step=2
          fi
        fi
        ;;

      3)
        TROLLSTORE_MODE=false
        if [[ "$PLATFORM" == "ios" ]]; then
          [[ $sel_sign -le 4 ]] || sel_sign=0
          TUI_NOTE="免费个人 Apple ID 签名无 VPN 扩展权限，代理软件无法工作！无付费证书真机使用必选 TrollStore"
          if tui_select "选择 iOS 签名与打包方式" "$sel_sign" true \
            "TrollStore / 巨魔模式 (推荐 · 无签名编译 + ldid 伪签完整 VPN/AppGroup 权限)" \
            "自动检测签名 (有付费证书导出签名包，否则生成无签名 IPA)" \
            "无签名 / Ad-hoc 打包 (供 \$99 付费开发者账号后续重签)" \
            "正式付费证书签名导出 (使用 ios/exportOptions.plist)" \
            "App Store 归档导出模式"; then
            sel_sign=$TUI_RESULT
            case $sel_sign in
              0) SIGN_MODE="unsigned"; TROLLSTORE_MODE=true ;;
              1) SIGN_MODE="auto" ;;
              2) SIGN_MODE="unsigned" ;;
              3) SIGN_MODE="signed" ;;
              4) SIGN_MODE="app-store" ;;
            esac
            step=4
          else
            step=25
          fi
        elif [[ "$PLATFORM" == "macos" ]]; then
          [[ $sel_sign -le 3 ]] || sel_sign=0
          if tui_select "选择 macOS 签名方式" "$sel_sign" true \
            "自动检测签名 (有 Developer ID 证书则签名，否则 Ad-hoc 无签名)" \
            "无签名 / Ad-hoc 构建 (本机直接运行)" \
            "Developer ID 证书签名构建" \
            "Mac App Store 签名构建"; then
            sel_sign=$TUI_RESULT
            case $sel_sign in
              0) SIGN_MODE="auto" ;;
              1) SIGN_MODE="unsigned" ;;
              2) SIGN_MODE="developer-id" ;;
              3) SIGN_MODE="app-store" ;;
            esac
            INSTALL_MODE="never"
            step=5
          else
            step=25
          fi
        else
          [[ $sel_sign -le 4 ]] || sel_sign=0
          TUI_NOTE="iOS 免费个人 Apple ID 无 VPN 扩展权限会导致代理不可用！无付费证书真机使用必选 TrollStore"
          if tui_select "选择双端签名与打包方式" "$sel_sign" true \
            "iOS 启用 TrollStore 巨魔伪签 (完整 VPN 权限) + macOS 自动检测签名 (推荐)" \
            "自动检测双端付费证书签名环境" \
            "双端无签名 / Ad-hoc 构建 (macOS Ad-hoc + iOS 无签名 IPA)" \
            "双端正式付费证书签名 (Developer ID + exportOptions.plist)" \
            "双端 App Store 签名模式"; then
            sel_sign=$TUI_RESULT
            case $sel_sign in
              0) SIGN_MODE="auto"; TROLLSTORE_MODE=true ;;
              1) SIGN_MODE="auto" ;;
              2) SIGN_MODE="unsigned" ;;
              3) SIGN_MODE="signed" ;;
              4) SIGN_MODE="app-store" ;;
            esac
            step=4
          else
            step=25
          fi
        fi
        ;;

      4)
        if [[ "$TROLLSTORE_MODE" == true ]]; then
          [[ $sel_install -le 1 ]] || sel_install=0
          if tui_select "选择巨魔真机安装策略" "$sel_install" true \
            "编译完成后自动通过 USB 推送并安装到巨魔真机" \
            "仅编译打包巨魔 .ipa 文件，暂不推送真机"; then
            sel_install=$TUI_RESULT
            if [[ $sel_install -eq 0 ]]; then
              INSTALL_MODE="always"
              select_usb_device_interactive || continue
            else
              INSTALL_MODE="never"
            fi
            step=5
          else
            step=3
          fi
        else
          [[ $sel_install -le 3 ]] || sel_install=0
          TUI_NOTE="免费个人 Apple ID 签名无 VPN 代理权限！无 \$99 付费账号请选 TrollStore 推送（免费账号重签也会自动转巨魔）"
          if tui_select "选择 iOS 编译后真机安装策略" "$sel_install" true \
            "编译后改用 TrollStore / 巨魔模式推送安装到真机 (补齐完整 VPN 权限 · 推荐)" \
            "仅编译生成 .ipa 文件，跳过真机安装" \
            "使用付费开发者 Apple ID 重签安装 (plumesign · 需 \$99/年账号)" \
            "自动检测 (连有真机时尝试重签，若为免费账号自动转 TrollStore 推送)"; then
            sel_install=$TUI_RESULT
            case $sel_install in
              0)
                TROLLSTORE_MODE=true
                INSTALL_MODE="always"
                select_usb_device_interactive || continue
                ;;
              1) INSTALL_MODE="never" ;;
              2)
                INSTALL_MODE="always"
                select_usb_device_interactive || continue
                ;;
              3) INSTALL_MODE="auto" ;;
            esac
            step=5
          else
            step=3
          fi
        fi
        ;;

      5)
        local multi_opts=()
        local has_dmg_opt=false
        if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
          has_dmg_opt=true
          multi_opts+=("额外生成 macOS .dmg 安装镜像")
        fi
        multi_opts+=(
          "强制重编 Rust 原生模块 (libhiddify_rust / XCFramework)"
          "强制同步/重编 hiddify-core 子模块核心库"
          "强制刷新依赖与代码生成 (pub get / build_runner / pod install)"
          "跳过 Dart 代码生成 (已有生成文件时跳过 build_runner)"
          "跳过 Rust 重新编译 (已有原生库产物时直接复用)"
          "构建前执行 flutter clean 全量清理缓存"
          "自定义高级设置 (输出目录 / 重签 Bundle ID / 名称 / 描述文件)"
        )

        if tui_multiselect "构建附加开关 (空格勾选，直接回车保持默认增量构建)" "$multi_state" "${multi_opts[@]}"; then
          multi_state="${TUI_MULTI_SELECTED[*]}"
          local offset=0
          if [[ "$has_dmg_opt" == true ]]; then
            [[ "${TUI_MULTI_SELECTED[0]}" == "1" ]] && BUILD_DMG=true || BUILD_DMG=false
            offset=1
          fi
          [[ "${TUI_MULTI_SELECTED[$((offset + 0))]}" == "1" ]] && FORCE_RUST=true || FORCE_RUST=false
          [[ "${TUI_MULTI_SELECTED[$((offset + 1))]}" == "1" ]] && FORCE_LIBS=true || FORCE_LIBS=false
          [[ "${TUI_MULTI_SELECTED[$((offset + 2))]}" == "1" ]] && FORCE_DEPS=true || FORCE_DEPS=false
          [[ "${TUI_MULTI_SELECTED[$((offset + 3))]}" == "1" ]] && SKIP_CODEGEN=true || SKIP_CODEGEN=false
          [[ "${TUI_MULTI_SELECTED[$((offset + 4))]}" == "1" ]] && SKIP_RUST=true || SKIP_RUST=false
          [[ "${TUI_MULTI_SELECTED[$((offset + 5))]}" == "1" ]] && DO_CLEAN=true || DO_CLEAN=false
          local want_adv="${TUI_MULTI_SELECTED[$((offset + 6))]}"

          if [[ "$FORCE_RUST" == true && "$SKIP_RUST" == true ]]; then
            SKIP_RUST=false
          fi
          if [[ ( "$FORCE_DEPS" == true || "$DO_CLEAN" == true ) && "$SKIP_CODEGEN" == true ]]; then
            SKIP_CODEGEN=false
          fi
          if [[ "$want_adv" == "1" ]]; then
            local has_ios=false
            [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]] && has_ios=true
            prompt_advanced_settings "$has_ios"
          fi
          step=6
        else
          if [[ "$PLATFORM" == "macos" ]]; then
            step=3
          else
            step=4
          fi
        fi
        ;;

      6)
        local confirm_extra_lines=5
        printf "\n${C_CYAN}  ┌─ 待执行配置确认 ─────────────────────────────────────────${C_RESET}\n" >> "$TUI_OUT"
        if [[ "$CHECK_ONLY" == true ]]; then
          confirm_extra_lines=4
          printf "${C_CYAN}  │${C_RESET} 模式: 仅检测并自动修复构建环境 (目标: %s)\n" "$PLATFORM" >> "$TUI_OUT"
        elif [[ "$INSTALL_ONLY" == true ]]; then
          printf "${C_CYAN}  │${C_RESET} 模式: 仅签名/推送已有 IPA (%s)\n" "${IOS_IPA_OVERRIDE:-自动查找}" >> "$TUI_OUT"
          printf "${C_CYAN}  │${C_RESET} 方式: %s\n" "$([[ "$TROLLSTORE_MODE" == true ]] && echo 'TrollStore 巨魔推送' || echo 'plumesign 重签安装')" >> "$TUI_OUT"
        else
          confirm_extra_lines=6
          local arch_desc=""
          if [[ "$PLATFORM" == "macos" ]]; then
            arch_desc="macOS=${MACOS_ARCH}"
          elif [[ "$PLATFORM" == "ios" ]]; then
            arch_desc="iOS-Rust=${IOS_RUST_ARCH}"
          else
            arch_desc="macOS=${MACOS_ARCH} | iOS-Rust=${IOS_RUST_ARCH}"
          fi
          printf "${C_CYAN}  │${C_RESET} 平台: %-10s 渠道: %-8s 签名: %s%s\n" \
            "$PLATFORM" "$CHANNEL" "$SIGN_MODE" "$([[ "$TROLLSTORE_MODE" == true ]] && echo ' (iOS TrollStore)' || echo '')" >> "$TUI_OUT"
          printf "${C_CYAN}  │${C_RESET} 架构: %s\n" "$arch_desc" >> "$TUI_OUT"
          printf "${C_CYAN}  │${C_RESET} 输出: %s\n" "$OUTPUT_DIR" >> "$TUI_OUT"
        fi
        printf "${C_CYAN}  └──────────────────────────────────────────────────────────${C_RESET}\n\n" >> "$TUI_OUT"

        if tui_select "确认执行上述任务？" 0 true \
          "立即开始执行" \
          "返回从头重新选择" \
          "取消并退出"; then
          case $TUI_RESULT in
            0)
              tui_close_fds
              START_TS="$(date +%s)"
              return 0
              ;;
            1)
              printf "\n" >> "$TUI_OUT"
              step=1
              ;;
            2)
              log_warn "已取消操作并退出"
              exit 0
              ;;
          esac
        else
          # tui_select 已擦除自身与上一行空行，这里继续擦除确认框与上一级摘要行
          printf '\033[%dA\r\033[J' "$((confirm_extra_lines + 1))" >> "$TUI_OUT"
          if [[ "$CHECK_ONLY" == true ]]; then
            step=10
          elif [[ "$INSTALL_ONLY" == true ]]; then
            step=1
          else
            step=5
          fi
        fi
        ;;
    esac
  done
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

  # 禁止外置磁盘写入 AppleDouble (._*) 资源分叉文件（防止破坏 XCFramework 与 App Store ITMS-90255 校验）
  export COPYFILE_DISABLE=1

  # 预加载常见路径
  prepend_path "$HOME/.cargo/bin"
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

  local core_url core_branch
  core_url="$(git config -f "$ROOT_DIR/.gitmodules" submodule.hiddify-core.url 2>/dev/null || echo 'https://github.com/hdjfhfjejrjfjs-byte/hiddify-core.git')"
  core_branch="$(git config -f "$ROOT_DIR/.gitmodules" submodule.hiddify-core.branch 2>/dev/null || echo 'main')"
  log_info "应用版本: $APP_VERSION | Commit: $APP_COMMIT | 渠道: $CHANNEL ($BUILD_TARGET)"
  log_info "目标平台: $PLATFORM | 架构: macOS=$MACOS_ARCH / iOS-Rust=$IOS_RUST_ARCH | 签名模式: $SIGN_MODE | Flutter: >= $REQUIRED_FLUTTER"
  log_info "核心子模块: hiddify-core -> $core_url ($core_branch)"
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
  log_step "3/6 检测 Flutter SDK、CocoaPods 与 Rust 工具链"

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

ensure_go() {
  if ! command -v go >/dev/null 2>&1; then
    ensure_homebrew
    log_fix "未检测到 Go 工具链，正在通过 Homebrew 安装 go..."
    brew install go || die "Go 工具链安装失败，请手动执行 brew install go"
    hash -r
  fi
  local gopath_bin gomod_cache gomobile_ver="v0.1.12" gomobile_stamp="$ROOT_DIR/.dart_tool/.build_sh_gomobile_stamp"
  gopath_bin="$(go env GOPATH 2>/dev/null || echo "$HOME/go")/bin"
  gomod_cache="$(go env GOMODCACHE 2>/dev/null || echo "$HOME/go/pkg/mod")"
  export GOMODCACHE="$gomod_cache"
  prepend_path "$gopath_bin"
  if ! [[ -x "$gopath_bin/gomobile" && -x "$gopath_bin/gobind" ]] || ! stamp_matches "$gomobile_stamp" "$gomobile_ver"; then
    log_info "安装/对齐 gomobile 与 gobind ($gomobile_ver)..."
    go install "github.com/sagernet/gomobile/cmd/gomobile@${gomobile_ver}" \
      && go install "github.com/sagernet/gomobile/cmd/gobind@${gomobile_ver}" \
      || die "安装 gomobile/gobind (${gomobile_ver}) 失败"
    stamp_write "$gomobile_stamp" "$gomobile_ver"
  fi
  log_ok "Go 工具链: $(go version) | gomobile=${gomobile_ver}"
}

ensure_core_submodule() {
  local core_url core_branch
  core_url="$(git config -f "$ROOT_DIR/.gitmodules" submodule.hiddify-core.url 2>/dev/null || echo 'https://github.com/hdjfhfjejrjfjs-byte/hiddify-core.git')"
  core_branch="$(git config -f "$ROOT_DIR/.gitmodules" submodule.hiddify-core.branch 2>/dev/null || echo 'main')"

  local git_https_flags=(
    -c url."https://github.com/".insteadOf="git@github.com:"
    -c url."https://github.com/".insteadOf="ssh://git@github.com/"
  )

  # 若子模块尚未检出（或仅有旧残留目录、缺少 Makefile / Rust launcher），从 .gitmodules 指定的远程 Git 拉取
  if [[ ! -f "$HIDDIFY_CORE_DIR/Makefile" || ! -f "$HIDDIFY_CORE_DIR/cmd/launcher/Cargo.toml" ]]; then
    log_fix "未检测到完整的 hiddify-core 子模块源码，正在从远程 Git 拉取 ($core_url @ $core_branch)..."
    rm -rf "$HIDDIFY_CORE_DIR"
    git -C "$ROOT_DIR" submodule sync -- hiddify-core >/dev/null 2>&1 || true
    if git -C "$ROOT_DIR" "${git_https_flags[@]}" submodule update --init --remote hiddify-core; then
      git -C "$HIDDIFY_CORE_DIR" "${git_https_flags[@]}" submodule update --init --recursive \
        || die "初始化 hiddify-core 嵌套子模块失败"
    else
      rm -rf "$HIDDIFY_CORE_DIR"
      git "${git_https_flags[@]}" clone --recurse-submodules -b "$core_branch" "$core_url" "$HIDDIFY_CORE_DIR" \
        || die "无法从远程拉取 hiddify-core: $core_url"
    fi
  elif [[ "$FORCE_LIBS" == true && -e "$HIDDIFY_CORE_DIR/.git" ]]; then
    log_info "正在从远程同步最新 hiddify-core ($core_url @ $core_branch)..."
    git -C "$HIDDIFY_CORE_DIR" "${git_https_flags[@]}" fetch origin "$core_branch" \
      && git -C "$HIDDIFY_CORE_DIR" checkout -f "FETCH_HEAD" \
      && git -C "$HIDDIFY_CORE_DIR" "${git_https_flags[@]}" submodule update --init --recursive \
      || log_warn "远程拉取更新失败，继续使用当前已检出的 hiddify-core 源码"
  fi

  # 确保嵌套子模块 (hiddify-sing-box / ray2sing) 已递归检出
  if [[ ! -f "$HIDDIFY_CORE_DIR/hiddify-sing-box/go.mod" || ! -f "$HIDDIFY_CORE_DIR/ray2sing/go.mod" ]]; then
    log_fix "补齐 hiddify-core 嵌套子模块 (hiddify-sing-box / ray2sing)..."
    git -C "$HIDDIFY_CORE_DIR" "${git_https_flags[@]}" submodule update --init --recursive \
      || die "初始化 hiddify-core 嵌套子模块失败"
  fi

  local core_head
  core_head="$(git -C "$HIDDIFY_CORE_DIR" rev-parse --short=7 HEAD 2>/dev/null || echo 'unknown')"
  log_ok "hiddify-core 子模块已就绪: commit=$core_head ($core_url)"
}

ensure_rust() {
  prepend_path "$HOME/.cargo/bin"

  if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
    log_fix "未检测到 Rust 工具链 (cargo/rustc)，正在通过官方 rustup 自动安装 stable..."
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile minimal --default-toolchain stable \
      || die "Rust 工具链自动安装失败，请手动执行: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    prepend_path "$HOME/.cargo/bin"
    hash -r
  fi

  command -v cargo >/dev/null 2>&1 || die "cargo 仍不可用，请检查 ~/.cargo/bin"
  command -v rustc >/dev/null 2>&1 || die "rustc 仍不可用，请检查 ~/.cargo/bin"

  RUSTC_VER="$(rustc --version 2>/dev/null || echo unknown)"
  local cargo_ver
  cargo_ver="$(cargo --version 2>/dev/null || echo unknown)"
  log_ok "Rust 工具链: $RUSTC_VER | $cargo_ver"

  local required_targets=()
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
    if [[ "$CHECK_ONLY" == true || "$MACOS_ARCH" == "universal" ]]; then
      required_targets+=(aarch64-apple-darwin x86_64-apple-darwin)
    elif [[ "$MACOS_ARCH" == "x86_64" ]]; then
      required_targets+=(x86_64-apple-darwin)
    else
      required_targets+=(aarch64-apple-darwin)
    fi
  fi
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]]; then
    if [[ "$CHECK_ONLY" == true || "$IOS_RUST_ARCH" != "arm64" ]]; then
      required_targets+=(aarch64-apple-ios aarch64-apple-ios-sim)
    else
      required_targets+=(aarch64-apple-ios)
    fi
  fi

  if command -v rustup >/dev/null 2>&1 && [[ ${#required_targets[@]} -gt 0 ]]; then
    local installed_targets missing_targets=() t
    installed_targets="$(rustup target list --installed 2>/dev/null || true)"
    for t in "${required_targets[@]}"; do
      if ! printf '%s\n' "$installed_targets" | grep -qx "$t"; then
        missing_targets+=("$t")
      fi
    done
    if [[ ${#missing_targets[@]} -gt 0 ]]; then
      log_fix "自动安装缺失的 Rust 交叉编译目标: ${missing_targets[*]}"
      rustup target add "${missing_targets[@]}" || die "安装 Rust 目标失败: ${missing_targets[*]}"
    fi
    log_ok "Rust 目标架构已就绪: ${required_targets[*]}"
  elif [[ ${#required_targets[@]} -gt 0 ]]; then
    log_warn "未检测到 rustup（使用的是独立安装的 rustc），跨架构编译若报错请安装 rustup"
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
    log_fix "已勾选强制刷新依赖：重新执行 flutter pub get"
    run_pub_get
  elif pub_get_needed; then
    run_pub_get
  else
    log_skip "Flutter 依赖已是最新（pubspec 未变化），跳过 flutter pub get"
  fi

  local need_codegen=true
  local codegen_hash=""
  if [[ "$SKIP_CODEGEN" == true ]]; then
    if codegen_outputs_exist; then
      need_codegen=false
      log_skip "已勾选跳过代码生成且生成文件存在，跳过 build_runner/slang"
    else
      log_warn "勾选了跳过代码生成，但未检测到生成文件，自动执行代码生成..."
    fi
  elif [[ "$FORCE_DEPS" != true ]] && codegen_outputs_exist; then
    codegen_hash="$(codegen_input_hash)"
    if stamp_matches "$ROOT_DIR/.dart_tool/.build_sh_codegen_stamp" "$codegen_hash"; then
      need_codegen=false
      log_skip "Dart 代码生成结果已是最新，跳过 build_runner/slang"
    fi
  fi

  if [[ "$need_codegen" == true ]]; then
    [[ -n "$codegen_hash" ]] || codegen_hash="$(codegen_input_hash)"
    run_codegen "$codegen_hash"
  fi
}

hiddify_core_input_hash() {
  [[ -d "$HIDDIFY_CORE_DIR" ]] || die "未找到本地 hiddify-core 源码目录: $HIDDIFY_CORE_DIR"
  local head_rev status_hash diff_hash
  head_rev="$(git -C "$HIDDIFY_CORE_DIR" rev-parse HEAD 2>/dev/null || echo nogit)"
  status_hash="$(git -C "$HIDDIFY_CORE_DIR" status --porcelain --untracked-files=all 2>/dev/null | shasum -a 256 | awk '{print $1}')"
  diff_hash="$(
    {
      git -C "$HIDDIFY_CORE_DIR" diff 2>/dev/null || true
      git -C "$HIDDIFY_CORE_DIR" ls-files --others --exclude-standard 2>/dev/null | LC_ALL=C sort | while IFS= read -r rel; do
        [[ -f "$HIDDIFY_CORE_DIR/$rel" ]] && shasum -a 256 "$HIDDIFY_CORE_DIR/$rel"
      done
    } | shasum -a 256 | awk '{print $1}'
  )"
  printf 'dir=%s;head=%s;st=%s;diff=%s' "$HIDDIFY_CORE_DIR" "$head_rev" "$status_hash" "$diff_hash"
}

macos_core_dylib_ready() {
  local dylib="$HIDDIFY_CORE_DIR/bin/hiddify-core.dylib"
  [[ -f "$dylib" ]] || return 1
  local archs
  archs="$(lipo -archs "$dylib" 2>/dev/null || true)"
  case "$MACOS_ARCH" in
    arm64)  [[ "$archs" == "arm64" ]] ;;
    x86_64) [[ "$archs" == "x86_64" ]] ;;
    *)      [[ "$archs" == *arm64* && "$archs" == *x86_64* ]] ;;
  esac
}

prepare_macos_core_libs() {
  [[ -d "$HIDDIFY_CORE_DIR" && -f "$HIDDIFY_CORE_DIR/Makefile" ]] \
    || die "未找到本地 hiddify-core 源码目录或 Makefile: $HIDDIFY_CORE_DIR"
  [[ -f "$HIDDIFY_CORE_DIR/cmd/launcher/Cargo.toml" ]] \
    || die "本地 hiddify-core 缺少 Rust launcher (cmd/launcher/Cargo.toml): $HIDDIFY_CORE_DIR"

  local stamp="$ROOT_DIR/.dart_tool/.build_sh_core_macos_stamp"
  local tunnel_stamp="$ROOT_DIR/.dart_tool/.build_sh_core_macos_tunnel_stamp"
  local core_hash want
  core_hash="$(hiddify_core_input_hash)"
  want="${core_hash};arch=${MACOS_ARCH}"

  local need_dylib=false
  local need_tunnel=false
  if [[ "$FORCE_LIBS" == true ]]; then
    need_dylib=true
    need_tunnel=true
  else
    if ! macos_core_dylib_ready || ! stamp_matches "$stamp" "$want"; then
      need_dylib=true
    fi
    if [[ ! -d "$ROOT_DIR/macos/Frameworks/HiddifyTunnelCore.xcframework" ]] \
      || ! stamp_matches "$tunnel_stamp" "$core_hash"; then
      need_tunnel=true
    fi
  fi

  if [[ "$need_dylib" == false && "$need_tunnel" == false ]]; then
    log_skip "本地 macOS 核心库已是最新 ($HIDDIFY_CORE_DIR, arch=$MACOS_ARCH)，跳过编译"
    return 0
  fi

  mkdir -p "$HIDDIFY_CORE_DIR/bin" "$ROOT_DIR/macos/Frameworks"

  if [[ "$need_dylib" == true ]]; then
    log_info "从本地源码编译 macOS hiddify-core.dylib ($HIDDIFY_CORE_DIR, arch=$MACOS_ARCH)..."
    case "$MACOS_ARCH" in
      arm64)
        make -C "$HIDDIFY_CORE_DIR" -f Makefile macos-arm64 \
          || die "编译本地 macOS arm64 hiddify-core.dylib 失败"
        cp -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-arm64.dylib" "$HIDDIFY_CORE_DIR/bin/hiddify-core.dylib"
        [[ -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-arm64.h" ]] \
          && cp -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-arm64.h" "$HIDDIFY_CORE_DIR/bin/desktop.h"
        ;;
      x86_64)
        make -C "$HIDDIFY_CORE_DIR" -f Makefile macos-amd64 \
          || die "编译本地 macOS x86_64 hiddify-core.dylib 失败"
        cp -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-amd64.dylib" "$HIDDIFY_CORE_DIR/bin/hiddify-core.dylib"
        [[ -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-amd64.h" ]] \
          && cp -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-amd64.h" "$HIDDIFY_CORE_DIR/bin/desktop.h"
        ;;
      *)
        make -C "$HIDDIFY_CORE_DIR" -f Makefile macos-amd64 macos-arm64 \
          || die "编译本地 macOS Universal2 hiddify-core.dylib 失败"
        lipo -create \
          "$HIDDIFY_CORE_DIR/bin/hiddify-core-amd64.dylib" \
          "$HIDDIFY_CORE_DIR/bin/hiddify-core-arm64.dylib" \
          -output "$HIDDIFY_CORE_DIR/bin/hiddify-core.dylib"
        [[ -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-arm64.h" ]] \
          && cp -f "$HIDDIFY_CORE_DIR/bin/hiddify-core-arm64.h" "$HIDDIFY_CORE_DIR/bin/desktop.h"
        ;;
    esac
    cp -f "$HIDDIFY_CORE_DIR/bin/hiddify-core.dylib" "$HIDDIFY_CORE_DIR/hiddify-core.dylib"
    macos_core_dylib_ready || die "生成的 $HIDDIFY_CORE_DIR/bin/hiddify-core.dylib 架构与目标 $MACOS_ARCH 不匹配"
    stamp_write "$stamp" "$want"
    log_ok "本地 macOS hiddify-core.dylib 编译完成 ($(lipo -archs "$HIDDIFY_CORE_DIR/bin/hiddify-core.dylib"), $(du -sh "$HIDDIFY_CORE_DIR/bin/hiddify-core.dylib" | awk '{print $1}'))"
  fi

  if [[ "$need_tunnel" == true ]]; then
    log_info "编译 macOS HiddifyTunnelCore.xcframework (hiddify-core 子模块)..."
    rm -rf "$HIDDIFY_CORE_DIR/build" "$HIDDIFY_CORE_DIR/bin/HiddifyTunnelCore.xcframework"
    make -C "$HIDDIFY_CORE_DIR" -f Makefile macos-tunnel \
      || die "编译 macOS HiddifyTunnelCore.xcframework 失败"
    [[ -d "$HIDDIFY_CORE_DIR/bin/HiddifyTunnelCore.xcframework" ]] \
      || die "缺少编译产物: $HIDDIFY_CORE_DIR/bin/HiddifyTunnelCore.xcframework"
    rm -rf "$ROOT_DIR/macos/Frameworks/HiddifyTunnelCore.xcframework"
    ditto "$HIDDIFY_CORE_DIR/bin/HiddifyTunnelCore.xcframework" "$ROOT_DIR/macos/Frameworks/HiddifyTunnelCore.xcframework"
    stamp_write "$tunnel_stamp" "$core_hash"
  fi

  log_ok "macOS 核心库与 Rust launcher 源码就绪 ($HIDDIFY_CORE_DIR)"
}

prepare_ios_core_libs() {
  [[ -d "$HIDDIFY_CORE_DIR" && -f "$HIDDIFY_CORE_DIR/Makefile" ]] \
    || die "未找到 hiddify-core 子模块源码目录或 Makefile: $HIDDIFY_CORE_DIR"

  local stamp="$ROOT_DIR/.dart_tool/.build_sh_core_ios_stamp"
  local want
  want="$(hiddify_core_input_hash)"

  if [[ "$FORCE_LIBS" != true ]] \
    && [[ -d "$ROOT_DIR/ios/Frameworks/HiddifyCore.xcframework" ]] \
    && stamp_matches "$stamp" "$want"; then
    log_skip "iOS 核心库已是最新 (hiddify-core 子模块)，跳过编译"
    return 0
  fi

  log_info "编译 iOS HiddifyCore.xcframework (hiddify-core 子模块)..."
  mkdir -p "$HIDDIFY_CORE_DIR/bin" "$ROOT_DIR/ios/Frameworks"
  rm -rf "$HIDDIFY_CORE_DIR/build" "$HIDDIFY_CORE_DIR/bin/HiddifyCore.xcframework"
  make -C "$HIDDIFY_CORE_DIR" -f Makefile ios \
    || die "编译 iOS HiddifyCore.xcframework 失败"
  [[ -d "$HIDDIFY_CORE_DIR/bin/HiddifyCore.xcframework" ]] \
    || die "缺少编译产物: $HIDDIFY_CORE_DIR/bin/HiddifyCore.xcframework"
  rm -rf "$ROOT_DIR/ios/Frameworks/HiddifyCore.xcframework"
  ditto "$HIDDIFY_CORE_DIR/bin/HiddifyCore.xcframework" "$ROOT_DIR/ios/Frameworks/HiddifyCore.xcframework"
  stamp_write "$stamp" "$want"
  log_ok "iOS 核心库编译就绪: $ROOT_DIR/ios/Frameworks/HiddifyCore.xcframework"
}

rust_input_hash() {
  local files_hash
  files_hash="$(
    {
      find "$ROOT_DIR/rust/crates" -type f \( -name '*.rs' -o -name 'Cargo.toml' \) -print
      printf '%s\n' \
        "$ROOT_DIR/rust/Cargo.toml" \
        "$ROOT_DIR/rust/include/hiddify_rust.h" \
        "$ROOT_DIR/scripts/build_rust.sh"
      if [[ -f "$ROOT_DIR/rust/Cargo.lock" ]]; then
        printf '%s\n' "$ROOT_DIR/rust/Cargo.lock"
      fi
    } | LC_ALL=C sort | tr '\n' '\0' | xargs -0 shasum -a 256 | shasum -a 256 | awk '{print $1}'
  )"
  printf 'rustc=%s;%s' "${RUSTC_VER:-unknown}" "$files_hash"
}

macos_rust_artifacts_ready() {
  local dylib="$ROOT_DIR/macos/Frameworks/libhiddify_rust.dylib"
  [[ -f "$dylib" ]] || return 1
  local archs
  archs="$(lipo -archs "$dylib" 2>/dev/null || true)"
  case "$MACOS_ARCH" in
    arm64)
      [[ "$archs" == "arm64" ]]
      ;;
    x86_64)
      [[ "$archs" == "x86_64" ]]
      ;;
    *)
      [[ "$archs" == *arm64* && "$archs" == *x86_64* ]]
      ;;
  esac
}

ios_rust_artifacts_ready() {
  local xcf="$ROOT_DIR/ios/Frameworks/HiddifyRust.xcframework"
  [[ -f "$xcf/Info.plist" ]] || return 1
  [[ -f "$xcf/ios-arm64/HiddifyRust.framework/HiddifyRust" ]] || return 1
  if [[ "$IOS_RUST_ARCH" == "arm64" ]]; then
    [[ -z "$(find "$xcf" -maxdepth 2 -name 'HiddifyRust.framework' -path '*simulator*' -print -quit 2>/dev/null)" ]]
  else
    [[ -n "$(find "$xcf" -maxdepth 2 -name 'HiddifyRust.framework' -path '*simulator*' -print -quit 2>/dev/null)" ]]
  fi
}

prepare_macos_rust_libs() {
  local stamp="$ROOT_DIR/.dart_tool/.build_sh_rust_macos_stamp"
  local dylib="$ROOT_DIR/macos/Frameworks/libhiddify_rust.dylib"
  local want="${1};arch=${MACOS_ARCH}"

  if [[ "$SKIP_RUST" == true ]] && macos_rust_artifacts_ready; then
    log_skip "已勾选跳过 Rust 编译且检测到匹配的 macOS libhiddify_rust.dylib ($MACOS_ARCH)，直接复用"
    return 0
  fi

  if [[ "$FORCE_LIBS" != true && "$FORCE_RUST" != true ]] \
    && macos_rust_artifacts_ready \
    && stamp_matches "$stamp" "$want"; then
    log_skip "macOS Rust 原生库已是最新 ($MACOS_ARCH)，跳过编译"
    return 0
  fi

  log_info "编译 macOS Rust 原生库 (libhiddify_rust.dylib, arch=$MACOS_ARCH)..."
  bash "$ROOT_DIR/scripts/build_rust.sh" macos "$MACOS_ARCH" || die "macOS Rust 原生库编译失败"
  macos_rust_artifacts_ready || die "生成的 libhiddify_rust.dylib 架构与目标 ${MACOS_ARCH} 不匹配"
  stamp_write "$stamp" "$want"
  log_ok "macOS Rust 原生库编译就绪: $dylib ($(lipo -archs "$dylib"))"
}

prepare_ios_rust_libs() {
  local stamp="$ROOT_DIR/.dart_tool/.build_sh_rust_ios_stamp"
  local xcf="$ROOT_DIR/ios/Frameworks/HiddifyRust.xcframework"
  local want="${1};arch=${IOS_RUST_ARCH}"

  if [[ "$SKIP_RUST" == true ]] && ios_rust_artifacts_ready; then
    log_skip "已勾选跳过 Rust 编译且检测到匹配的 HiddifyRust.xcframework ($IOS_RUST_ARCH)，直接复用"
    return 0
  fi

  if [[ "$FORCE_LIBS" != true && "$FORCE_RUST" != true ]] \
    && ios_rust_artifacts_ready \
    && stamp_matches "$stamp" "$want"; then
    log_skip "iOS Rust 原生框架已是最新 (HiddifyRust.xcframework, arch=$IOS_RUST_ARCH)，跳过编译"
    return 0
  fi

  log_info "编译 iOS Rust 原生动态框架 (HiddifyRust.xcframework, arch=$IOS_RUST_ARCH)..."
  bash "$ROOT_DIR/scripts/build_rust.sh" ios "$IOS_RUST_ARCH" || die "iOS Rust 原生框架编译失败"
  ios_rust_artifacts_ready || die "生成的 HiddifyRust.xcframework 结构与目标 ${IOS_RUST_ARCH} 不匹配"
  stamp_write "$stamp" "$want"
  log_ok "iOS Rust 原生框架编译就绪: $xcf ($IOS_RUST_ARCH)"
}

prepare_core_libs() {
  log_step "5/6 检测与准备 hiddify-core 与 Rust 原生库"
  local rust_hash
  rust_hash="$(rust_input_hash)"
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "macos" ]]; then
    prepare_macos_core_libs
    prepare_macos_rust_libs "$rust_hash"
  fi
  if [[ "$PLATFORM" == "all" || "$PLATFORM" == "ios" ]]; then
    prepare_ios_core_libs
    prepare_ios_rust_libs "$rust_hash"
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
    log_skip "$platform_dir Pods 已是最新（Podfile.lock 与 Manifest.lock 一致），跳过 pod install"
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

resign_macos_target() {
  local target="$1"
  local sign_id="$2"
  local use_runtime="$3"
  local flags=(--force --sign "$sign_id")
  if [[ "$use_runtime" == true ]]; then
    flags+=(--options runtime --timestamp)
  else
    flags+=(--timestamp=none)
  fi
  if ! codesign "${flags[@]}" --preserve-metadata=entitlements,requirements,flags,runtime "$target" >/dev/null 2>&1; then
    codesign "${flags[@]}" "$target" >/dev/null 2>&1 || die "重签 macOS 组件失败: $target"
  fi
}

thin_macos_app_bundle() {
  local app="$1"
  local target_arch="$2"
  local effective_sign="$3"
  [[ "$target_arch" == "arm64" || "$target_arch" == "x86_64" ]] || return 0

  local size_before core_before
  size_before="$(du -sh "$app" | awk '{print $1}')"
  core_before="$(du -sh "$app/Contents/Frameworks/hiddify-core.dylib" 2>/dev/null | awk '{print $1}' || echo 'N/A')"

  # 裁剪前先读取现有签名身份与 Hardened Runtime 状态
  local sign_id="-"
  local use_runtime=false
  if [[ "$effective_sign" != "unsigned" ]]; then
    local cs_info detected_auth
    cs_info="$(codesign -dvv "$app" 2>&1 || true)"
    detected_auth="$(printf '%s\n' "$cs_info" | sed -n 's/^Authority=//p' | head -n 1)"
    if [[ -n "$detected_auth" ]]; then
      sign_id="$detected_auth"
    fi
    if printf '%s\n' "$cs_info" | grep -q '(runtime)'; then
      use_runtime=true
    fi
  fi

  log_info "正在将 $app 裁剪为单架构 ($target_arch)..."
  local thinned_count=0
  local bin archs
  while IFS= read -r -d '' bin; do
    archs="$(lipo -archs "$bin" 2>/dev/null || true)"
    # 仅处理含多架构（包含空格）且包含目标架构的胖二进制
    if [[ "$archs" == *" "* && " $archs " == *" $target_arch "* ]]; then
      lipo -thin "$target_arch" "$bin" -output "${bin}.thin" || die "lipo 裁剪失败: $bin"
      chmod --reference="$bin" "${bin}.thin" 2>/dev/null || chmod 0755 "${bin}.thin"
      mv -f "${bin}.thin" "$bin"
      thinned_count=$((thinned_count + 1))
    fi
  done < <(find "$app" -type f \( -perm +111 -o -name '*.dylib' \) -print0)

  # 由内向外重新签名以保证单架构后的代码签名完整有效
  local item
  if [[ -d "$app/Contents/Frameworks" ]]; then
    for item in "$app/Contents/Frameworks"/*.dylib; do
      [[ -f "$item" ]] && resign_macos_target "$item" "$sign_id" "$use_runtime"
    done
    for item in "$app/Contents/Frameworks"/*.framework; do
      [[ -d "$item" ]] && resign_macos_target "$item" "$sign_id" "$use_runtime"
    done
  fi
  for item in \
    "$app/Contents/Helpers"/*.app \
    "$app/Contents/Library/LoginItems"/*.app \
    "$app/Contents/Library/SystemExtensions"/*.systemextension \
    "$app/Contents/PlugIns"/*.appex; do
    [[ -d "$item" ]] && resign_macos_target "$item" "$sign_id" "$use_runtime"
  done
  resign_macos_target "$app" "$sign_id" "$use_runtime"

  codesign --verify --deep --strict "$app" >/dev/null 2>&1 || die "单架构裁剪后代码签名校验失败: $app"

  local size_after core_after
  size_after="$(du -sh "$app" | awk '{print $1}')"
  core_after="$(du -sh "$app/Contents/Frameworks/hiddify-core.dylib" 2>/dev/null | awk '{print $1}' || echo 'N/A')"
  log_ok "单架构 ($target_arch) 裁剪并重签完成: 共处理 ${thinned_count} 个二进制 | App ${size_before} -> ${size_after} | hiddify-core.dylib ${core_before} -> ${core_after}"
}

build_macos() {
  log_step "6/6 [macOS] 开始编译 macOS 客户端 (.app)"
  cd "$ROOT_DIR"

  local effective_sign
  effective_sign="$(resolve_macos_sign_mode)"
  log_info "macOS 签名方式: $effective_sign | 目标架构: $MACOS_ARCH"

  local extra_defines=()
  local saved_xcconfig="${XCODE_XCCONFIG_FILE:-}"

  case "$effective_sign" in
    unsigned)
      export XCODE_XCCONFIG_FILE="$ROOT_DIR/macos/Runner/Configs/Unsigned.xcconfig"
      ;;
    developer-id)
      unset XCODE_XCCONFIG_FILE || true
      if ! security find-identity -v -p codesigning 2>/dev/null | grep -q "Developer ID Application"; then
        die "未在本机钥匙串检测到 Developer ID Application 证书，请在菜单中选择无签名构建或先导入证书"
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
  if [[ ! -f "$built_app/Contents/Frameworks/libhiddify_rust.dylib" ]]; then
    if [[ "$effective_sign" == "unsigned" && -f "$ROOT_DIR/macos/Frameworks/libhiddify_rust.dylib" ]]; then
      log_fix "补齐嵌入 libhiddify_rust.dylib 到 Hiddify.app/Contents/Frameworks..."
      mkdir -p "$built_app/Contents/Frameworks"
      cp -f "$ROOT_DIR/macos/Frameworks/libhiddify_rust.dylib" "$built_app/Contents/Frameworks/libhiddify_rust.dylib"
      codesign --force --sign - --timestamp=none "$built_app/Contents/Frameworks/libhiddify_rust.dylib" >/dev/null 2>&1 || true
    else
      die "macOS 产物缺少嵌入的 libhiddify_rust.dylib: $built_app/Contents/Frameworks/libhiddify_rust.dylib"
    fi
  fi
  find "$built_app" -name '._*' -delete 2>/dev/null || true

  mkdir -p "$OUTPUT_DIR"
  local out_app="$OUTPUT_DIR/Hiddify.app"
  rm -rf "$out_app"
  ditto "$built_app" "$out_app"

  if [[ "$MACOS_ARCH" != "universal" ]]; then
    thin_macos_app_bundle "$out_app" "$MACOS_ARCH" "$effective_sign"
  fi
  touch "$out_app"
  record_built_artifact "$out_app"

  log_ok "macOS .app 编译成功: $out_app ($MACOS_ARCH)"

  if [[ "$BUILD_DMG" == true ]]; then
    local dmg_suffix=""
    [[ "$MACOS_ARCH" != "universal" ]] && dmg_suffix="-${MACOS_ARCH}"
    local out_dmg="$OUTPUT_DIR/Hiddify-macOS-${APP_VERSION}${dmg_suffix}.dmg"
    local stage_dir
    stage_dir="$(mktemp -d)"
    ditto "$out_app" "$stage_dir/Hiddify.app"
    ln -s /Applications "$stage_dir/Applications"
    hdiutil create -volname "Hiddify" -srcfolder "$stage_dir" -format UDZO -ov "$out_dmg" >/dev/null
    rm -rf "$stage_dir"
    record_built_artifact "$out_dmg"
    log_ok "macOS .dmg 打包成功: $out_dmg"
  fi
}

# ==============================================================================
# 阶段 5：构建 iOS (.ipa) 与真机签名安装 (plumesign)
# ==============================================================================
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
  [[ -f "$app/Frameworks/HiddifyRust.framework/HiddifyRust" ]] \
    || die "iOS .app 缺少嵌入的 Rust 动态框架: $app/Frameworks/HiddifyRust.framework/HiddifyRust"

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

ensure_ldid() {
  if command -v ldid >/dev/null 2>&1; then
    return 0
  fi
  ensure_homebrew
  log_fix "未检测到 ldid，正在通过 Homebrew 自动安装 ldid（TrollStore 伪签 VPN 权限所需）..."
  brew install ldid || die "ldid 自动安装失败，请手动执行 brew install ldid"
  hash -r
  command -v ldid >/dev/null 2>&1 || die "ldid 仍不可用，请检查 Homebrew 环境"
}

ensure_afcclient() {
  if command -v afcclient >/dev/null 2>&1; then
    return 0
  fi
  ensure_homebrew
  log_fix "未检测到 afcclient，正在通过 Homebrew 自动安装 libimobiledevice（TrollStore USB 传输所需）..."
  brew install libimobiledevice || die "libimobiledevice 自动安装失败，请手动执行 brew install libimobiledevice"
  hash -r
  command -v afcclient >/dev/null 2>&1 || die "afcclient 仍不可用，请检查 libimobiledevice 安装"
}

render_ios_entitlements() {
  local src_plist="$1"
  local dst_plist="$2"
  local base_bundle_id="$3"
  local bundle_id="$4"
  local team_id="$5"

  [[ -f "$src_plist" ]] || die "未找到 entitlements 文件: $src_plist"
  python3 - "$src_plist" "$dst_plist" "$base_bundle_id" "$bundle_id" "$team_id" <<'PY'
import plistlib, sys

src, dst, base_id, bundle_id, team_id = sys.argv[1:6]
raw = open(src, "rb").read().decode("utf-8")
raw = raw.replace("$(BASE_BUNDLE_IDENTIFIER)", base_id).replace("${BASE_BUNDLE_IDENTIFIER}", base_id)
data = plistlib.loads(raw.encode("utf-8"))

if team_id and bundle_id:
    data["application-identifier"] = f"{team_id}.{bundle_id}"
    data["com.apple.developer.team-identifier"] = team_id
    data["keychain-access-groups"] = [f"{team_id}.{base_id}"]

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
  local base_bundle_id team_id
  base_bundle_id="$(read_ios_base_bundle_id "$app/Info.plist")"
  team_id="$(awk -F= '/^[[:space:]]*DEVELOPMENT_TEAM[[:space:]]*=/ {gsub(/[[:space:]]/, "", $2); print $2; exit}' "$ROOT_DIR/ios/Base.xcconfig" 2>/dev/null || true)"
  team_id="${team_id:-M7Q8ASP66Z}"

  if [[ "${TROLLSTORE_MODE:-false}" == true ]]; then
    ensure_ldid
  fi

  local ent_dir
  ent_dir="$(mktemp -d)"

  local runner_ent="$ent_dir/Runner.entitlements"
  render_ios_entitlements "$ROOT_DIR/ios/Runner/Runner.entitlements" "$runner_ent" "$base_bundle_id" "$base_bundle_id" "$team_id"

  local tunnel_ent="$ent_dir/HiddifyPacketTunnel.entitlements"
  if [[ -f "$ROOT_DIR/ios/HiddifyPacketTunnel/HiddifyPacketTunnel.entitlements" ]]; then
    render_ios_entitlements "$ROOT_DIR/ios/HiddifyPacketTunnel/HiddifyPacketTunnel.entitlements" "$tunnel_ent" "$base_bundle_id" "${base_bundle_id}.HiddifyPacketTunnel" "$team_id"
  fi

  if [[ "${TROLLSTORE_MODE:-false}" == true ]]; then
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
  embed_ios_ad_hoc_entitlements "$staged_app"
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
  devices="$(list_usb_ios_devices)" || die "未检测到通过 USB 连接的 iOS 设备（请连接数据线，或在菜单中选择跳过真机安装）"

  RESOLVED_DEVICE_ID=""
  RESOLVED_DEVICE_UDID=""

  if [[ -z "$DEVICE_ID" ]]; then
    local first_line
    first_line="${devices%%$'\n'*}"
    RESOLVED_DEVICE_UDID="${first_line%%$'\t'*}"
    RESOLVED_DEVICE_ID="${first_line#*$'\t'}"
    [[ -n "$RESOLVED_DEVICE_ID" ]] || die "无法从 usbmuxd 响应解析 DeviceID"
    log_info "自动选用首个 USB 设备: UDID=$RESOLVED_DEVICE_UDID (DeviceID=$RESOLVED_DEVICE_ID)"
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

repackage_ipa_for_trollstore() {
  local src_ipa="$1"
  local dst_ipa="$2"
  ensure_ldid

  local tmp_stage
  tmp_stage="$(mktemp -d)"
  unzip -q "$src_ipa" -d "$tmp_stage" || { rm -rf "$tmp_stage"; die "无法解压 IPA: $src_ipa"; }
  local app_bundle
  app_bundle="$(find "$tmp_stage/Payload" -maxdepth 1 -name '*.app' -type d | head -n 1)"
  [[ -n "$app_bundle" ]] || { rm -rf "$tmp_stage"; die "IPA 结构异常，缺少 Payload/*.app: $src_ipa"; }
  [[ -d "$app_bundle/PlugIns/HiddifyPacketTunnel.appex" ]] \
    || { rm -rf "$tmp_stage"; die "该 IPA 缺少 PlugIns/HiddifyPacketTunnel.appex，代理软件缺少 VPN 扩展，请重新完整编译 IPA"; }

  TROLLSTORE_MODE=true embed_ios_ad_hoc_entitlements "$app_bundle"
  TROLLSTORE_MODE=true verify_trollstore_app "$app_bundle"

  mkdir -p "$(dirname "$dst_ipa")"
  rm -f "$dst_ipa"
  (
    cd "$tmp_stage"
    zip -qry "$dst_ipa" Payload
  )
  rm -rf "$tmp_stage"
  [[ -s "$dst_ipa" ]] || die "生成 TrollStore IPA 失败: $dst_ipa"
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
  ensure_ldid
  ensure_afcclient

  local push_ipa="$ipa_path"
  local ts_stage_dir=""
  log_info "安装前校验 Runner 与 HiddifyPacketTunnel.appex 的 VPN / App Group 权限..."
  if ! (
    set +e
    TROLLSTORE_MODE=true
    local check_stage check_app
    check_stage="$(mktemp -d)"
    unzip -q "$ipa_path" -d "$check_stage" 2>/dev/null || { rm -rf "$check_stage"; exit 1; }
    check_app="$(find "$check_stage/Payload" -maxdepth 1 -name '*.app' -type d | head -n 1)"
    [[ -n "$check_app" && -d "$check_app/PlugIns/HiddifyPacketTunnel.appex" ]] || { rm -rf "$check_stage"; exit 1; }
    local main_dump ext_dump
    main_dump="$(ldid -e "$check_app/Runner" 2>/dev/null || true)"
    ext_dump="$(ldid -e "$check_app/PlugIns/HiddifyPacketTunnel.appex/HiddifyPacketTunnel" 2>/dev/null || true)"
    rm -rf "$check_stage"
    [[ "$main_dump" == *com.apple.developer.networking.networkextension* && "$main_dump" == *allow-vpn* && "$ext_dump" == *packet-tunnel-provider* ]]
  ); then
    log_fix "检测到当前 IPA 尚未写入 ldid 巨魔伪签权限，正在自动补齐完整 VPN + App Group 权限..."
    ts_stage_dir="$(mktemp -d)"
    push_ipa="$ts_stage_dir/Hiddify-TrollStore.ipa"
    repackage_ipa_for_trollstore "$ipa_path" "$push_ipa"
  fi
  TROLLSTORE_MODE=true verify_trollstore_ipa "$push_ipa"

  log_step "通过 TrollStore / TrollStore Lite 模式推送 IPA 到越狱/巨魔真机"
  resolve_ios_device
  check_ios_device_os

  local remote_rel="/Downloads/Hiddify.ipa"
  local remote_abs="/var/mobile/Media/Downloads/Hiddify.ipa"
  local afc_args=()
  [[ -n "$RESOLVED_DEVICE_UDID" ]] && afc_args+=(-u "$RESOLVED_DEVICE_UDID")

  afcclient "${afc_args[@]}" mkdir /Downloads >/dev/null 2>&1 || true
  afcclient "${afc_args[@]}" rm "$remote_rel" >/dev/null 2>&1 || true
  log_info "正在通过 USB AFC 传输 $push_ipa -> 真机 $remote_abs ..."
  afcclient "${afc_args[@]}" put "$push_ipa" "$remote_rel"
  [[ -n "$ts_stage_dir" ]] && rm -rf "$ts_stage_dir"
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
  [[ -f "$ipa_path" ]] || die "未找到待安装的 IPA 文件: $ipa_path（请先编译或在菜单中指定有效路径）"

  if [[ "$TROLLSTORE_MODE" == true ]]; then
    install_ios_ipa_trollstore "$ipa_path"
    return 0
  fi

  local plumesign=""
  if ! plumesign="$(resolve_plumesign_bin)"; then
    if [[ -n "$PLUMESIGN_BIN" ]]; then
      die "指定的 plumesign 不可执行: $PLUMESIGN_BIN"
    fi
    die "未找到 plumesign（请安装 Impactor.app 或在菜单高级设置中指定路径）"
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
    log_info "未指定描述文件：将由 plumesign 通过 Apple ID 自动注册 App ID / App Group 并签发描述文件"
  fi

  local sign_log="$sign_dir/plumesign.log"
  log_info "正在签名并安装到设备 DeviceID=$RESOLVED_DEVICE_ID (${RESOLVED_DEVICE_UDID:-unknown})..."
  if RUST_LOG="${RUST_LOG:-info}" "$plumesign" "${args[@]}" 2>&1 | tee "$sign_log"; then
    rm -rf "$sign_dir"
    log_ok "已安装 Hiddify ($APP_VERSION) 到设备 ${RESOLVED_DEVICE_UDID:-DeviceID $RESOLVED_DEVICE_ID}"
    return 0
  fi

  # 免费个人 Apple ID 不支持 NetworkExtension / PacketTunnel (VPN) 权限：
  # 代理软件绝不能剥离 VPN 扩展（否则装上也无法启动代理），必须借助 TrollStore 安装！
  if [[ -z "$PROVISION_PATH" ]] && grep -Eiq "capabilit|entitlement|networkextension|vpn|app.?id|0xe8008016|maxQuantity|limit|addApplicationGroup|not available" "$sign_log"; then
    rm -rf "$sign_dir"
    log_warn "检测到当前 Apple ID 为免费个人账户（无 NetworkExtension / VPN 扩展权限）："
    log_warn "代理软件剥离 VPN 权限后无法工作，免费账户真机安装必须借助 TrollStore！"
    log_fix "正在自动切换至 TrollStore / 巨魔模式（用 ldid 伪签完整 VPN + App Group 权限并推送真机）..."
    TROLLSTORE_MODE=true
    install_ios_ipa_trollstore "$ipa_path"
    return 0
  fi

  rm -rf "$sign_dir"
  die "plumesign 签名或真机安装失败（若使用的是免费 Apple ID，请重新运行 ./build.sh 并选择 TrollStore / 巨魔模式）"
}

maybe_install_ios_ipa() {
  local ipa_path="$1"
  case "$INSTALL_MODE" in
    never)
      log_skip "已选择跳过真机签名与安装"
      return 0
      ;;
    always)
      if [[ "$INSTALL_ONLY" != true ]] && ! list_usb_ios_devices >/dev/null 2>&1; then
        log_warn "当前未检测到通过 USB 连接的 iOS 真机，已保留编译好的 IPA: $ipa_path"
        log_info "连接数据线后可直接运行 ./build.sh 选择「仅安装已有 iOS .ipa 到 USB 真机」完成巨魔推送安装"
        return 0
      fi
      install_ios_ipa "$ipa_path"
      ;;
    auto)
      if ! list_usb_ios_devices >/dev/null 2>&1; then
        log_skip "未检测到 USB 连接的 iOS 设备，跳过自动安装（连接真机后可运行 ./build.sh 选择仅安装已有 IPA）"
        return 0
      fi
      if [[ "$TROLLSTORE_MODE" == true ]]; then
        install_ios_ipa_trollstore "$ipa_path"
        return 0
      fi
      if ! resolve_plumesign_bin >/dev/null 2>&1; then
        log_warn "检测到 USB iOS 设备但未找到 plumesign；无付费开发者证书运行代理必须借助 TrollStore，自动改走 TrollStore 推送..."
        TROLLSTORE_MODE=true
        install_ios_ipa_trollstore "$ipa_path"
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
    ensure_ldid
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
    record_built_artifact "$BUILT_IOS_IPA"
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
    record_built_artifact "$BUILT_IOS_IPA"
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

  local min_ts=$((START_TS - 2))
  local valid_artifacts=()
  local item item_mtime size
  if [[ ${#BUILT_ARTIFACTS[@]} -gt 0 ]]; then
    for item in "${BUILT_ARTIFACTS[@]}"; do
      [[ -e "$item" ]] || continue
      item_mtime="$(stat -f '%m' "$item" 2>/dev/null || echo 0)"
      if [[ "$item_mtime" =~ ^[0-9]+$ ]] && [[ "$item_mtime" -ge "$min_ts" ]]; then
        valid_artifacts+=("$item")
      fi
    done
  fi

  log_info "本次构建产物 (目录: $OUTPUT_DIR):"
  if [[ ${#valid_artifacts[@]} -gt 0 ]]; then
    for item in "${valid_artifacts[@]}"; do
      size="$(du -sh "$item" | awk '{print $1}')"
      printf "  - %-40s (%s)\n" "$(basename "$item")" "$size"
    done
  else
    log_warn "本次运行未记录到新生成的构建产物"
  fi

  if [[ -d "$OUTPUT_DIR" ]]; then
    local stale_count=0
    local candidate built is_current
    for candidate in "$OUTPUT_DIR"/*; do
      [[ -e "$candidate" ]] || continue
      is_current=false
      if [[ ${#valid_artifacts[@]} -gt 0 ]]; then
        for built in "${valid_artifacts[@]}"; do
          if [[ "$candidate" == "$built" ]] || [[ "$candidate" -ef "$built" ]]; then
            is_current=true
            break
          fi
        done
      fi
      if [[ "$is_current" == false ]]; then
        stale_count=$((stale_count + 1))
      fi
    done
    if [[ $stale_count -gt 0 ]]; then
      log_info "提示: 输出目录中另有 ${stale_count} 个历史构建残留文件（本次未重新编译）"
    fi
  fi
}

main() {
  if [[ $# -gt 0 ]]; then
    die "本脚本已移除所有命令行参数，请直接运行 ./build.sh 并使用方向键菜单交互选择功能"
  fi

  run_interactive_wizard

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
  ensure_go
  ensure_core_submodule
  ensure_rust

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
