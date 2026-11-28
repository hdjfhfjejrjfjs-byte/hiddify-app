//! Desktop `CoreProcess` child-process supervisor (`lib/hiddifycore/core_interface/core_process.dart`
//! and `lib/hiddifycore/core_interface/windows_crash.dart`).
//!
//! Spawns `<launcher> core ...`, passes `HIDDIFY_CORE_SECRET` via child environment (never argv),
//! holds `stdin` open as a liveness pipe, drains `stdout` (`HIDDIFY_CORE_READY` / `HIDDIFY_CORE_ERROR`)
//! and `stderr` (50-line ring buffer) on dedicated OS threads so pipes never block, enforces a 60s
//! ready timeout, extracts Go/Windows crash reasons on early or unexpected exit, and performs
//! graceful `stdin`-close shutdown with a 10s `SIGKILL` / `TerminateProcess` fallback.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hiddify_diagnostics::core_guard::{
    core_crash_reason_of, describe_windows_exit_code, parse_core_stdout_line,
    windows_application_error_for, CoreStdoutLine,
};
use serde::{Deserialize, Serialize};

const SECRET_ENV: &str = "HIDDIFY_CORE_SECRET";
const DEFAULT_READY_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_STOP_TIMEOUT_MS: u64 = 10_000;
const STDERR_TAIL_LINES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreProcessStartRequest {
    pub executable: String,
    pub base_dir: String,
    pub working_dir: String,
    pub temp_dir: String,
    pub listen: String,
    pub mode: i32,
    /// Optional symbolic `SetupMode` name (`"GRPC_NORMAL"`, `"GRPC_NORMAL_INSECURE"`, etc.)
    /// matching `<workingDir>/data/stderr<modeName>.log`.
    #[serde(default)]
    pub mode_name: String,
    pub secret: String,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub owner_pid: u32,
    #[serde(default)]
    pub ready_timeout_ms: u64,
    #[serde(default)]
    pub stop_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreProcessStartOutcome {
    /// `"ok"`, `"core_error"`, `"timeout"`, or `"spawn_error"`.
    pub status: String,
    pub handle: u64,
    pub pid: u32,
    pub certificate_pem: String,
    pub message: Option<String>,
}

#[derive(Clone)]
enum HandshakeState {
    Pending,
    Ready(String),
    CoreError(String),
}

struct CoreProcessInner {
    pid: u32,
    started_at_ms: i64,
    crash_file_path: String,
    stop_timeout_ms: u64,
    stopping: AtomicBool,
    stdin: Mutex<Option<ChildStdin>>,
    stderr_tail: Mutex<(VecDeque<String>, bool)>,
    stderr_cv: Condvar,
    handshake: Mutex<HandshakeState>,
    handshake_cv: Condvar,
    exit_code: Mutex<Option<i32>>,
    exit_cv: Condvar,
}

fn next_handle() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn process_registry() -> &'static Mutex<HashMap<u64, Arc<CoreProcessInner>>> {
    static REG: OnceLock<Mutex<HashMap<u64, Arc<CoreProcessInner>>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolves the desktop core launcher next to `resolved_executable`, matching `CoreProcess.executablePath()`.
pub fn resolve_core_executable_path(resolved_executable: &str) -> Option<String> {
    if std::env::var_os("FLUTTER_TEST").is_some() {
        return None;
    }
    let exe_path = Path::new(resolved_executable);
    let dir = exe_path.parent()?;

    let candidate: PathBuf = if cfg!(target_os = "windows") {
        dir.join("HiddifyCoreEngine.exe")
    } else if cfg!(target_os = "macos") {
        lexical_normalize(
            &dir.join("..")
                .join("Helpers")
                .join("Hiddify Core.app")
                .join("Contents")
                .join("MacOS")
                .join("Hiddify"),
        )
    } else if cfg!(target_os = "linux") {
        dir.join("HiddifyCli")
    } else {
        return None;
    };

    if candidate.is_file() {
        Some(candidate.to_string_lossy().into_owned())
    } else {
        None
    }
}

fn setup_mode_name(mode: i32, explicit: &str) -> String {
    if !explicit.trim().is_empty() {
        return explicit.trim().to_string();
    }
    match mode {
        0 => "OLD".to_string(),
        1 => "GRPC_NORMAL".to_string(),
        2 => "GRPC_BACKGROUND".to_string(),
        3 => "GRPC_NORMAL_INSECURE".to_string(),
        4 => "GRPC_BACKGROUND_INSECURE".to_string(),
        other => other.to_string(),
    }
}

/// Starts the desktop core process and blocks until `HIDDIFY_CORE_READY`, early exit, or timeout.
pub fn core_process_start(req: &CoreProcessStartRequest) -> CoreProcessStartOutcome {
    let started_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64;

    let owner_pid = if req.owner_pid != 0 {
        req.owner_pid
    } else {
        std::process::id()
    };
    let ready_timeout = Duration::from_millis(if req.ready_timeout_ms > 0 {
        req.ready_timeout_ms
    } else {
        DEFAULT_READY_TIMEOUT_MS
    });
    let stop_timeout_ms = if req.stop_timeout_ms > 0 {
        req.stop_timeout_ms
    } else {
        DEFAULT_STOP_TIMEOUT_MS
    };

    let mut args = vec![
        "core".to_string(),
        "--base-dir".to_string(),
        req.base_dir.clone(),
        "--working-dir".to_string(),
        req.working_dir.clone(),
        "--temp-dir".to_string(),
        req.temp_dir.clone(),
        "--listen".to_string(),
        req.listen.clone(),
        "--mode".to_string(),
        req.mode.to_string(),
        "--owner-pid".to_string(),
        owner_pid.to_string(),
    ];
    if req.debug {
        args.push("--debug".to_string());
    }

    let mut cmd = Command::new(&req.executable);
    cmd.args(&args)
        .env(SECRET_ENV, &req.secret)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(work_dir) = Path::new(&req.executable)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        cmd.current_dir(work_dir);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return CoreProcessStartOutcome {
                status: "spawn_error".to_string(),
                handle: 0,
                pid: 0,
                certificate_pem: String::new(),
                message: Some(err.to_string()),
            };
        }
    };

    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mode_label = setup_mode_name(req.mode, &req.mode_name);
    let crash_file_path = Path::new(&req.working_dir)
        .join("data")
        .join(format!("stderr{mode_label}.log"))
        .to_string_lossy()
        .into_owned();

    let inner = Arc::new(CoreProcessInner {
        pid,
        started_at_ms,
        crash_file_path,
        stop_timeout_ms,
        stopping: AtomicBool::new(false),
        stdin: Mutex::new(stdin),
        stderr_tail: Mutex::new((VecDeque::with_capacity(STDERR_TAIL_LINES), false)),
        stderr_cv: Condvar::new(),
        handshake: Mutex::new(HandshakeState::Pending),
        handshake_cv: Condvar::new(),
        exit_code: Mutex::new(None),
        exit_cv: Condvar::new(),
    });

    // Drain stdout completely until EOF so a full pipe never blocks the core.
    if let Some(out_pipe) = stdout {
        let st = inner.clone();
        thread::spawn(move || {
            let reader = BufReader::new(out_pipe);
            for line in reader.lines().map_while(Result::ok) {
                match parse_core_stdout_line(&line) {
                    CoreStdoutLine::Ready { certificate_pem } => {
                        if let Ok(mut hs) = st.handshake.lock() {
                            if matches!(*hs, HandshakeState::Pending) {
                                *hs = HandshakeState::Ready(certificate_pem);
                                st.handshake_cv.notify_all();
                            }
                        }
                    }
                    CoreStdoutLine::Error { message } => {
                        if let Ok(mut hs) = st.handshake.lock() {
                            if matches!(*hs, HandshakeState::Pending) {
                                *hs = HandshakeState::CoreError(message);
                                st.handshake_cv.notify_all();
                            }
                        }
                    }
                    CoreStdoutLine::Log { .. } | CoreStdoutLine::Empty => {}
                }
            }
        });
    }

    // Drain stderr completely until EOF, retaining the last 50 non-empty lines.
    if let Some(err_pipe) = stderr {
        let st = inner.clone();
        thread::spawn(move || {
            let reader = BufReader::new(err_pipe);
            for line in reader.lines().map_while(Result::ok) {
                if line.is_empty() {
                    continue;
                }
                if let Ok(mut guard) = st.stderr_tail.lock() {
                    if guard.0.len() >= STDERR_TAIL_LINES {
                        guard.0.pop_front();
                    }
                    guard.0.push_back(line);
                }
            }
            if let Ok(mut guard) = st.stderr_tail.lock() {
                guard.1 = true;
                st.stderr_cv.notify_all();
            }
        });
    }

    // Reap child exit status and resolve early-exit crash reason if not yet ready.
    {
        let st = inner.clone();
        thread::spawn(move || {
            let code = match child.wait() {
                Ok(status) => status.code().unwrap_or(-1),
                Err(_) => -1,
            };
            if let Ok(mut ec) = st.exit_code.lock() {
                *ec = Some(code);
                st.exit_cv.notify_all();
            }

            let still_pending = st
                .handshake
                .lock()
                .map(|hs| matches!(*hs, HandshakeState::Pending))
                .unwrap_or(false);
            if !still_pending {
                return;
            }

            // Wait up to 1s for remaining stderr lines arriving right after exit.
            if let Ok(mut guard) = st.stderr_tail.lock() {
                let deadline = Instant::now() + Duration::from_secs(1);
                while !guard.1 {
                    let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                        break;
                    };
                    let Ok((next_guard, _)) = st.stderr_cv.wait_timeout(guard, rem) else {
                        break;
                    };
                    guard = next_guard;
                }
            }

            let reason = compute_crash_reason_from_inner(&st, false);
            if let Ok(mut hs) = st.handshake.lock() {
                if matches!(*hs, HandshakeState::Pending) {
                    let msg = match reason {
                        Some(r) => format!("the core exited ({code}) before it was ready: {r}"),
                        None => format!("the core exited ({code}) before it was ready"),
                    };
                    *hs = HandshakeState::CoreError(msg);
                    st.handshake_cv.notify_all();
                }
            }
        });
    }

    // Wait up to `ready_timeout` for handshake completion.
    let deadline = Instant::now() + ready_timeout;
    let final_hs = {
        let Ok(mut guard) = inner.handshake.lock() else {
            force_kill_pid(pid);
            return CoreProcessStartOutcome {
                status: "core_error".to_string(),
                handle: 0,
                pid,
                certificate_pem: String::new(),
                message: Some("internal lock error".to_string()),
            };
        };
        loop {
            if !matches!(*guard, HandshakeState::Pending) {
                break guard.clone();
            }
            let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                break HandshakeState::Pending;
            };
            let Ok((next_guard, _)) = inner.handshake_cv.wait_timeout(guard, rem) else {
                break HandshakeState::Pending;
            };
            guard = next_guard;
        }
    };

    match final_hs {
        HandshakeState::Ready(certificate_pem) => {
            let handle = next_handle();
            if let Ok(mut reg) = process_registry().lock() {
                reg.insert(handle, inner);
            }
            CoreProcessStartOutcome {
                status: "ok".to_string(),
                handle,
                pid,
                certificate_pem,
                message: None,
            }
        }
        HandshakeState::CoreError(msg) => {
            force_kill_pid(pid);
            CoreProcessStartOutcome {
                status: "core_error".to_string(),
                handle: 0,
                pid,
                certificate_pem: String::new(),
                message: Some(msg),
            }
        }
        HandshakeState::Pending => {
            force_kill_pid(pid);
            CoreProcessStartOutcome {
                status: "timeout".to_string(),
                handle: 0,
                pid,
                certificate_pem: String::new(),
                message: Some(format!(
                    "TimeoutException after {}s: waiting for core process ready",
                    ready_timeout.as_secs()
                )),
            }
        }
    }
}

/// Blocks on a zero-CPU `Condvar` until the process exits and returns its exit code.
pub fn core_process_wait_exit(handle: u64) -> Option<i32> {
    let st = process_registry().lock().ok()?.get(&handle)?.clone();
    let mut guard = st.exit_code.lock().ok()?;
    while guard.is_none() {
        guard = st.exit_cv.wait(guard).ok()?;
    }
    *guard
}

pub fn core_process_is_stopping(handle: u64) -> bool {
    process_registry()
        .lock()
        .ok()
        .and_then(|reg| reg.get(&handle).map(|s| s.stopping.load(Ordering::Acquire)))
        .unwrap_or(false)
}

/// Extracts why the core process crashed (`crashReason()` in `core_process.dart`).
pub fn core_process_crash_reason(handle: u64) -> Option<String> {
    let st = process_registry().lock().ok()?.get(&handle)?.clone();
    compute_crash_reason_from_inner(&st, cfg!(target_os = "windows"))
}

/// Gracefully closes `stdin` so the core restores system proxy and exits, escalating to `SIGKILL`
/// after `stop_timeout_ms` (`10s`) if still alive, then removes `handle` (`CoreProcess.stop()`).
pub fn core_process_stop(handle: u64) -> bool {
    let Some(st) = process_registry()
        .lock()
        .ok()
        .and_then(|reg| reg.get(&handle).cloned())
    else {
        return false;
    };

    if st.stopping.swap(true, Ordering::AcqRel) {
        let _ = wait_for_exit_inner(&st, None);
        if let Ok(mut reg) = process_registry().lock() {
            reg.remove(&handle);
        }
        return true;
    }

    // Closing stdin tells hiddify-core to restore system proxy and shut down cleanly.
    if let Ok(mut stdin_guard) = st.stdin.lock() {
        let _ = stdin_guard.take();
    }

    if wait_for_exit_inner(&st, Some(Duration::from_millis(st.stop_timeout_ms))).is_none() {
        force_kill_pid(st.pid);
        let _ = wait_for_exit_inner(&st, Some(Duration::from_secs(5)));
    }

    if let Ok(mut reg) = process_registry().lock() {
        reg.remove(&handle);
    }
    true
}

/// Releases `handle` from the registry without killing (e.g. after observing unexpected exit + crash reason).
pub fn core_process_forget(handle: u64) -> bool {
    process_registry()
        .lock()
        .ok()
        .and_then(|mut reg| reg.remove(&handle))
        .is_some()
}

fn wait_for_exit_inner(st: &CoreProcessInner, timeout: Option<Duration>) -> Option<i32> {
    let mut guard = st.exit_code.lock().ok()?;
    match timeout {
        None => {
            while guard.is_none() {
                guard = st.exit_cv.wait(guard).ok()?;
            }
            *guard
        }
        Some(dur) => {
            let deadline = Instant::now() + dur;
            while guard.is_none() {
                let rem = deadline.checked_duration_since(Instant::now())?;
                let (next_guard, _) = st.exit_cv.wait_timeout(guard, rem).ok()?;
                guard = next_guard;
            }
            *guard
        }
    }
}

fn compute_crash_reason_from_inner(
    st: &CoreProcessInner,
    check_windows_event_log: bool,
) -> Option<String> {
    let joined_tail = st
        .stderr_tail
        .lock()
        .map(|g| g.0.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    if let Some(reason) =
        core_crash_reason_of(&st.crash_file_path, &joined_tail, st.started_at_ms)
    {
        return Some(reason);
    }
    if check_windows_event_log {
        let code = wait_for_exit_inner(st, Some(Duration::from_secs(2))).unwrap_or(-1);
        return windows_crash_reason(st.pid as i64, code as i64);
    }
    None
}

/// Queries Windows Event 1000 (`Application Error`) up to 4 times (500ms apart) before falling back
/// to NTSTATUS exit-code decoding (`WindowsCrash.reason` in `windows_crash.dart`).
pub fn windows_crash_reason(pid: i64, exit_code: i64) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        for attempt in 0..4 {
            if let Some(ev) = query_wevtutil_application_error(pid) {
                return Some(ev);
            }
            if attempt < 3 {
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
    let _ = pid;
    describe_windows_exit_code(exit_code)
}

#[cfg(target_os = "windows")]
fn query_wevtutil_application_error(pid: i64) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut child = Command::new("wevtutil")
        .args([
            "qe",
            "Application",
            r#"/q:*[System[Provider[@Name="Application Error"] and EventID=1000 and TimeCreated[timediff(@SystemTime) <= 120000]]]"#,
            "/f:xml",
            "/rd:true",
            "/c:10",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout_pipe = child.stdout.take()?;
    let reader_handle = thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout_pipe, &mut s);
        s
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait().ok()? {
            Some(status) => {
                if !status.success() {
                    return None;
                }
                let xml = reader_handle.join().ok()?;
                return windows_application_error_for(&xml, pid);
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn force_kill_pid(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    unsafe {
        if pid <= i32::MAX as u32 {
            let _ = libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    // Suppress unused import warning on non-Windows targets
    let _ = windows_application_error_for as fn(&str, i64) -> Option<String>;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use base64::Engine;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn write_mock_script(dir: &Path, name: &str, body: &str) -> String {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn ready_handshake_stdin_graceful_stop_early_panic_and_sigkill_fallback_match_dart() {
        let dir = tempfile::tempdir().unwrap();
        let cert_pem = "-----BEGIN CERTIFICATE-----\nTEST\n-----END CERTIFICATE-----\n";
        let cert_b64 = base64::engine::general_purpose::STANDARD.encode(cert_pem.as_bytes());

        // 1. Normal startup + secret env verification + graceful exit on stdin EOF
        let ok_exe = write_mock_script(
            dir.path(),
            "mock_core_ok.sh",
            &format!(
                "#!/bin/sh\n\
                 if [ \"$HIDDIFY_CORE_SECRET\" != \"topsecret\" ]; then\n\
                   echo \"HIDDIFY_CORE_ERROR wrong secret\"\n\
                   exit 1\n\
                 fi\n\
                 echo \"HIDDIFY_CORE_READY {cert_b64}\"\n\
                 cat >/dev/null\n\
                 exit 0\n"
            ),
        );

        let out = core_process_start(&CoreProcessStartRequest {
            executable: ok_exe,
            base_dir: dir.path().to_string_lossy().into_owned(),
            working_dir: dir.path().to_string_lossy().into_owned(),
            temp_dir: dir.path().to_string_lossy().into_owned(),
            listen: "127.0.0.1:17078".to_string(),
            mode: 1,
            mode_name: "GRPC_NORMAL".to_string(),
            secret: "topsecret".to_string(),
            debug: true,
            owner_pid: 0,
            ready_timeout_ms: 5_000,
            stop_timeout_ms: 2_000,
        });
        assert_eq!(out.status, "ok");
        assert!(out.handle > 0);
        assert_eq!(out.certificate_pem, cert_pem);
        assert!(!core_process_is_stopping(out.handle));

        let handle = out.handle;
        let waiter = thread::spawn(move || core_process_wait_exit(handle));
        thread::sleep(Duration::from_millis(50));
        assert!(core_process_stop(out.handle));
        assert_eq!(waiter.join().unwrap(), Some(0));

        // 2. Early Go panic before ready -> surfaces exact exit code + panic headline
        let panic_exe = write_mock_script(
            dir.path(),
            "mock_core_panic.sh",
            "#!/bin/sh\n\
             echo \"panic: boom in init\" >&2\n\
             echo \"goroutine 1 [running]:\" >&2\n\
             exit 2\n",
        );
        let panic_out = core_process_start(&CoreProcessStartRequest {
            executable: panic_exe,
            base_dir: dir.path().to_string_lossy().into_owned(),
            working_dir: dir.path().to_string_lossy().into_owned(),
            temp_dir: dir.path().to_string_lossy().into_owned(),
            listen: "127.0.0.1:17078".to_string(),
            mode: 1,
            mode_name: "GRPC_NORMAL".to_string(),
            secret: "topsecret".to_string(),
            debug: false,
            owner_pid: 0,
            ready_timeout_ms: 5_000,
            stop_timeout_ms: 1_000,
        });
        assert_eq!(panic_out.status, "core_error");
        assert_eq!(
            panic_out.message.as_deref(),
            Some("the core exited (2) before it was ready: panic: boom in init")
        );

        // 3. Hung process ignoring stdin EOF is escalated to SIGKILL after stop_timeout_ms
        let hung_exe = write_mock_script(
            dir.path(),
            "mock_core_hung.sh",
            &format!(
                "#!/bin/sh\n\
                 echo \"HIDDIFY_CORE_READY {cert_b64}\"\n\
                 sleep 30\n"
            ),
        );
        let hung_out = core_process_start(&CoreProcessStartRequest {
            executable: hung_exe,
            base_dir: dir.path().to_string_lossy().into_owned(),
            working_dir: dir.path().to_string_lossy().into_owned(),
            temp_dir: dir.path().to_string_lossy().into_owned(),
            listen: "127.0.0.1:17078".to_string(),
            mode: 3,
            mode_name: "GRPC_NORMAL_INSECURE".to_string(),
            secret: "topsecret".to_string(),
            debug: false,
            owner_pid: 0,
            ready_timeout_ms: 5_000,
            stop_timeout_ms: 200,
        });
        assert_eq!(hung_out.status, "ok");
        let t0 = Instant::now();
        assert!(core_process_stop(hung_out.handle));
        assert!(t0.elapsed() < Duration::from_secs(3));
    }
}
