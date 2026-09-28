//! Panic Hook module
//!
//! Crash reports are stored under the application's own data directory.
//! It is convenient for users and developers to diagnose crash problems.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Application version number (read from Cargo.toml)
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CRASH_LOG_MAX_SIZE: u64 = 5 * 1024 * 1024;
const CRASH_LOG_ARCHIVES_TO_KEEP: usize = 2;

static APP_CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
static CRASH_LOG_LOCK: Mutex<()> = Mutex::new(());

pub fn init_app_config_dir(dir: PathBuf) {
    let _ = APP_CONFIG_DIR.set(dir);
}

/// Get the default application configuration directory (no panic)
fn default_app_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".copilot-bridge-atlas")
}

/// Get the application configuration directory (the value written during initialization will be used first; there will be no panic)
fn get_app_config_dir() -> PathBuf {
    APP_CONFIG_DIR
        .get()
        .cloned()
        .unwrap_or_else(default_app_config_dir)
}

/// Get crash log file path
fn get_crash_log_path() -> PathBuf {
    get_app_config_dir().join("crash.log")
}

fn rotated_crash_log_path(path: &Path, index: usize) -> PathBuf {
    let mut rotated = path.as_os_str().to_os_string();
    rotated.push(format!(".{index}"));
    PathBuf::from(rotated)
}

fn rotate_crash_log_if_needed_with_limit(
    path: &Path,
    max_size: u64,
    archives_to_keep: usize,
) -> std::io::Result<()> {
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if size < max_size || archives_to_keep == 0 {
        return Ok(());
    }

    for index in (1..=archives_to_keep).rev() {
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            rotated_crash_log_path(path, index - 1)
        };
        if !source.exists() {
            continue;
        }

        let destination = rotated_crash_log_path(path, index);
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(source, destination)?;
    }

    Ok(())
}

fn rotate_crash_log_if_needed(path: &Path) -> std::io::Result<()> {
    rotate_crash_log_if_needed_with_limit(path, CRASH_LOG_MAX_SIZE, CRASH_LOG_ARCHIVES_TO_KEEP)
}

/// Get log directory path
pub fn get_log_dir() -> PathBuf {
    get_app_config_dir().join("logs")
}

/// Get environment information safely (no panic)
fn get_system_info() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let family = std::env::consts::FAMILY;

    // Safely obtain the current working directory
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Safely obtain current thread information
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let thread_id = format!("{:?}", thread.id());

    format!(
        "OS: {os} ({family})\n\
         Arch: {arch}\n\
         App Version: {APP_VERSION}\n\
         Working Dir: {cwd}\n\
         Thread: {thread_name} (ID: {thread_id})"
    )
}

/// Set up a panic hook, capture crash information and write it to a log file
///
/// Call this function when the application starts to ensure that any panics are logged.
/// The log format contains:
/// - timestamp
/// - App version and system information
/// - Panic messages
/// - Location of occurrence (file: line number)
/// - Backtrace (complete call stack)
pub fn setup_panic_hook() {
    // Enable backtrace (make sure release mode also captures)
    if std::env::var("RUST_BACKTRACE").is_err() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        let log_path = get_crash_log_path();

        // Make sure the directory exists
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Construct crash information (use catch_unwind to protect time formatting and avoid nested panics)
        let timestamp = std::panic::catch_unwind(|| {
            chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|_| {
            // Fallback to unix timestamp when chrono panics
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| format!("unix:{}.{:03}", d.as_secs(), d.subsec_millis()))
                .unwrap_or_else(|_| "unknown".to_string())
        });

        // Get system information
        let system_info = std::panic::catch_unwind(get_system_info)
            .unwrap_or_else(|_| "Failed to get system info".to_string());

        // Get the panic message (try multiple ways to extract it)
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            // Try using the Display trait
            format!("{panic_info}")
        };

        // Get location information
        let location = if let Some(loc) = panic_info.location() {
            format!(
                "File: {}\n         Line: {}\n         Column: {}",
                loc.file(),
                loc.line(),
                loc.column()
            )
        } else {
            "Unknown location".to_string()
        };

        // Capture backtrace (full call stack)
        let backtrace = std::backtrace::Backtrace::force_capture();
        let backtrace_str = format!("{backtrace}");

        // Format log entries
        let separator = "=".repeat(80);
        let sub_separator = "-".repeat(40);
        let crash_entry = format!(
            r#"
{separator}
[CRASH REPORT] {timestamp}
{separator}

{sub_separator}
System Information
{sub_separator}
{system_info}

{sub_separator}
Error Details
{sub_separator}
Message: {message}

Location: {location}

{sub_separator}
Stack Trace (Backtrace)
{sub_separator}
{backtrace_str}

{separator}
"#
        );

        // Combine size check, rotation and append into the same critical section to avoid simultaneous panic of multiple threads
        // When two hooks compete for rename, the archive is lost.
        let crash_log_guard = CRASH_LOG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = rotate_crash_log_if_needed(&log_path);
        let saved =
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
                let _ = file.write_all(crash_entry.as_bytes());
                let _ = file.flush();
                true
            } else {
                false
            };
        drop(crash_log_guard);

        if saved {
            eprintln!(
                "\n[Copilot Bridge Atlas] Crash log saved to: {}",
                log_path.display()
            );
        }

        // At the same time, it is output to stderr (convenient for development and debugging)
        eprintln!("{crash_entry}");

        // Call default hook
        default_hook(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_log_path() {
        let path = get_crash_log_path();
        assert!(path.ends_with("crash.log"));
        assert!(path.to_string_lossy().contains(".copilot-bridge-atlas"));
    }

    #[test]
    fn test_system_info() {
        let info = get_system_info();
        assert!(info.contains("OS:"));
        assert!(info.contains("Arch:"));
        assert!(info.contains("App Version:"));
    }

    #[test]
    fn crash_log_rotation_keeps_bounded_archives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.log");

        fs::write(&path, b"first").unwrap();
        rotate_crash_log_if_needed_with_limit(&path, 4, 2).unwrap();
        assert!(!path.exists());
        assert_eq!(
            fs::read(rotated_crash_log_path(&path, 1)).unwrap(),
            b"first"
        );

        fs::write(&path, b"second").unwrap();
        rotate_crash_log_if_needed_with_limit(&path, 4, 2).unwrap();
        assert_eq!(
            fs::read(rotated_crash_log_path(&path, 1)).unwrap(),
            b"second"
        );
        assert_eq!(
            fs::read(rotated_crash_log_path(&path, 2)).unwrap(),
            b"first"
        );

        fs::write(&path, b"third").unwrap();
        rotate_crash_log_if_needed_with_limit(&path, 4, 2).unwrap();
        assert_eq!(
            fs::read(rotated_crash_log_path(&path, 1)).unwrap(),
            b"third"
        );
        assert_eq!(
            fs::read(rotated_crash_log_path(&path, 2)).unwrap(),
            b"second"
        );
        assert!(!rotated_crash_log_path(&path, 3).exists());
    }
}
