//! Thread-safe task cancellation and streaming progress registry for asynchronous
//! Rust downloads and network operations (`fetch_url`, `update_download`, `pipeline`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug)]
struct TaskEntry {
    cancelled: AtomicBool,
    received_bytes: AtomicU64,
    total_bytes: AtomicU64,
    finished: AtomicBool,
    wake_lock: Mutex<bool>,
    wake_cv: Condvar,
}

impl TaskEntry {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            received_bytes: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            wake_lock: Mutex::new(false),
            wake_cv: Condvar::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgressSnapshot {
    pub task_id: u64,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub cancelled: bool,
    pub finished: bool,
}

fn registry() -> &'static Mutex<HashMap<u64, Arc<TaskEntry>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u64, Arc<TaskEntry>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Allocates a fresh non-zero task ID and registers it in the task table.
pub fn create_task() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    ensure_task(id);
    id
}

/// Ensures `task_id` exists in the registry (no-op if `task_id == 0`).
pub fn ensure_task(task_id: u64) {
    if task_id == 0 {
        return;
    }
    if let Ok(mut map) = registry().lock() {
        map.entry(task_id).or_insert_with(|| Arc::new(TaskEntry::new()));
    }
}

fn get_entry(task_id: u64) -> Option<Arc<TaskEntry>> {
    if task_id == 0 {
        return None;
    }
    registry().lock().ok().and_then(|map| map.get(&task_id).cloned())
}

/// Marks `task_id` as cancelled and immediately wakes any thread sleeping on `wait_or_cancelled`.
pub fn cancel_task(task_id: u64) -> bool {
    if task_id == 0 {
        return false;
    }
    let entry = {
        let Ok(mut map) = registry().lock() else {
            return false;
        };
        map.entry(task_id)
            .or_insert_with(|| Arc::new(TaskEntry::new()))
            .clone()
    };
    entry.cancelled.store(true, Ordering::SeqCst);
    if let Ok(mut guard) = entry.wake_lock.lock() {
        *guard = true;
        entry.wake_cv.notify_all();
    }
    true
}

/// Reports whether `task_id` has been cancelled (`false` when `task_id == 0`).
pub fn is_cancelled(task_id: u64) -> bool {
    get_entry(task_id)
        .map(|e| e.cancelled.load(Ordering::SeqCst))
        .unwrap_or(false)
}

/// Sleeps for `duration`, waking early and returning `true` if `task_id` is cancelled.
pub fn wait_or_cancelled(task_id: u64, duration: Duration) -> bool {
    if task_id == 0 {
        std::thread::sleep(duration);
        return false;
    }
    ensure_task(task_id);
    let Some(entry) = get_entry(task_id) else {
        std::thread::sleep(duration);
        return false;
    };
    if entry.cancelled.load(Ordering::SeqCst) {
        return true;
    }
    let deadline = Instant::now() + duration;
    let Ok(mut guard) = entry.wake_lock.lock() else {
        std::thread::sleep(duration);
        return entry.cancelled.load(Ordering::SeqCst);
    };
    while !*guard && !entry.cancelled.load(Ordering::SeqCst) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let Ok((next_guard, _)) = entry.wake_cv.wait_timeout(guard, deadline - now) else {
            break;
        };
        guard = next_guard;
    }
    entry.cancelled.load(Ordering::SeqCst)
}

/// Updates streaming byte counters for `task_id`.
pub fn update_progress(task_id: u64, received_bytes: u64, total_bytes: u64) {
    if let Some(entry) = get_entry(task_id) {
        entry.received_bytes.store(received_bytes, Ordering::Relaxed);
        entry.total_bytes.store(total_bytes, Ordering::Relaxed);
    }
}

/// Marks `task_id` as finished while retaining its final snapshot until `remove_task`.
pub fn finish_task(task_id: u64) {
    if let Some(entry) = get_entry(task_id) {
        entry.finished.store(true, Ordering::Release);
    }
}

/// Reads the current progress/cancellation snapshot of `task_id`.
pub fn task_progress(task_id: u64) -> Option<TaskProgressSnapshot> {
    let entry = get_entry(task_id)?;
    Some(TaskProgressSnapshot {
        task_id,
        received_bytes: entry.received_bytes.load(Ordering::Relaxed),
        total_bytes: entry.total_bytes.load(Ordering::Relaxed),
        cancelled: entry.cancelled.load(Ordering::SeqCst),
        finished: entry.finished.load(Ordering::Acquire),
    })
}

/// Removes `task_id` from the global registry once the caller no longer needs its state.
pub fn remove_task(task_id: u64) {
    if task_id == 0 {
        return;
    }
    if let Ok(mut map) = registry().lock() {
        // Bound registry size defensively against leaked IDs (> 4096 finished entries)
        map.remove(&task_id);
        if map.len() > 4096 {
            map.retain(|_, v| !v.finished.load(Ordering::Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_wakes_waiter_immediately_and_tracks_progress() {
        let id = create_task();
        assert!(!is_cancelled(id));
        update_progress(id, 512, 2048);
        let snap = task_progress(id).expect("snapshot");
        assert_eq!(snap.received_bytes, 512);
        assert_eq!(snap.total_bytes, 2048);

        let waiter = std::thread::spawn(move || {
            let start = Instant::now();
            let cancelled = wait_or_cancelled(id, Duration::from_secs(5));
            (cancelled, start.elapsed())
        });
        std::thread::sleep(Duration::from_millis(30));
        assert!(cancel_task(id));
        let (cancelled, elapsed) = waiter.join().expect("join");
        assert!(cancelled);
        assert!(elapsed < Duration::from_secs(1), "elapsed={elapsed:?}");
        remove_task(id);
    }
}
