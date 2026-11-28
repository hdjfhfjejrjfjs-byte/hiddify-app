//! Desktop privilege detection (`IsAdmin`, `TunAllowed`), background service readiness probing
//! (`NeedAdminBgService`), and cross-platform UAC / PolicyKit / sudo elevation (`EnsureAdminBgService`).
//!
//! 100% behavior-equivalent port of:
//! - `v2/hcore/admin_bgservice.go`
//! - `v2/hcore/admin_bgservice_elevate_windows.go`
//! - `v2/hcore/admin_bgservice_cli_windows.go`
//! - `v2/hcore/admin_bgservice_elevate_linux.go`
//! - `v2/hcore/admin_bgservice_terminal_linux.go`
//! - `v2/hutils/windows_utils.go` & `v2/hutils/linux_utils.go`

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ADMIN_BG_SERVICE_PORT: u16 = 18020;

const ERR_NO_ADMIN_PROMPT_MSG: &str = "VPN mode needs pkexec to ask for the administrator password, and it is not installed: install the pkexec package (polkit) and try again, or use system proxy mode";
const ERR_NO_ADMIN_AGENT_MSG: &str = "could not ask for the administrator password";
const ERR_ADMIN_PROMPT_REFUSED_MSG: &str =
    "could not install the VPN service; allow the administrator prompt and try again";

/// Matches `hcore.AdminPreference` in `v2/hcore/hcore.pb.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum AdminPreference {
    Unset = 0,
    PreferAdmin = 1,
    PreferNoAdmin = 2,
}

impl AdminPreference {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::PreferAdmin,
            2 => Self::PreferNoAdmin,
            _ => Self::Unset,
        }
    }
}

/// Matches `hcore.AdminBgServiceDecision` in `v2/hcore/hcore.pb.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum AdminBgServiceDecision {
    No = 0,
    AlreadyConnected = 1,
    NeedsAdmin = 2,
    OptionalProxyNeedAdmin = 3,
}

impl AdminBgServiceDecision {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Matches `hcore.EnsureAdminBgServiceProgress` in `v2/hcore/hcore.pb.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum EnsureAdminBgServiceProgress {
    WaitingForElevation = 0,
    ServiceStarting = 1,
    ServiceConnected = 2,
    FailedPermissionDenied = 3,
    FailedOther = 4,
}

impl EnsureAdminBgServiceProgress {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminNeededProxy {
    pub tag: String,
    pub summary: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedAdminBgServiceRequest {
    pub service_mode: String,
    pub hiddify_settings_json: String,
    pub admin_preference: i32,
    pub privilege_error_remembered: bool,
    pub cli_path: Option<String>,
    pub build_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedAdminBgServiceOutcome {
    pub decision: i32,
    pub admin_needed_proxies: Vec<AdminNeededProxy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnsureAdminBgServiceRequest {
    pub config_content: String,
    pub config_name: String,
    pub settings_json: String,
    /// Map of resource key -> base64-encoded bytes (matching Go's `map[string][]byte` JSON encoding).
    pub resources_b64: BTreeMap<String, String>,
    pub disable_memory_limit: bool,
    pub owner_pid: i32,
    pub cli_path: String,
    pub owner_user: String,
    pub proxy_only: bool,
    pub build_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnsureAdminBgServiceEvent {
    pub progress: i32,
    pub message: String,
    pub service_port: u32,
}

impl EnsureAdminBgServiceEvent {
    fn new(progress: EnsureAdminBgServiceProgress, message: impl Into<String>, port: u16) -> Self {
        Self {
            progress: progress.as_i32(),
            message: message.into(),
            service_port: port as u32,
        }
    }
}

// ============================================================================
// 1. Privilege Detection (`IsAdmin` & `TunAllowed`)
// ============================================================================

/// Reports whether the current process runs with administrator (Windows) or root (Unix) privileges.
/// Checks the process token / UID in-process without spawning `id -u` or PowerShell.
pub fn is_admin() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_is_admin()
    }
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 || libc::getuid() == 0 }
    }
    #[cfg(not(any(target_os = "windows", unix)))]
    {
        false
    }
}

/// Reports whether the current process is allowed to open a TUN device (`IsAdmin` on Windows,
/// `CAP_NET_ADMIN` effective capability or root on Linux).
pub fn tun_allowed() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_is_admin()
    }
    #[cfg(target_os = "linux")]
    {
        linux_tun_allowed()
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        is_admin()
    }
    #[cfg(not(any(target_os = "windows", unix)))]
    {
        false
    }
}

#[cfg(target_os = "windows")]
fn windows_is_admin() -> bool {
    use std::ffi::c_void;
    use std::ptr;

    const WIN_BUILTIN_ADMINISTRATORS_SID: u32 = 26;
    extern "system" {
        fn CreateWellKnownSid(
            well_known_sid_type: u32,
            domain_sid: *mut c_void,
            sid: *mut u8,
            cb_sid: *mut u32,
        ) -> i32;
        fn CheckTokenMembership(
            token_handle: *mut c_void,
            sid_to_check: *mut u8,
            is_member: *mut i32,
        ) -> i32;
    }

    unsafe {
        let mut sid = [0u8; 68];
        let mut cb_sid = sid.len() as u32;
        if CreateWellKnownSid(
            WIN_BUILTIN_ADMINISTRATORS_SID,
            ptr::null_mut(),
            sid.as_mut_ptr(),
            &mut cb_sid,
        ) == 0
        {
            return false;
        }
        let mut is_member: i32 = 0;
        if CheckTokenMembership(ptr::null_mut(), sid.as_mut_ptr(), &mut is_member) == 0 {
            return false;
        }
        is_member != 0
    }
}

#[cfg(target_os = "linux")]
fn linux_tun_allowed() -> bool {
    if is_admin() {
        return true;
    }
    #[repr(C)]
    #[derive(Default)]
    struct CapUserHeader {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct CapUserData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    const CAP_NET_ADMIN: u32 = 12;

    let mut hdr = CapUserHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let mut data = [CapUserData::default(); 2];
    let rc = unsafe {
        libc::syscall(
            libc::SYS_capget,
            &mut hdr as *mut CapUserHeader,
            data.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return false;
    }
    (data[0].effective & (1u32 << CAP_NET_ADMIN)) != 0
}

// ============================================================================
// 2. Settings Inspection & Service Readiness Probing
// ============================================================================

pub fn is_vpn_service_mode(mode: &str, settings_json: &str) -> bool {
    match mode.trim().to_ascii_lowercase().as_str() {
        "vpn" | "tun" | "router" => return true,
        "proxy" | "system-proxy" | "system_proxy" => return false,
        _ => {}
    }
    let Ok(root) = parse_settings_map(settings_json) else {
        return false;
    };
    if root.get("router-mode").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if let Some(v) = root.get("enable-tun").and_then(Value::as_bool) {
        return v;
    }
    if let Some(inbound) = root.get("inbound_options").and_then(Value::as_object) {
        if let Some(v) = inbound.get("enable_tun").and_then(Value::as_bool) {
            return v;
        }
        if let Some(v) = inbound.get("enableTun").and_then(Value::as_bool) {
            return v;
        }
    }
    false
}

pub fn scan_admin_needed_features(settings_json: &str) -> Vec<AdminNeededProxy> {
    let Ok(root) = parse_settings_map(settings_json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if mixed_sni_enabled(&root) {
        out.push(AdminNeededProxy {
            tag: "tls-tricks".to_string(),
            summary: "Mixed SNI case".to_string(),
            detail: "TLS mixed-SNI case needs administrator permission on this platform."
                .to_string(),
        });
    }
    out
}

fn mixed_sni_enabled(root: &Map<String, Value>) -> bool {
    if let Some(tricks) = root.get("tls-tricks").and_then(Value::as_object) {
        if let Some(v) = tricks.get("mixed-sni-case").and_then(Value::as_bool) {
            return v;
        }
        if let Some(v) = tricks.get("mixedSniCase").and_then(Value::as_bool) {
            return v;
        }
    }
    if let Some(tricks) = root.get("tls_tricks").and_then(Value::as_object) {
        if let Some(v) = tricks.get("mixed_sni_case").and_then(Value::as_bool) {
            return v;
        }
    }
    false
}

fn parse_settings_map(settings_json: &str) -> Result<Map<String, Value>, String> {
    if settings_json.trim().is_empty() {
        return Err("empty settings".to_string());
    }
    let val: Value = serde_json::from_str(settings_json).map_err(|e| e.to_string())?;
    val.as_object()
        .cloned()
        .ok_or_else(|| "settings must be a JSON object".to_string())
}

pub fn admin_bg_data_dir() -> PathBuf {
    if let Ok(dir) = env::var("HIDDIFY_TUNNEL_DATA") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if cfg!(target_os = "windows") {
        let base = env::var("PROGRAMDATA")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| r"C:\ProgramData".to_string());
        PathBuf::from(base).join("Hiddify").join("AdminBgService")
    } else {
        PathBuf::from("/var/lib/hiddify-admin-bgservice")
    }
}

pub fn admin_bg_secret_path() -> PathBuf {
    admin_bg_data_dir().join("control.secret")
}

pub fn admin_bg_build_stamp_path() -> PathBuf {
    admin_bg_data_dir().join("build.id")
}

fn query_cli_build_id(cli_path: Option<&str>, explicit_build_id: Option<&str>) -> String {
    if let Some(id) = explicit_build_id {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let Some(cli) = cli_path.map(str::trim).filter(|s| !s.is_empty()) else {
        return String::new();
    };
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(cli) {
            return cached.clone();
        }
    }
    let mut cmd = Command::new(cli);
    cmd.arg("admin_bgservice").arg("build-id");
    if let Some(parent) = Path::new(cli).parent() {
        if !parent.as_os_str().is_empty() {
            cmd.current_dir(parent);
        }
    }
    prepare_cli_command(&mut cmd);
    let out = cmd
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if !out.is_empty() && out != "unset" {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(cli.to_string(), out.clone());
        }
    }
    out
}

pub fn admin_bg_service_ready(cli_path: Option<&str>, explicit_build_id: Option<&str>) -> bool {
    let Ok(secret) = fs::read_to_string(admin_bg_secret_path()) else {
        return false;
    };
    if secret.trim().len() < 32 {
        return false;
    }
    let Ok(stamped) = fs::read_to_string(admin_bg_build_stamp_path()) else {
        return false;
    };
    let want = stamped.trim();
    if want.is_empty() || want == "unset" {
        return false;
    }
    let have = query_cli_build_id(cli_path, explicit_build_id);
    if !have.is_empty() && (have == "unset" || want != have) {
        return false;
    }
    if have.is_empty() && (cli_path.is_some() || explicit_build_id.is_some()) {
        return false;
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], ADMIN_BG_SERVICE_PORT));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

pub fn need_admin_bg_service(req: &NeedAdminBgServiceRequest) -> NeedAdminBgServiceOutcome {
    if !cfg!(any(target_os = "windows", target_os = "linux")) {
        return NeedAdminBgServiceOutcome {
            decision: AdminBgServiceDecision::No.as_i32(),
            admin_needed_proxies: Vec::new(),
        };
    }
    need_admin_bg_service_with(
        req,
        is_admin(),
        admin_bg_service_ready(req.cli_path.as_deref(), req.build_id.as_deref()),
    )
}

pub fn need_admin_bg_service_with(
    req: &NeedAdminBgServiceRequest,
    currently_admin: bool,
    service_ready: bool,
) -> NeedAdminBgServiceOutcome {
    let features = scan_admin_needed_features(&req.hiddify_settings_json);
    let vpn = is_vpn_service_mode(&req.service_mode, &req.hiddify_settings_json);

    if currently_admin {
        return NeedAdminBgServiceOutcome {
            decision: AdminBgServiceDecision::No.as_i32(),
            admin_needed_proxies: Vec::new(),
        };
    }

    if vpn {
        let decision = if service_ready {
            AdminBgServiceDecision::AlreadyConnected
        } else {
            AdminBgServiceDecision::NeedsAdmin
        };
        return NeedAdminBgServiceOutcome {
            decision: decision.as_i32(),
            admin_needed_proxies: Vec::new(),
        };
    }

    let needs_optional = !features.is_empty() || req.privilege_error_remembered;
    if !needs_optional {
        return NeedAdminBgServiceOutcome {
            decision: AdminBgServiceDecision::No.as_i32(),
            admin_needed_proxies: Vec::new(),
        };
    }

    match AdminPreference::from_i32(req.admin_preference) {
        AdminPreference::PreferAdmin => NeedAdminBgServiceOutcome {
            decision: if service_ready {
                AdminBgServiceDecision::AlreadyConnected.as_i32()
            } else {
                AdminBgServiceDecision::NeedsAdmin.as_i32()
            },
            admin_needed_proxies: features,
        },
        AdminPreference::PreferNoAdmin => NeedAdminBgServiceOutcome {
            decision: AdminBgServiceDecision::No.as_i32(),
            admin_needed_proxies: features,
        },
        AdminPreference::Unset => NeedAdminBgServiceOutcome {
            decision: AdminBgServiceDecision::OptionalProxyNeedAdmin.as_i32(),
            admin_needed_proxies: features,
        },
    }
}

// ============================================================================
// 3. Elevation & Service Installation (`EnsureAdminBgService`)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum ElevateError {
    NoAdminPrompt(String),
    NoAdminAgent(String),
    AdminPromptRefused(String),
    Other(String),
}

impl ElevateError {
    fn message(&self) -> &str {
        match self {
            Self::NoAdminPrompt(m)
            | Self::NoAdminAgent(m)
            | Self::AdminPromptRefused(m)
            | Self::Other(m) => m,
        }
    }

    fn is_permission_denied(&self) -> bool {
        match self {
            Self::NoAdminPrompt(_) | Self::NoAdminAgent(_) => false,
            Self::AdminPromptRefused(_) => true,
            Self::Other(msg) => is_permission_denied_message(msg),
        }
    }
}

pub fn is_permission_denied_message(msg: &str) -> bool {
    if msg.contains(ERR_NO_ADMIN_PROMPT_MSG) || msg.contains(ERR_NO_ADMIN_AGENT_MSG) {
        return false;
    }
    if msg.contains(ERR_ADMIN_PROMPT_REFUSED_MSG) {
        return true;
    }
    let lower = msg.to_ascii_lowercase();
    for needle in [
        "permission denied",
        "access is denied",
        "operation not permitted",
        "user canceled",
        "user cancelled",
        "canceled by the user",
        "cancelled by the user",
        "polkit",
        "not authorized",
        "elevation",
    ] {
        if lower.contains(needle) {
            return true;
        }
    }
    false
}

pub fn ensure_admin_bg_service(
    req: &EnsureAdminBgServiceRequest,
) -> Vec<EnsureAdminBgServiceEvent> {
    let mut events = Vec::new();
    if !cfg!(any(target_os = "windows", target_os = "linux")) {
        events.push(EnsureAdminBgServiceEvent::new(
            EnsureAdminBgServiceProgress::FailedOther,
            "admin_bgservice is only available on Windows and Linux",
            0,
        ));
        return events;
    }
    let cli_path = req.cli_path.trim();
    if cli_path.is_empty() {
        events.push(EnsureAdminBgServiceEvent::new(
            EnsureAdminBgServiceProgress::FailedOther,
            "cli_path is required",
            0,
        ));
        return events;
    }
    if req.config_content.is_empty() {
        events.push(EnsureAdminBgServiceEvent::new(
            EnsureAdminBgServiceProgress::FailedOther,
            "missing configuration",
            0,
        ));
        return events;
    }

    let explicit_build_id = req.build_id.as_deref();
    if !admin_bg_service_ready(Some(cli_path), explicit_build_id) {
        events.push(EnsureAdminBgServiceEvent::new(
            EnsureAdminBgServiceProgress::WaitingForElevation,
            "Administrator permission is required to install the VPN service",
            ADMIN_BG_SERVICE_PORT,
        ));
        if let Err(err) =
            ensure_admin_bg_service_installed(cli_path, &req.owner_user, explicit_build_id)
        {
            let progress = if err.is_permission_denied() {
                EnsureAdminBgServiceProgress::FailedPermissionDenied
            } else {
                EnsureAdminBgServiceProgress::FailedOther
            };
            events.push(EnsureAdminBgServiceEvent::new(progress, err.message(), 0));
            return events;
        }
        if let Err(msg) = wait_admin_bg_service_ready(
            Duration::from_secs(25),
            Some(cli_path),
            explicit_build_id,
        ) {
            events.push(EnsureAdminBgServiceEvent::new(
                EnsureAdminBgServiceProgress::FailedOther,
                msg,
                0,
            ));
            return events;
        }
    }

    events.push(EnsureAdminBgServiceEvent::new(
        EnsureAdminBgServiceProgress::ServiceStarting,
        "Starting the VPN service",
        ADMIN_BG_SERVICE_PORT,
    ));

    let tmp_path = match write_admin_bg_connect_request(req) {
        Ok(p) => p,
        Err(msg) => {
            events.push(EnsureAdminBgServiceEvent::new(
                EnsureAdminBgServiceProgress::FailedOther,
                msg,
                0,
            ));
            return events;
        }
    };
    struct TempFileGuard(PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _tmp_guard = TempFileGuard(tmp_path.clone());

    let tmp_str = tmp_path.to_string_lossy().into_owned();
    if let Err(msg) = run_cli(cli_path, &["admin_bgservice", "connect", &tmp_str]) {
        let progress = if is_permission_denied_message(&msg) {
            EnsureAdminBgServiceProgress::FailedPermissionDenied
        } else {
            EnsureAdminBgServiceProgress::FailedOther
        };
        events.push(EnsureAdminBgServiceEvent::new(progress, msg, 0));
        return events;
    }

    events.push(EnsureAdminBgServiceEvent::new(
        EnsureAdminBgServiceProgress::ServiceConnected,
        "VPN service is ready",
        ADMIN_BG_SERVICE_PORT,
    ));
    events
}

fn wait_admin_bg_service_ready(
    timeout: Duration,
    cli_path: Option<&str>,
    explicit_build_id: Option<&str>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if admin_bg_service_ready(cli_path, explicit_build_id) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("the VPN service was installed but did not start with this app build".to_string())
}

fn ensure_admin_bg_service_installed(
    cli_path: &str,
    owner_user: &str,
    explicit_build_id: Option<&str>,
) -> Result<(), ElevateError> {
    if admin_bg_service_ready(Some(cli_path), explicit_build_id) {
        return Ok(());
    }
    if admin_bg_secret_path().exists() {
        let stamped = fs::read_to_string(admin_bg_build_stamp_path()).unwrap_or_default();
        let have = query_cli_build_id(Some(cli_path), explicit_build_id);
        if !have.is_empty() && stamped.trim() == have.trim() {
            if run_cli(cli_path, &["admin_bgservice", "start"]).is_ok()
                && admin_bg_service_ready(Some(cli_path), explicit_build_id)
            {
                return Ok(());
            }
        }
    }
    let mut owner = owner_user.trim().to_string();
    if owner.is_empty() {
        owner = env::var("USER")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| env::var("USERNAME").ok().filter(|s| !s.is_empty()))
            .unwrap_or_default();
    }
    if !owner.is_empty() {
        env::set_var("HIDDIFY_TUNNEL_OWNER", &owner);
    }
    elevate_admin_bg_install(cli_path, &owner)
}

fn prepare_cli_command(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.env("__COMPAT_LAYER", "RunAsInvoker");
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = cmd;
    }
}

fn explain_cli_error(cli_path: &str, err: &std::io::Error) -> String {
    #[cfg(target_os = "windows")]
    {
        const ERROR_ELEVATION_REQUIRED: i32 = 740;
        if err.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) {
            return format!(
                "Windows asks for administrator rights to run {cli_path}: turn off \"Run this program as an administrator\" in its Properties > Compatibility, then try again: {err}"
            );
        }
    }
    let _ = cli_path;
    err.to_string()
}

pub fn run_cli(cli_path: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(cli_path);
    cmd.args(args);
    if let Some(dir) = Path::new(cli_path).parent() {
        if !dir.as_os_str().is_empty() {
            cmd.current_dir(dir);
        }
    }
    prepare_cli_command(&mut cmd);
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                return Ok(());
            }
            let mut combined = Vec::with_capacity(output.stdout.len() + output.stderr.len());
            combined.extend_from_slice(&output.stdout);
            combined.extend_from_slice(&output.stderr);
            let msg = String::from_utf8_lossy(&combined).trim().to_string();
            if msg.is_empty() {
                Err(format!(
                    "command exited with status {}",
                    output.status.code().unwrap_or(1)
                ))
            } else {
                Err(msg)
            }
        }
        Err(e) => Err(explain_cli_error(cli_path, &e)),
    }
}

fn write_admin_bg_connect_request(req: &EnsureAdminBgServiceRequest) -> Result<PathBuf, String> {
    #[derive(Serialize)]
    struct ConnectBody<'a> {
        #[serde(rename = "configContent")]
        config_content: &'a str,
        #[serde(rename = "configName")]
        config_name: &'a str,
        #[serde(rename = "settingsJSON")]
        settings_json: &'a str,
        resources: &'a BTreeMap<String, String>,
        #[serde(rename = "disableMemoryLimit")]
        disable_memory_limit: bool,
        #[serde(rename = "ownerPid")]
        owner_pid: i32,
        #[serde(rename = "proxyOnly")]
        proxy_only: bool,
    }

    let raw = serde_json::to_vec(&ConnectBody {
        config_content: &req.config_content,
        config_name: &req.config_name,
        settings_json: &req.settings_json,
        resources: &req.resources_b64,
        disable_memory_limit: req.disable_memory_limit,
        owner_pid: req.owner_pid,
        proxy_only: req.proxy_only,
    })
    .map_err(|e| e.to_string())?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let path = env::temp_dir().join(format!("hiddify-admin-bgservice-{pid}-{nanos}.json"));
    fs::write(&path, &raw).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Quotes one argument for a Windows `CreateProcess`-style command line (`quoteWindowsArg`).
pub fn quote_windows_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    if !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    format!("\"{}\"", arg.replace('"', "\\\""))
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn shell_word(s: &str) -> String {
    if !s.is_empty()
        && !s.contains([
            ' ', '\t', '\'', '"', '$', '`', '\\', ';', '&', '|', '<', '>', '(', ')', '*', '?', '[',
            ']', '{', '}', '~', '!', '#',
        ])
    {
        return s.to_string();
    }
    shell_quote(s)
}

#[cfg(target_os = "windows")]
fn elevate_admin_bg_install(cli_path: &str, owner: &str) -> Result<(), ElevateError> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
    const SEE_MASK_FLAG_NO_UI: u32 = 0x0000_0400;
    const SEE_MASK_NOZONECHECKS: u32 = 0x0080_0000;
    const SW_HIDE: i32 = 0;
    const INFINITE: u32 = 0xFFFF_FFFF;

    #[repr(C)]
    struct ShellExecuteInfoW {
        cb_size: u32,
        f_mask: u32,
        hwnd: *mut c_void,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: *mut c_void,
        lp_id_list: *mut c_void,
        lp_class: *const u16,
        hkey_class: *mut c_void,
        dw_hot_key: u32,
        h_icon: *mut c_void,
        h_process: *mut c_void,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteExW(exec_info: *mut ShellExecuteInfoW) -> i32;
    }
    extern "system" {
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(process: *mut c_void, exit_code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    let dir = Path::new(cli_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let exe = dir.join("HiddifyAdmin.exe");
    if !exe.exists() {
        return Err(ElevateError::Other(format!(
            "HiddifyAdmin.exe could not be found in {}; reinstall Hiddify",
            dir.display()
        )));
    }
    let args = format!("admin_bgservice install {}", quote_windows_arg(owner));

    let verb_w: Vec<u16> = std::ffi::OsStr::new("runas")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let file_w: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let params_w: Vec<u16> = std::ffi::OsStr::new(&args)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut info = ShellExecuteInfoW {
            cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
            f_mask: SEE_MASK_NOCLOSEPROCESS
                | SEE_MASK_NOASYNC
                | SEE_MASK_FLAG_NO_UI
                | SEE_MASK_NOZONECHECKS,
            hwnd: ptr::null_mut(),
            lp_verb: verb_w.as_ptr(),
            lp_file: file_w.as_ptr(),
            lp_parameters: params_w.as_ptr(),
            lp_directory: ptr::null(),
            n_show: SW_HIDE,
            h_inst_app: ptr::null_mut(),
            lp_id_list: ptr::null_mut(),
            lp_class: ptr::null(),
            hkey_class: ptr::null_mut(),
            dw_hot_key: 0,
            h_icon: ptr::null_mut(),
            h_process: ptr::null_mut(),
        };
        if ShellExecuteExW(&mut info) == 0 {
            let err = std::io::Error::last_os_error();
            return Err(ElevateError::AdminPromptRefused(format!(
                "could not install the VPN service; allow the administrator prompt and try again: {err}"
            )));
        }
        if info.h_process.is_null() {
            return Err(ElevateError::Other(
                "could not install the VPN service; allow the administrator prompt and try again: elevated process handle not returned".to_string(),
            ));
        }
        let _ = WaitForSingleObject(info.h_process, INFINITE);
        let mut code: u32 = 1;
        let ok = GetExitCodeProcess(info.h_process, &mut code);
        let _ = CloseHandle(info.h_process);
        if ok == 0 {
            return Err(ElevateError::Other(
                "could not read VPN service installer exit code".to_string(),
            ));
        }
        if code != 0 {
            return Err(ElevateError::Other(format!(
                "the VPN service installer exited with code {code}; allow the administrator prompt and try again"
            )));
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn elevate_admin_bg_install(cli_path: &str, owner: &str) -> Result<(), ElevateError> {
    if let Ok(appimage) = env::var("APPIMAGE") {
        if !appimage.is_empty() {
            return Err(ElevateError::Other(format!(
                "VPN mode in the AppImage needs root; quit and run: sudo {appimage}"
            )));
        }
    }
    let args = vec![
        cli_path.to_string(),
        "admin_bgservice".to_string(),
        "install".to_string(),
        owner.to_string(),
    ];
    if look_path("pkexec").is_some() {
        let mut cmd_args = vec!["pkexec".to_string()];
        cmd_args.extend(args.iter().cloned());
        match run_elevated(&cmd_args, owner, None, Some(pkexec_refused)) {
            Ok(()) => return Ok(()),
            Err(err) => {
                if !pkexec_has_no_agent(err.message()) {
                    return Err(err);
                }
                match elevate_with_sudo(&args, owner) {
                    Err(ElevateError::NoAdminPrompt(_)) => {
                        return Err(ElevateError::NoAdminAgent(no_admin_agent_error(
                            cli_path, owner,
                        )));
                    }
                    other => return other,
                }
            }
        }
    }
    elevate_with_sudo(&args, owner)
}

pub fn pkexec_refused(code: i32, _stderr: &str) -> bool {
    code == 126
}

pub fn pkexec_has_no_agent(msg: &str) -> bool {
    msg.contains("textual authentication agent")
}

pub fn no_admin_agent_error(cli_path: &str, owner: &str) -> String {
    format!(
        "{ERR_NO_ADMIN_AGENT_MSG}: no authentication agent is running, as when Hiddify is started from a terminal or over SSH. Start Hiddify from the applications menu, or run once: sudo {} admin_bgservice install {}",
        shell_word(cli_path),
        shell_word(owner)
    )
}

pub fn sudo_refused(code: i32, stderr: &str) -> bool {
    if code != 1 {
        return false;
    }
    for refused in [
        "no password was provided",
        "incorrect password",
        "a password is required",
    ] {
        if stderr.contains(refused) {
            return true;
        }
    }
    false
}

#[cfg(not(target_os = "windows"))]
const ASKPASS_DIALOGS: &[(&str, &str)] = &[
    (
        "zenity",
        r#"--password --title="Hiddify: administrator password for VPN mode""#,
    ),
    (
        "kdialog",
        r#"--title=Hiddify --password="Administrator password for VPN mode""#,
    ),
    (
        "qarma",
        r#"--password --title="Hiddify: administrator password for VPN mode""#,
    ),
    ("ksshaskpass", r#""$@""#),
    ("lxqt-openssh-askpass", r#""$@""#),
    ("ssh-askpass", r#""$@""#),
    ("/usr/libexec/openssh/gnome-ssh-askpass", r#""$@""#),
    ("/usr/lib/openssh/gnome-ssh-askpass", r#""$@""#),
    ("/usr/lib/ssh/x11-ssh-askpass", r#""$@""#),
];

#[cfg(not(target_os = "windows"))]
const TERMINALS: &[(&str, &[&str], &str)] = &[
    ("ptyxis", &["--new-window", "--"], "GNOME"),
    ("kgx", &["--"], "GNOME"),
    ("gnome-terminal", &["--"], "GNOME"),
    ("konsole", &["-e"], "KDE"),
    ("xfce4-terminal", &["-x"], "XFCE"),
    ("mate-terminal", &["-x"], "MATE"),
    ("qterminal", &["-e"], "LXQT"),
    ("lxterminal", &["-e"], "LXDE"),
    ("cosmic-term", &["-e"], "COSMIC"),
    ("x-terminal-emulator", &["-e"], ""),
    ("tilix", &["-e"], ""),
    ("terminator", &["-x"], ""),
    ("kitty", &[], ""),
    ("alacritty", &["-e"], ""),
    ("foot", &[], ""),
    ("wezterm", &["start", "--"], ""),
    ("urxvt", &["-e"], ""),
    ("xterm", &["-e"], ""),
];

pub fn look_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return is_executable(&p).then_some(p);
    }
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        (meta.permissions().mode() & 0o111) != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(not(target_os = "windows"))]
fn elevate_with_sudo(args: &[String], owner: &str) -> Result<(), ElevateError> {
    if look_path("sudo").is_none() {
        return Err(ElevateError::NoAdminPrompt(
            ERR_NO_ADMIN_PROMPT_MSG.to_string(),
        ));
    }
    let (askpass, cleanup_dir) = sudo_askpass()?;
    struct DirCleanup(Option<PathBuf>);
    impl Drop for DirCleanup {
        fn drop(&mut self) {
            if let Some(ref d) = self.0 {
                let _ = fs::remove_dir_all(d);
            }
        }
    }
    let _cleanup = DirCleanup(cleanup_dir);

    if askpass.is_empty() {
        let mut non_interactive = vec!["sudo".to_string(), "-n".to_string()];
        non_interactive.extend(args.iter().cloned());
        if run_elevated(&non_interactive, owner, None, None).is_ok() {
            return Ok(());
        }
        return elevate_in_terminal(args);
    }

    let mut cmd_args = vec!["sudo".to_string(), "-A".to_string()];
    cmd_args.extend(args.iter().cloned());
    run_elevated(
        &cmd_args,
        owner,
        Some(("SUDO_ASKPASS", &askpass)),
        Some(sudo_refused),
    )
}

#[cfg(not(target_os = "windows"))]
fn sudo_askpass() -> Result<(String, Option<PathBuf>), ElevateError> {
    if let Ok(own) = env::var("SUDO_ASKPASS") {
        if !own.is_empty() && Path::new(&own).exists() {
            return Ok((own, None));
        }
    }
    let has_display = env::var("DISPLAY").ok().filter(|s| !s.is_empty()).is_some()
        || env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
    if !has_display {
        return Ok((String::new(), None));
    }
    for &(program_name, dialog_args) in ASKPASS_DIALOGS {
        let Some(program) = look_path(program_name) else {
            continue;
        };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!("hiddify-askpass-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| {
            ElevateError::Other(format!("could not ask for the administrator password: {e}"))
        })?;
        let script_path = dir.join("askpass");
        let prog_str = program.to_string_lossy().replace('\'', r"'\''");
        let script = format!("#!/bin/sh\nexec '{prog_str}' {dialog_args}\n");
        if let Err(e) = write_executable_script(&script_path, &script) {
            let _ = fs::remove_dir_all(&dir);
            return Err(ElevateError::Other(format!(
                "could not ask for the administrator password: {e}"
            )));
        }
        return Ok((script_path.to_string_lossy().into_owned(), Some(dir)));
    }
    Ok((String::new(), None))
}

#[cfg(not(target_os = "windows"))]
fn write_executable_script(path: &Path, content: &str) -> std::io::Result<()> {
    fs::write(path, content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn find_terminal() -> Option<(PathBuf, Vec<String>)> {
    let has_display = env::var("DISPLAY").ok().filter(|s| !s.is_empty()).is_some()
        || env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
    if !has_display {
        return None;
    }
    if let Ok(own_term) = env::var("TERMINAL") {
        if let Some(first) = own_term.split_whitespace().next() {
            if let Some(path) = look_path(first) {
                let base = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                for &(prog, args, _) in TERMINALS {
                    if base == prog {
                        return Some((path, args.iter().map(|s| s.to_string()).collect()));
                    }
                }
                return Some((path, vec!["-e".to_string()]));
            }
        }
    }
    let desktop = env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_uppercase();
    for own_desktop in [true, false] {
        for &(prog, args, dt) in TERMINALS {
            if own_desktop && (dt.is_empty() || !desktop.contains(dt)) {
                continue;
            }
            if let Some(path) = look_path(prog) {
                return Some((path, args.iter().map(|s| s.to_string()).collect()));
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn elevate_in_terminal(args: &[String]) -> Result<(), ElevateError> {
    let Some((terminal, terminal_args)) = find_terminal() else {
        return Err(ElevateError::NoAdminPrompt(
            ERR_NO_ADMIN_PROMPT_MSG.to_string(),
        ));
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = env::temp_dir().join(format!("hiddify-sudo-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| {
        ElevateError::Other(format!("could not ask for the administrator password: {e}"))
    })?;
    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _dir_guard = TempDirGuard(dir.clone());

    let status_path = dir.join("status");
    let status_c = std::ffi::CString::new(status_path.to_string_lossy().as_bytes())
        .map_err(|e| ElevateError::Other(e.to_string()))?;
    if unsafe { libc::mkfifo(status_c.as_ptr(), 0o600) } != 0 {
        let e = std::io::Error::last_os_error();
        return Err(ElevateError::Other(format!(
            "could not ask for the administrator password: {e}"
        )));
    }

    let quoted: Vec<String> = args.iter().map(|a| shell_quote(a)).collect();
    let err_file = dir.join("error");
    let status_q = shell_quote(&status_path.to_string_lossy());
    let err_q = shell_quote(&err_file.to_string_lossy());
    let joined_args = quoted.join(" ");
    let script = format!(
        "#!/bin/sh\nexec 3>{status_q}\nprintf '\\033[1m%s\\033[0m\\n\\n' 'Hiddify needs root access for VPN mode'\nif ! sudo -v -p '[sudo] password for %u: '; then\n\techo refused >&3\n\texit 1\nfi\nsudo -n -- {joined_args} 2>{err_q}\ncode=$?\necho \"$code\" >&3\nexec 3>&-\nif [ \"$code\" != 0 ]; then\n\tcat {err_q}\n\tprintf '\\nPress Enter to close.'\n\tread -r _\nfi\n"
    );
    let script_path = dir.join("run");
    write_executable_script(&script_path, &script).map_err(|e| {
        ElevateError::Other(format!("could not ask for the administrator password: {e}"))
    })?;

    let status_reader_path = status_path.clone();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    thread::spawn(move || {
        let content = fs::read_to_string(&status_reader_path)
            .unwrap_or_default()
            .trim()
            .to_string();
        let _ = tx.send(content);
    });

    let mut cmd = Command::new(&terminal);
    cmd.args(&terminal_args).arg(&script_path);
    let mut child = cmd
        .spawn()
        .map_err(|_| ElevateError::NoAdminPrompt(ERR_NO_ADMIN_PROMPT_MSG.to_string()))?;

    let timeout = Duration::from_secs(5 * 60);
    let start = Instant::now();
    let got = loop {
        if let Ok(res) = rx.try_recv() {
            break res;
        }
        if let Ok(Some(exit_status)) = child.try_wait() {
            if !exit_status.success() {
                match rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(res) => break res,
                    Err(_) => {
                        unblock_fifo(&status_path);
                        return Err(ElevateError::NoAdminPrompt(
                            ERR_NO_ADMIN_PROMPT_MSG.to_string(),
                        ));
                    }
                }
            } else {
                let rem = timeout.saturating_sub(start.elapsed());
                match rx.recv_timeout(rem) {
                    Ok(res) => break res,
                    Err(_) => {
                        unblock_fifo(&status_path);
                        return Err(ElevateError::AdminPromptRefused(
                            ERR_ADMIN_PROMPT_REFUSED_MSG.to_string(),
                        ));
                    }
                }
            }
        }
        if start.elapsed() >= timeout {
            unblock_fifo(&status_path);
            return Err(ElevateError::AdminPromptRefused(
                ERR_ADMIN_PROMPT_REFUSED_MSG.to_string(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    };

    match got.as_str() {
        "0" => Ok(()),
        "" | "refused" => Err(ElevateError::AdminPromptRefused(
            ERR_ADMIN_PROMPT_REFUSED_MSG.to_string(),
        )),
        other => {
            let code = other.parse::<i32>().unwrap_or(1);
            let detail = fs::read_to_string(&err_file)
                .unwrap_or_default()
                .trim()
                .to_string();
            if !detail.is_empty() {
                Err(ElevateError::Other(format!(
                    "could not install the VPN service (exit status {code}): {detail}"
                )))
            } else {
                Err(ElevateError::Other(format!(
                    "could not install the VPN service (exit status {code})"
                )))
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn unblock_fifo(path: &Path) {
    if let Ok(c_path) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
        unsafe {
            let fd = libc::open(c_path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK);
            if fd >= 0 {
                let _ = libc::close(fd);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn run_elevated(
    cmd_and_args: &[String],
    owner: &str,
    extra_env: Option<(&str, &str)>,
    refused: Option<fn(i32, &str) -> bool>,
) -> Result<(), ElevateError> {
    let mut cmd = Command::new(&cmd_and_args[0]);
    cmd.args(&cmd_and_args[1..]);
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::piped());
    if !owner.is_empty() {
        cmd.env("HIDDIFY_TUNNEL_OWNER", owner);
    }
    if let Some((k, v)) = extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output().map_err(|e| {
        ElevateError::Other(format!("could not install the VPN service ({e})"))
    })?;
    if output.status.success() {
        return Ok(());
    }
    let code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    if let Some(check_refused) = refused {
        if check_refused(code, &stderr) {
            return Err(ElevateError::AdminPromptRefused(
                ERR_ADMIN_PROMPT_REFUSED_MSG.to_string(),
            ));
        }
    }
    let detail = stderr.trim();
    if !detail.is_empty() {
        Err(ElevateError::Other(format!(
            "could not install the VPN service (exit status {code}): {detail}"
        )))
    } else {
        Err(ElevateError::Other(format!(
            "could not install the VPN service (exit status {code})"
        )))
    }
}

// ============================================================================
// 5. Linux AppImage Root Foreground Service Lifecycle (`appimage_service.dart`)
// ============================================================================

pub const APPIMAGE_SERVICE_PORT: u16 = 18021;
const APPIMAGE_READY_LINE: &str = "HIDDIFY_SERVICE_READY";
const APPIMAGE_ERROR_PREFIX: &str = "HIDDIFY_CORE_ERROR ";
const APPIMAGE_READY_TIMEOUT: Duration = Duration::from_secs(300);
const APPIMAGE_STOP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppImageServiceStartOutcome {
    /// `"ok"`, `"no_admin_prompt"`, `"no_admin_agent"`, `"permission_refused"`,
    /// `"core_error"`, or `"service_error"`.
    pub status: String,
    pub handle: u64,
    pub secret: String,
    pub port: u16,
    pub message: Option<String>,
}

struct AppImageServiceEntry {
    child: Mutex<std::process::Child>,
    stdin: Mutex<Option<std::process::ChildStdin>>,
    exited: std::sync::atomic::AtomicBool,
}

fn appimage_services() -> &'static Mutex<std::collections::HashMap<u64, std::sync::Arc<AppImageServiceEntry>>> {
    static MAP: OnceLock<Mutex<std::collections::HashMap<u64, std::sync::Arc<AppImageServiceEntry>>>> =
        OnceLock::new();
    MAP.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn generate_appimage_secret() -> String {
    use rand::distributions::Uniform;
    use rand::rngs::OsRng;
    use rand::Rng;

    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let dist = Uniform::from(0..CHARS.len());
    let mut rng = OsRng;
    (0..64).map(|_| CHARS[rng.sample(dist)] as char).collect()
}

/// Classifies an early exit of the `pkexec` / `sudo` AppImage VPN service before `HIDDIFY_SERVICE_READY`,
/// matching lines 88-100 of `appimage_service.dart` 100%.
pub fn classify_appimage_early_exit(via_sudo: bool, code: i32, stderr_raw: &str) -> (&'static str, String) {
    let detail = stderr_raw.trim();
    if via_sudo && detail.contains("a password is required") {
        return ("no_admin_prompt", ERR_NO_ADMIN_PROMPT_MSG.to_string());
    }
    if !via_sudo && detail.contains("textual authentication agent") {
        return ("no_admin_agent", ERR_NO_ADMIN_AGENT_MSG.to_string());
    }
    let refused = if via_sudo {
        code == 1
            && (detail.contains("no password was provided")
                || detail.contains("incorrect password"))
    } else {
        code == 126 || (code == 127 && detail.is_empty())
    };
    if refused {
        return (
            "permission_refused",
            ERR_ADMIN_PROMPT_REFUSED_MSG.to_string(),
        );
    }
    (
        "service_error",
        format!("The VPN service did not start ({code}): {detail}"),
    )
}

#[cfg(not(target_os = "windows"))]
pub fn appimage_service_start(appimage_path: &str, owner_pid: i32) -> AppImageServiceStartOutcome {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread;

    let base_args = vec![
        appimage_path.to_string(),
        "HiddifyCli".to_string(),
        "core-service".to_string(),
        "--listen".to_string(),
        format!("127.0.0.1:{APPIMAGE_SERVICE_PORT}"),
        "--owner-pid".to_string(),
        owner_pid.to_string(),
    ];

    let mut via_sudo = false;
    let mut cleanup_dir: Option<PathBuf> = None;
    let mut cmd = if look_path("pkexec").is_some() {
        let mut c = Command::new("pkexec");
        c.args(&base_args);
        c
    } else {
        if look_path("sudo").is_none() {
            return AppImageServiceStartOutcome {
                status: "no_admin_prompt".to_string(),
                handle: 0,
                secret: String::new(),
                port: APPIMAGE_SERVICE_PORT,
                message: Some(ERR_NO_ADMIN_PROMPT_MSG.to_string()),
            };
        }
        via_sudo = true;
        let (askpass, dir_opt) = match sudo_askpass() {
            Ok(pair) => pair,
            Err(err) => {
                return AppImageServiceStartOutcome {
                    status: "service_error".to_string(),
                    handle: 0,
                    secret: String::new(),
                    port: APPIMAGE_SERVICE_PORT,
                    message: Some(err.message().to_string()),
                };
            }
        };
        cleanup_dir = dir_opt;
        if askpass.is_empty() {
            let mut c = Command::new("sudo");
            c.arg("-n").args(&base_args);
            c
        } else {
            let mut c = Command::new("sudo");
            c.arg("-A")
                .args(&base_args)
                .env("SUDO_ASKPASS", &askpass);
            c
        }
    };

    struct AskpassCleanup(Option<PathBuf>);
    impl Drop for AskpassCleanup {
        fn drop(&mut self) {
            if let Some(ref d) = self.0 {
                let _ = fs::remove_dir_all(d);
            }
        }
    }
    let _askpass_guard = AskpassCleanup(cleanup_dir);

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return AppImageServiceStartOutcome {
                status: "service_error".to_string(),
                handle: 0,
                secret: String::new(),
                port: APPIMAGE_SERVICE_PORT,
                message: Some(format!("could not spawn AppImage VPN service: {e}")),
            };
        }
    };

    let secret = generate_appimage_secret();
    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return AppImageServiceStartOutcome {
                status: "service_error".to_string(),
                handle: 0,
                secret: String::new(),
                port: APPIMAGE_SERVICE_PORT,
                message: Some("missing child stdin pipe".to_string()),
            };
        }
    };

    if writeln!(stdin, "{secret}").is_err() || stdin.flush().is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return AppImageServiceStartOutcome {
            status: "service_error".to_string(),
            handle: 0,
            secret: String::new(),
            port: APPIMAGE_SERVICE_PORT,
            message: Some("failed writing control secret to service stdin".to_string()),
        };
    }

    enum HandshakeSignal {
        Ready,
        CoreError(String),
        StdoutClosed,
    }

    let (tx, rx) = mpsc::channel::<HandshakeSignal>();
    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut sent_terminal = false;
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if !sent_terminal && trimmed == APPIMAGE_READY_LINE {
                    sent_terminal = true;
                    let _ = tx.send(HandshakeSignal::Ready);
                } else if !sent_terminal && line.starts_with(APPIMAGE_ERROR_PREFIX) {
                    sent_terminal = true;
                    let _ = tx.send(HandshakeSignal::CoreError(
                        line[APPIMAGE_ERROR_PREFIX.len()..].to_string(),
                    ));
                }
            }
            if !sent_terminal {
                let _ = tx.send(HandshakeSignal::StdoutClosed);
            }
        });
    }

    let stderr_buf = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let sb = stderr_buf.clone();
        thread::spawn(move || {
            let mut r = stderr;
            let mut tmp = [0u8; 2048];
            while let Ok(n) = r.read(&mut tmp) {
                if n == 0 {
                    break;
                }
                if let Ok(mut s) = sb.lock() {
                    if s.len() < 16 * 1024 {
                        s.push_str(&String::from_utf8_lossy(&tmp[..n]));
                    }
                }
            }
        });
    }

    let deadline = Instant::now() + APPIMAGE_READY_TIMEOUT;
    loop {
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return AppImageServiceStartOutcome {
                status: "service_error".to_string(),
                handle: 0,
                secret: String::new(),
                port: APPIMAGE_SERVICE_PORT,
                message: Some("Timed out waiting for AppImage VPN service".to_string()),
            };
        }

        let wait_step = (deadline - now).min(Duration::from_millis(50));
        match rx.recv_timeout(wait_step) {
            Ok(HandshakeSignal::Ready) => {
                static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
                let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
                let entry = Arc::new(AppImageServiceEntry {
                    child: Mutex::new(child),
                    stdin: Mutex::new(Some(stdin)),
                    exited: AtomicBool::new(false),
                });
                if let Ok(mut map) = appimage_services().lock() {
                    map.insert(handle, entry);
                }
                return AppImageServiceStartOutcome {
                    status: "ok".to_string(),
                    handle,
                    secret,
                    port: APPIMAGE_SERVICE_PORT,
                    message: None,
                };
            }
            Ok(HandshakeSignal::CoreError(msg)) => {
                let _ = child.kill();
                let _ = child.wait();
                return AppImageServiceStartOutcome {
                    status: "core_error".to_string(),
                    handle: 0,
                    secret: String::new(),
                    port: APPIMAGE_SERVICE_PORT,
                    message: Some(msg),
                };
            }
            Ok(HandshakeSignal::StdoutClosed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let code = child
                    .wait()
                    .ok()
                    .and_then(|st| st.code())
                    .unwrap_or(1);
                // Give stderr reader a brief moment to flush final message
                thread::sleep(Duration::from_millis(20));
                let detail = stderr_buf.lock().map(|s| s.clone()).unwrap_or_default();
                let (status, msg) = classify_appimage_early_exit(via_sudo, code, &detail);
                return AppImageServiceStartOutcome {
                    status: status.to_string(),
                    handle: 0,
                    secret: String::new(),
                    port: APPIMAGE_SERVICE_PORT,
                    message: Some(msg),
                };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(exit_status)) = child.try_wait() {
                    let code = exit_status.code().unwrap_or(1);
                    thread::sleep(Duration::from_millis(20));
                    let detail = stderr_buf.lock().map(|s| s.clone()).unwrap_or_default();
                    let (status, msg) = classify_appimage_early_exit(via_sudo, code, &detail);
                    return AppImageServiceStartOutcome {
                        status: status.to_string(),
                        handle: 0,
                        secret: String::new(),
                        port: APPIMAGE_SERVICE_PORT,
                        message: Some(msg),
                    };
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub fn appimage_service_start(_appimage_path: &str, _owner_pid: i32) -> AppImageServiceStartOutcome {
    AppImageServiceStartOutcome {
        status: "service_error".to_string(),
        handle: 0,
        secret: String::new(),
        port: APPIMAGE_SERVICE_PORT,
        message: Some("AppImageService is only supported on Linux".to_string()),
    }
}

pub fn appimage_service_is_running(handle: u64) -> bool {
    use std::sync::atomic::Ordering;
    let Some(entry) = appimage_services()
        .lock()
        .ok()
        .and_then(|m| m.get(&handle).cloned())
    else {
        return false;
    };
    if entry.exited.load(Ordering::Acquire) {
        return false;
    }
    if let Ok(mut child) = entry.child.lock() {
        if let Ok(Some(_)) = child.try_wait() {
            entry.exited.store(true, Ordering::Release);
            return false;
        }
        return true;
    }
    false
}

pub fn appimage_service_stop(handle: u64) -> bool {
    use std::sync::atomic::Ordering;
    let Some(entry) = appimage_services()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&handle))
    else {
        return false;
    };
    if entry.exited.load(Ordering::Acquire) {
        return true;
    }
    // Closing stdin tells `HiddifyCli core-service` to restore TUN/routes and exit cleanly
    if let Ok(mut stdin_opt) = entry.stdin.lock() {
        drop(stdin_opt.take());
    }
    let deadline = Instant::now() + APPIMAGE_STOP_TIMEOUT;
    if let Ok(mut child) = entry.child.lock() {
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    entry.exited.store(true, Ordering::Release);
                    return true;
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        entry.exited.store(true, Ordering::Release);
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    entry.exited.store(true, Ordering::Release);
                    return true;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpn_mode_and_admin_needed_features_match_go_rules() {
        assert!(is_vpn_service_mode("vpn", "{}"));
        assert!(is_vpn_service_mode("TUN", "{}"));
        assert!(is_vpn_service_mode("router", "{}"));
        assert!(!is_vpn_service_mode("proxy", r#"{"enable-tun":true}"#));
        assert!(!is_vpn_service_mode("system-proxy", r#"{"enable-tun":true}"#));
        assert!(is_vpn_service_mode("", r#"{"router-mode":true}"#));
        assert!(is_vpn_service_mode("", r#"{"enable-tun":true}"#));
        assert!(is_vpn_service_mode(
            "",
            r#"{"inbound_options":{"enable_tun":true}}"#
        ));
        assert!(is_vpn_service_mode(
            "",
            r#"{"inbound_options":{"enableTun":true}}"#
        ));
        assert!(!is_vpn_service_mode("", "{}"));

        let features =
            scan_admin_needed_features(r#"{"tls-tricks":{"mixed-sni-case":true}}"#);
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].tag, "tls-tricks");
        assert_eq!(features[0].summary, "Mixed SNI case");

        // NeedAdminBgService matrix
        let req_vpn = NeedAdminBgServiceRequest {
            service_mode: "vpn".to_string(),
            hiddify_settings_json: "{}".to_string(),
            admin_preference: 0,
            privilege_error_remembered: false,
            cli_path: None,
            build_id: None,
        };
        assert_eq!(
            need_admin_bg_service_with(&req_vpn, true, false).decision,
            AdminBgServiceDecision::No.as_i32()
        );
        assert_eq!(
            need_admin_bg_service_with(&req_vpn, false, false).decision,
            AdminBgServiceDecision::NeedsAdmin.as_i32()
        );
        assert_eq!(
            need_admin_bg_service_with(&req_vpn, false, true).decision,
            AdminBgServiceDecision::AlreadyConnected.as_i32()
        );

        let req_opt = NeedAdminBgServiceRequest {
            service_mode: "proxy".to_string(),
            hiddify_settings_json: r#"{"tls-tricks":{"mixedSniCase":true}}"#.to_string(),
            admin_preference: AdminPreference::Unset as i32,
            privilege_error_remembered: false,
            cli_path: None,
            build_id: None,
        };
        let out_unset = need_admin_bg_service_with(&req_opt, false, false);
        assert_eq!(
            out_unset.decision,
            AdminBgServiceDecision::OptionalProxyNeedAdmin.as_i32()
        );
        assert_eq!(out_unset.admin_needed_proxies.len(), 1);
    }

    #[test]
    fn windows_quoting_and_linux_elevation_classifiers_match_go() {
        assert_eq!(quote_windows_arg(""), "\"\"");
        assert_eq!(quote_windows_arg("alice"), "alice");
        assert_eq!(quote_windows_arg("alice smith"), "\"alice smith\"");
        assert_eq!(quote_windows_arg("a\"b"), "\"a\\\"b\"");

        assert_eq!(shell_word("/usr/bin/HiddifyCli"), "/usr/bin/HiddifyCli");
        assert_eq!(
            shell_word("/opt/Hiddify App/HiddifyCli"),
            "'/opt/Hiddify App/HiddifyCli'"
        );

        assert!(pkexec_refused(126, ""));
        assert!(!pkexec_refused(127, ""));
        assert!(pkexec_has_no_agent(
            "Error creating textual authentication agent: Error opening current controlling terminal"
        ));
        assert!(sudo_refused(1, "sudo: no password was provided"));
        assert!(sudo_refused(1, "Sorry, try again.\nsudo: 3 incorrect password attempts"));
        assert!(!sudo_refused(1, "failed to load library"));

        assert!(is_permission_denied_message(ERR_ADMIN_PROMPT_REFUSED_MSG));
        assert!(is_permission_denied_message("Request dismissed by Polkit"));
        assert!(!is_permission_denied_message(ERR_NO_ADMIN_PROMPT_MSG));
        assert!(!is_permission_denied_message(ERR_NO_ADMIN_AGENT_MSG));

        // AppImage early-exit classifier (`appimage_service.dart` parity)
        assert_eq!(
            classify_appimage_early_exit(false, 126, "").0,
            "permission_refused"
        );
        assert_eq!(
            classify_appimage_early_exit(false, 127, "").0,
            "permission_refused"
        );
        assert_eq!(
            classify_appimage_early_exit(false, 127, "pkexec: command not executable").0,
            "service_error"
        );
        assert_eq!(
            classify_appimage_early_exit(
                false,
                127,
                "Error creating textual authentication agent"
            )
            .0,
            "no_admin_agent"
        );
        assert_eq!(
            classify_appimage_early_exit(true, 1, "sudo: a password is required").0,
            "no_admin_prompt"
        );
        assert_eq!(
            classify_appimage_early_exit(true, 1, "sudo: no password was provided").0,
            "permission_refused"
        );
        assert_eq!(generate_appimage_secret().len(), 64);
    }
}
