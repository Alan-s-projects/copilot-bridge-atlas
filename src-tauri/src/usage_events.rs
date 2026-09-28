//! Real-time refresh event module using statistics
//!
//! When new data is written to the `proxy_request_logs` table (agent log, session sync, archive, etc.),
//! Through this module, emit `usage-log-recorded` event to the front end, let UsageDashboard
//! Immediately invalidate the query cache without waiting for a polling cycle.
//!
//! Design points:
//! - Global singleton AppHandle: AppHandle is not held on the log writing path and is shared with OnceCell.
//! - 200ms anti-shake merging: Scenarios such as streaming response may write multiple logs in a short period of time.
//!   Merging into one event can avoid continuous invalidate on the front end.
//! - Non-blocking writes: Only warn logs are logged for notification failures, and errors are not propagated upward.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// The event name monitored by the front end
pub const EVENT_USAGE_LOG_RECORDED: &str = "usage-log-recorded";

/// Anti-shake window: merge multiple notifications within 200ms.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(200);

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Anti-shake flag: true indicates that there is already a scheduled task waiting for emit, and subsequent notifications will be merged into this task.
static EMIT_SCHEDULED: AtomicBool = AtomicBool::new(false);

/// Called once during the application setup phase to inject AppHandle.
///
/// Repeated calls are harmless (OnceLock only takes effect for the first write), but it should only be called during application startup.
/// `lib.rs::run` is called once.
pub fn init(handle: AppHandle) {
    if APP_HANDLE.set(handle).is_err() {
        log::debug!("usage_events::init is called repeatedly and ignored");
    } else {
        log::info!("[usage-event] AppHandle injected, event push enabled");
    }
}

/// Notify the front end that new usage logs are written.
///
/// The caller does not need to hold the AppHandle and can be called from any thread/any write path.
/// Internal 200ms anti-shake merging, never blocking the calling thread.
pub fn notify_log_recorded() {
    #[cfg(test)]
    TEST_NOTIFY_COUNT.with(|count| count.set(count.get().saturating_add(1)));

    // AppHandle is not injected (typically appears before unit testing or setup): give up directly.
    let Some(handle) = APP_HANDLE.get() else {
        return;
    };

    // Existing scheduled tasks: This notification is merged into the existing tasks, and there is no need to start another thread.
    if EMIT_SCHEDULED.swap(true, Ordering::AcqRel) {
        return;
    }

    let handle = handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(DEBOUNCE_WINDOW);
        // The flag must be cleared first before emitting: in case a new notification comes in during emit,
        // The next round of anti-shake windows will be rescheduled and will not be lost.
        EMIT_SCHEDULED.store(false, Ordering::Release);

        if let Err(e) = handle.emit(EVENT_USAGE_LOG_RECORDED, ()) {
            log::warn!("emit {EVENT_USAGE_LOG_RECORDED} failed: {e}");
        }
    });
}

#[cfg(test)]
thread_local! {
    static TEST_NOTIFY_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_test_notify_count() -> u32 {
    TEST_NOTIFY_COUNT.with(|count| count.replace(0))
}
