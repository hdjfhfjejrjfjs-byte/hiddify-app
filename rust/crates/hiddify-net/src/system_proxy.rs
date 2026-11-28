//! User-session system proxy management (Windows WinINet, Linux GNOME `gsettings` & KDE
//! `kwriteconfig5`/`kwriteconfig6` + D-Bus) and crashed engine `data/system-proxy.on` leftover
//! cleanup (including macOS `networksetup`).
//!
//! 100% behavior-equivalent port of:
//! - `v2/hcore/system_proxy.go`
//! - `v2/hcore/leftovers.go` (`system-proxy.on` marker & `CleanupLeftovers`)
//! - `v2/hcore/system_proxy_restore_darwin.go`
//! - `v2/hcore/system_proxy_restore_settings.go`
//! - `hiddify-sing-box/common/settings/proxy_windows.go` (`wininet.SetSystemProxy` / `ClearSystemProxy`)
//! - `hiddify-sing-box/common/settings/proxy_linux.go`

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use crate::admin_service::look_path;

pub const SYSTEM_PROXY_MARKER_FILE: &str = "system-proxy.on";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemProxyStatusOutcome {
    pub available: bool,
    pub enabled: bool,
}

#[derive(Default)]
struct UserProxyState {
    enabled: bool,
    address: String,
    support_socks: bool,
}

fn proxy_state() -> &'static Mutex<UserProxyState> {
    static STATE: OnceLock<Mutex<UserProxyState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(UserProxyState::default()))
}

/// Reports whether a root service delegates the user's system proxy to the user-space app
/// (`Windows` and `Linux`).
pub const fn system_proxy_delegation_supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "linux"))
}

pub fn get_system_proxy_status() -> SystemProxyStatusOutcome {
    let enabled = proxy_state()
        .lock()
        .map(|g| g.enabled)
        .unwrap_or(false);
    SystemProxyStatusOutcome {
        available: system_proxy_delegation_supported(),
        enabled,
    }
}

/// Parses and validates a `host:port` (or `[ipv6]:port`) proxy address.
pub fn parse_proxy_host_port(address: &str) -> Result<(String, u16), String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err("set system proxy: invalid address ".to_string());
    }
    let (host, port_str) = if let Some(rest) = trimmed.strip_prefix('[') {
        let Some((h, after)) = rest.split_once("]:") else {
            return Err(format!("set system proxy: invalid address {address}"));
        };
        (h.to_string(), after)
    } else {
        let Some((h, p)) = trimmed.rsplit_once(':') else {
            return Err(format!("set system proxy: invalid address {address}"));
        };
        (h.to_string(), p)
    };
    if host.is_empty() {
        return Err(format!("set system proxy: invalid address {address}"));
    }
    let port: u16 = port_str
        .parse()
        .ok()
        .filter(|&p| p > 0)
        .ok_or_else(|| format!("set system proxy: invalid address {address}"))?;
    Ok((host, port))
}

pub fn format_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub fn marker_path(working_dir: &Path) -> PathBuf {
    working_dir.join("data").join(SYSTEM_PROXY_MARKER_FILE)
}

pub fn write_user_proxy_marker(working_dir: Option<&Path>, address: &str) {
    let Some(dir) = working_dir.filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };
    let path = marker_path(dir);
    if address.trim().is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, address.trim().as_bytes());
}

pub fn read_proxy_marker_addresses(working_dir: &Path) -> Vec<String> {
    let path = marker_path(working_dir);
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Enables or disables the user-session system proxy, matching `(*CoreService).SetSystemProxyEnabled`.
/// When `is_enabled` is `false`, it also cleans up any stopped engine's `data/system-proxy.on` marker.
pub fn set_system_proxy_enabled(
    working_dir: Option<&Path>,
    is_enabled: bool,
    address: &str,
    support_socks: bool,
) -> Result<(), String> {
    if is_enabled {
        enable_user_system_proxy(working_dir, address, support_socks)
    } else {
        if let Some(dir) = working_dir.filter(|p| !p.as_os_str().is_empty()) {
            cleanup_system_proxy_leftovers(dir);
        }
        disable_user_system_proxy(working_dir)
    }
}

pub fn enable_user_system_proxy(
    working_dir: Option<&Path>,
    address: &str,
    support_socks: bool,
) -> Result<(), String> {
    if !system_proxy_delegation_supported() {
        return Err(format!(
            "set system proxy: not delegated on {}",
            std::env::consts::OS
        ));
    }
    let (host, port) = parse_proxy_host_port(address)?;
    let normalized = format_host_port(&host, port);

    let mut guard = proxy_state()
        .lock()
        .map_err(|_| "system proxy state lock poisoned".to_string())?;
    if guard.enabled {
        let _ = os_disable_system_proxy();
        guard.enabled = false;
        write_user_proxy_marker(working_dir, "");
    }
    os_enable_system_proxy(&host, port, support_socks)
        .map_err(|e| format!("set system proxy: {e}"))?;
    guard.enabled = true;
    guard.address = normalized.clone();
    guard.support_socks = support_socks;
    write_user_proxy_marker(working_dir, &normalized);
    Ok(())
}

pub fn disable_user_system_proxy(working_dir: Option<&Path>) -> Result<(), String> {
    let mut guard = proxy_state()
        .lock()
        .map_err(|_| "system proxy state lock poisoned".to_string())?;
    if !guard.enabled {
        return Ok(());
    }
    guard.enabled = false;
    guard.address.clear();
    let res = os_disable_system_proxy();
    write_user_proxy_marker(working_dir, "");
    res
}

/// Cleans up `data/system-proxy.on` left behind by a killed/crashed engine (`CleanupLeftovers`).
pub fn cleanup_system_proxy_leftovers(working_dir: &Path) {
    let is_currently_enabled = proxy_state()
        .lock()
        .map(|g| g.enabled)
        .unwrap_or(false);
    if is_currently_enabled {
        return;
    }
    let path = marker_path(working_dir);
    let Ok(content) = fs::read_to_string(&path) else {
        return;
    };
    for address in content.split_whitespace() {
        let _ = restore_system_proxy(address);
    }
    let _ = fs::remove_file(&path);
}

/// Restores the OS system proxy if it still points at `address` (`restoreSystemProxy`).
pub fn restore_system_proxy(address: &str) -> Result<(), String> {
    let (host, port) = parse_proxy_host_port(address)?;
    #[cfg(target_os = "macos")]
    {
        darwin_restore_system_proxy(&host, &port.to_string())
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        let _ = (host, port);
        os_disable_system_proxy()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (host, port);
        Ok(())
    }
}

// ============================================================================
// Platform 1: Windows WinINet (`wininet.SetSystemProxy` / `ClearSystemProxy`)
// ============================================================================

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    const INTERNET_OPTION_PER_CONNECTION_OPTION: u32 = 75;
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;
    const INTERNET_OPTION_REFRESH: u32 = 37;
    const INTERNET_OPTION_PROXY_SETTINGS_CHANGED: u32 = 95;

    const INTERNET_PER_CONN_FLAGS: u32 = 1;
    const INTERNET_PER_CONN_PROXY_SERVER: u32 = 2;
    const INTERNET_PER_CONN_PROXY_BYPASS: u32 = 3;

    const PROXY_TYPE_DIRECT: u32 = 1;
    const PROXY_TYPE_PROXY: u32 = 2;
    const PROXY_TYPE_AUTO_DETECT: u32 = 8;

    #[repr(C)]
    struct InternetPerConnOptionList {
        dw_size: u32,
        psz_connection: *const u16,
        dw_option_count: u32,
        dw_option_error: u32,
        p_options: *mut InternetPerConnOption,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct InternetPerConnOption {
        dw_option: u32,
        value: u64,
    }

    #[link(name = "wininet")]
    extern "system" {
        fn InternetSetOptionW(
            h_internet: *mut c_void,
            dw_option: u32,
            lp_buffer: *mut c_void,
            dw_buffer_length: u32,
        ) -> i32;
    }

    unsafe fn set_options(options: &mut [InternetPerConnOption]) -> Result<(), String> {
        let mut list = InternetPerConnOptionList {
            dw_size: std::mem::size_of::<InternetPerConnOptionList>() as u32,
            psz_connection: ptr::null(),
            dw_option_count: options.len() as u32,
            dw_option_error: 0,
            p_options: options.as_mut_ptr(),
        };
        if InternetSetOptionW(
            ptr::null_mut(),
            INTERNET_OPTION_PER_CONNECTION_OPTION,
            &mut list as *mut InternetPerConnOptionList as *mut c_void,
            list.dw_size,
        ) == 0
        {
            return Err(format!(
                "InternetSetOption(PerConnectionOption): {}",
                std::io::Error::last_os_error()
            ));
        }
        for (opt, name) in [
            (INTERNET_OPTION_SETTINGS_CHANGED, "SettingsChanged"),
            (INTERNET_OPTION_PROXY_SETTINGS_CHANGED, "ProxySettingsChanged"),
            (INTERNET_OPTION_REFRESH, "Refresh"),
        ] {
            if InternetSetOptionW(ptr::null_mut(), opt, ptr::null_mut(), 0) == 0 {
                return Err(format!(
                    "InternetSetOption({name}): {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }

    pub fn set_system_proxy(proxy_url: &str, bypass: &str) -> Result<(), String> {
        let proxy_w: Vec<u16> = std::ffi::OsStr::new(proxy_url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut flags_opt = InternetPerConnOption {
            dw_option: INTERNET_PER_CONN_FLAGS,
            value: 0,
        };
        let mut proxy_opt = InternetPerConnOption {
            dw_option: INTERNET_PER_CONN_PROXY_SERVER,
            value: 0,
        };
        unsafe {
            *(&mut flags_opt.value as *mut u64 as *mut u32) = PROXY_TYPE_PROXY | PROXY_TYPE_DIRECT;
            *(&mut proxy_opt.value as *mut u64 as *mut usize) = proxy_w.as_ptr() as usize;
            if bypass.is_empty() {
                let mut opts = [flags_opt, proxy_opt];
                set_options(&mut opts)
            } else {
                let bypass_w: Vec<u16> = std::ffi::OsStr::new(bypass)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let mut bypass_opt = InternetPerConnOption {
                    dw_option: INTERNET_PER_CONN_PROXY_BYPASS,
                    value: 0,
                };
                *(&mut bypass_opt.value as *mut u64 as *mut usize) = bypass_w.as_ptr() as usize;
                let mut opts = [flags_opt, proxy_opt, bypass_opt];
                set_options(&mut opts)
            }
        }
    }

    pub fn clear_system_proxy() -> Result<(), String> {
        let mut flags_opt = InternetPerConnOption {
            dw_option: INTERNET_PER_CONN_FLAGS,
            value: 0,
        };
        unsafe {
            *(&mut flags_opt.value as *mut u64 as *mut u32) =
                PROXY_TYPE_DIRECT | PROXY_TYPE_AUTO_DETECT;
            let mut opts = [flags_opt];
            set_options(&mut opts)
        }
    }
}

// ============================================================================
// Platform 2: Linux GNOME `gsettings` & KDE `kwriteconfig5`/`kwriteconfig6`
// ============================================================================

pub fn build_linux_enable_commands(
    host: &str,
    port: u16,
    support_socks: bool,
    has_gsettings: bool,
    kwrite_cmd: Option<&str>,
) -> Vec<(String, Vec<String>)> {
    let mut cmds = Vec::new();
    let port_str = port.to_string();
    let host_port = format_host_port(host, port);

    if has_gsettings {
        cmds.push((
            "gsettings".to_string(),
            vec![
                "set".to_string(),
                "org.gnome.system.proxy.http".to_string(),
                "enabled".to_string(),
                "true".to_string(),
            ],
        ));
        let gnome_types: &[&str] = if support_socks {
            &["ftp", "http", "https", "socks"]
        } else {
            &["http", "https"]
        };
        for &ptype in gnome_types {
            cmds.push((
                "gsettings".to_string(),
                vec![
                    "set".to_string(),
                    format!("org.gnome.system.proxy.{ptype}"),
                    "host".to_string(),
                    host.to_string(),
                ],
            ));
            cmds.push((
                "gsettings".to_string(),
                vec![
                    "set".to_string(),
                    format!("org.gnome.system.proxy.{ptype}"),
                    "port".to_string(),
                    port_str.clone(),
                ],
            ));
        }
        cmds.push((
            "gsettings".to_string(),
            vec![
                "set".to_string(),
                "org.gnome.system.proxy".to_string(),
                "use-same-proxy".to_string(),
                if support_socks { "true" } else { "false" }.to_string(),
            ],
        ));
        cmds.push((
            "gsettings".to_string(),
            vec![
                "set".to_string(),
                "org.gnome.system.proxy".to_string(),
                "mode".to_string(),
                "manual".to_string(),
            ],
        ));
    }

    if let Some(kw) = kwrite_cmd.filter(|s| !s.is_empty()) {
        cmds.push((
            kw.to_string(),
            vec![
                "--file".to_string(),
                "kioslaverc".to_string(),
                "--group".to_string(),
                "Proxy Settings".to_string(),
                "--key".to_string(),
                "ProxyType".to_string(),
                "1".to_string(),
            ],
        ));
        let kde_types: &[&str] = if support_socks {
            &["ftp", "http", "https", "socks"]
        } else {
            &["http", "https"]
        };
        for &ptype in kde_types {
            let proxy_url = if ptype == "socks" {
                format!("socks://{host_port}")
            } else {
                format!("http://{host_port}")
            };
            cmds.push((
                kw.to_string(),
                vec![
                    "--file".to_string(),
                    "kioslaverc".to_string(),
                    "--group".to_string(),
                    "Proxy Settings".to_string(),
                    "--key".to_string(),
                    format!("{ptype}Proxy"),
                    proxy_url,
                ],
            ));
        }
        cmds.push((
            kw.to_string(),
            vec![
                "--file".to_string(),
                "kioslaverc".to_string(),
                "--group".to_string(),
                "Proxy Settings".to_string(),
                "--key".to_string(),
                "Authmode".to_string(),
                "0".to_string(),
            ],
        ));
        cmds.push((
            "dbus-send".to_string(),
            vec![
                "--type=signal".to_string(),
                "/KIO/Scheduler".to_string(),
                "org.kde.KIO.Scheduler.reparseSlaveConfiguration".to_string(),
                "string:''".to_string(),
            ],
        ));
    }
    cmds
}

pub fn build_linux_disable_commands(
    has_gsettings: bool,
    kwrite_cmd: Option<&str>,
) -> Vec<(String, Vec<String>)> {
    let mut cmds = Vec::new();
    if has_gsettings {
        cmds.push((
            "gsettings".to_string(),
            vec![
                "set".to_string(),
                "org.gnome.system.proxy".to_string(),
                "mode".to_string(),
                "none".to_string(),
            ],
        ));
    }
    if let Some(kw) = kwrite_cmd.filter(|s| !s.is_empty()) {
        cmds.push((
            kw.to_string(),
            vec![
                "--file".to_string(),
                "kioslaverc".to_string(),
                "--group".to_string(),
                "Proxy Settings".to_string(),
                "--key".to_string(),
                "ProxyType".to_string(),
                "0".to_string(),
            ],
        ));
        cmds.push((
            "dbus-send".to_string(),
            vec![
                "--type=signal".to_string(),
                "/KIO/Scheduler".to_string(),
                "org.kde.KIO.Scheduler.reparseSlaveConfiguration".to_string(),
                "string:''".to_string(),
            ],
        ));
    }
    cmds
}

#[cfg(target_os = "linux")]
fn detect_linux_proxy_tools() -> Result<(bool, Option<&'static str>), String> {
    let has_gsettings = look_path("gsettings").is_some();
    let kwrite_cmd = if look_path("kwriteconfig5").is_some() {
        Some("kwriteconfig5")
    } else if look_path("kwriteconfig6").is_some() {
        Some("kwriteconfig6")
    } else {
        None
    };
    if !has_gsettings && kwrite_cmd.is_none() {
        return Err("unsupported desktop environment".to_string());
    }
    Ok((has_gsettings, kwrite_cmd))
}

#[cfg(target_os = "linux")]
fn run_linux_proxy_commands(cmds: &[(String, Vec<String>)]) -> Result<(), String> {
    for (prog, args) in cmds {
        let status = Command::new(prog)
            .args(args)
            .status()
            .map_err(|e| format!("{prog}: {e}"))?;
        if !status.success() {
            return Err(format!(
                "{prog} exited with code {}",
                status.code().unwrap_or(1)
            ));
        }
    }
    Ok(())
}

fn os_enable_system_proxy(host: &str, port: u16, support_socks: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = support_socks;
        let url = format!("http://{}", format_host_port(host, port));
        win::set_system_proxy(&url, "")
    }
    #[cfg(target_os = "linux")]
    {
        let (has_gsettings, kwrite_cmd) = detect_linux_proxy_tools()?;
        let cmds =
            build_linux_enable_commands(host, port, support_socks, has_gsettings, kwrite_cmd);
        run_linux_proxy_commands(&cmds)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (host, port, support_socks);
        Err(format!("not delegated on {}", std::env::consts::OS))
    }
}

fn os_disable_system_proxy() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        win::clear_system_proxy()
    }
    #[cfg(target_os = "linux")]
    {
        let (has_gsettings, kwrite_cmd) = detect_linux_proxy_tools()?;
        let cmds = build_linux_disable_commands(has_gsettings, kwrite_cmd);
        run_linux_proxy_commands(&cmds)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Ok(())
    }
}

// ============================================================================
// Platform 3: macOS `networksetup` Leftover Proxy Restoration
// ============================================================================

/// Parses `networksetup -get*proxy` output (`Enabled: Yes / Server: ... / Port: ...`),
/// matching `proxyPointsAt` in `v2/hcore/system_proxy_restore_darwin.go`.
pub fn darwin_proxy_points_at(info: &str, host: &str, port: &str) -> bool {
    let mut values = HashMap::new();
    for line in info.lines() {
        if let Some((k, v)) = line.split_once(':') {
            values.insert(k.trim(), v.trim());
        }
    }
    values.get("Enabled") == Some(&"Yes")
        && values.get("Server") == Some(&host)
        && values.get("Port") == Some(&port)
}

#[cfg(target_os = "macos")]
fn darwin_restore_system_proxy(host: &str, port: &str) -> Result<(), String> {
    let out = Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("networksetup -listallnetworkservices failed".to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for (i, service) in text.trim().lines().enumerate() {
        if i == 0 || service.is_empty() || service.starts_with('*') {
            continue;
        }
        for kind in ["webproxy", "securewebproxy", "socksfirewallproxy"] {
            let get_arg = format!("-get{kind}");
            let Ok(info_out) = Command::new("networksetup")
                .arg(&get_arg)
                .arg(service)
                .output()
            else {
                continue;
            };
            if !info_out.status.success() {
                continue;
            }
            let info_str = String::from_utf8_lossy(&info_out.stdout);
            if !darwin_proxy_points_at(&info_str, host, port) {
                continue;
            }
            let set_arg = format!("-set{kind}state");
            let status = Command::new("networksetup")
                .arg(&set_arg)
                .arg(service)
                .arg("off")
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("networksetup {set_arg} {service} off failed"));
            }
        }
    }
    Ok(())
}

/// Helper used by tests and callers to update `<working_dir>/data/system-proxy.on` with multiple
/// deduplicated addresses (`writeLeftoversLocked` in `v2/hcore/leftovers.go`).
pub fn write_system_proxy_marker_list(working_dir: &Path, addresses: &[&str]) -> io::Result<()> {
    let mut unique = Vec::new();
    for &addr in addresses {
        let trimmed = addr.trim();
        if !trimmed.is_empty() && !unique.contains(&trimmed) {
            unique.push(trimmed);
        }
    }
    let path = marker_path(working_dir);
    if unique.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, unique.join("\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darwin_proxy_points_at_and_marker_cleanup_match_go_tests() {
        let sample = "Enabled: Yes\nServer: 127.0.0.1\nPort: 12334\nAuthenticated Proxy Enabled: 0\n";
        assert!(darwin_proxy_points_at(sample, "127.0.0.1", "12334"));
        assert!(!darwin_proxy_points_at(sample, "127.0.0.1", "2334"));
        let disabled = "Enabled: No\nServer: 127.0.0.1\nPort: 12334\n";
        assert!(!darwin_proxy_points_at(disabled, "127.0.0.1", "12334"));

        // Port of TestEngineMarkerFollowsWhatTheEngineRuns & TestStoppedEngineLeftoversAreCleanedOnDisable
        let dir = tempfile::tempdir().unwrap();
        write_system_proxy_marker_list(dir.path(), &["127.0.0.1:2334"]).unwrap();
        assert_eq!(
            read_proxy_marker_addresses(dir.path()),
            vec!["127.0.0.1:2334".to_string()]
        );

        write_system_proxy_marker_list(dir.path(), &["127.0.0.1:2334", "127.0.0.1:2335", "127.0.0.1:2334"]).unwrap();
        assert_eq!(
            read_proxy_marker_addresses(dir.path()),
            vec!["127.0.0.1:2334".to_string(), "127.0.0.1:2335".to_string()]
        );

        // An invalid marker address is removed on cleanup without touching the host's real proxy
        write_system_proxy_marker_list(dir.path(), &["not-an-address"]).unwrap();
        assert!(marker_path(dir.path()).exists());
        cleanup_system_proxy_leftovers(dir.path());
        assert!(!marker_path(dir.path()).exists());
    }

    #[test]
    fn linux_gnome_and_kde_command_sequences_match_sing_box() {
        let enable_cmds = build_linux_enable_commands(
            "127.0.0.1",
            12334,
            true,
            true,
            Some("kwriteconfig6"),
        );
        // 1 (http enabled) + 4*2 (ftp/http/https/socks host+port) + 1 (use-same-proxy) + 1 (mode manual) = 11 gsettings
        // + 1 (ProxyType 1) + 4 (ftp/http/https/socksProxy) + 1 (Authmode 0) + 1 (dbus-send) = 7 kde
        assert_eq!(enable_cmds.len(), 18);
        assert_eq!(
            enable_cmds[0],
            (
                "gsettings".to_string(),
                vec![
                    "set".to_string(),
                    "org.gnome.system.proxy.http".to_string(),
                    "enabled".to_string(),
                    "true".to_string()
                ]
            )
        );
        assert!(enable_cmds.iter().any(|(c, a)| c == "kwriteconfig6"
            && a.contains(&"socksProxy".to_string())
            && a.contains(&"socks://127.0.0.1:12334".to_string())));

        let disable_cmds = build_linux_disable_commands(true, Some("kwriteconfig6"));
        assert_eq!(disable_cmds.len(), 3);
    }
}
