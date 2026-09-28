use crate::error::AppError;
use serde::Serialize;
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

static LEGACY_APP_CONFIG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn get_home_dir() -> PathBuf {
    std::env::var("COPILOT_BRIDGE_ATLAS_TEST_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| PathBuf::from(value.trim()))
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
}

/// Atlas never discovers another application's data directory.
pub fn get_app_config_dir() -> PathBuf {
    LEGACY_APP_CONFIG_DIR
        .get()
        .cloned()
        .flatten()
        .unwrap_or_else(|| get_home_dir().join(".copilot-bridge-atlas"))
}

/// Keep an earlier Atlas folder selection for the process lifetime without
/// restoring the removed directory editor, store plugin, or write commands.
pub(crate) fn initialize_legacy_app_config_dir(app_data_dir: &Path) {
    LEGACY_APP_CONFIG_DIR.get_or_init(|| {
        read_legacy_app_config_dir(&app_data_dir.join("app_paths.json"), &get_home_dir())
    });
}

fn read_legacy_app_config_dir(store_path: &Path, home: &Path) -> Option<PathBuf> {
    let bytes = fs::read(store_path).ok()?;
    let store: Value = serde_json::from_slice(&bytes).ok()?;
    let raw = store.get("app_config_dir_override")?.as_str()?.trim();
    let path = if raw == "~" {
        home.to_path_buf()
    } else if let Some(relative) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        home.join(relative)
    } else {
        PathBuf::from(raw)
    };
    (path.is_absolute() && path.is_dir()).then_some(path)
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }

    normalized
}

fn comparable_path_key(path: &Path) -> String {
    let mut key = normalize_path_lexically(path).to_string_lossy().to_string();

    #[cfg(windows)]
    {
        key = key.replace('\\', "/");
    }

    while key.len() > 1 && key.ends_with('/') {
        key.pop();
    }

    #[cfg(windows)]
    {
        key.make_ascii_lowercase();
    }

    key
}

/// Returns true when `path` is lexically contained within `base`.
///
/// Both paths are normalized lexically (without hitting the filesystem), so
/// this works for non-existent paths. It is **not** a symlink defense: a
/// symlink inside `base` can still lead a resolved path outside it. Callers
/// that go on to open the file must canonicalize the existing path and
/// re-verify containment before reading.
/// On Windows the comparison is case-insensitive.
pub(crate) fn path_is_within(base: &Path, path: &Path) -> bool {
    let base_key = comparable_path_key(base);
    let path_key = comparable_path_key(path);

    if path_key == base_key {
        return true;
    }

    let prefix = format!("{base_key}/");
    path_key.starts_with(&prefix)
}

/// Recursively sort the JSON object's keys (alphabetical order), ensuring that the serialized output is deterministic
fn sort_json_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted_map = Map::new();
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for key in keys {
                sorted_map.insert(key.clone(), sort_json_keys(&map[key]));
            }
            Value::Object(sorted_map)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_json_keys).collect()),
        other => other.clone(),
    }
}

/// Write stable JSON into Atlas's own storage using atomic replacement.
pub fn write_json_file<T: Serialize>(path: &Path, data: &T) -> Result<(), AppError> {
    // Make sure the directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let value = serde_json::to_value(data).map_err(|e| AppError::JsonSerialize { source: e })?;
    let sorted_value = sort_json_keys(&value);
    let json = serde_json::to_string_pretty(&sorted_value)
        .map_err(|e| AppError::JsonSerialize { source: e })?;

    atomic_write(path, json.as_bytes())
}

/// Atomic write: Rename replacement after writing to temporary file to avoid half-write state
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("Invalid path".to_string()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("Invalid file name".to_string()))?
        .to_string_lossy()
        .to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (tmp, mut file) = (|| -> Result<(PathBuf, fs::File), AppError> {
        let mut last_collision = None;
        for _ in 0..16 {
            let counter = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let candidate = parent.join(format!(
                "{file_name}.tmp.{}.{ts}.{counter}",
                std::process::id()
            ));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            match options.open(&candidate) {
                Ok(file) => return Ok((candidate, file)),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_collision = Some((candidate, source));
                }
                Err(source) => return Err(AppError::io(&candidate, source)),
            }
        }

        let (candidate, source) = last_collision.expect("temporary filename loop must run");
        Err(AppError::io(&candidate, source))
    })()?;

    if let Err(source) = file.write_all(data).and_then(|_| file.flush()) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(AppError::io(&tmp, source));
    }
    drop(file);

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::ERROR_NOT_SUPPORTED, Storage::FileSystem::ReplaceFileW,
        };

        let replaced: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let replacement: Vec<u16> = tmp
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut completed = false;
        let mut last_error = None;

        for _ in 0..3 {
            // SAFETY: both path buffers are NUL-terminated UTF-16 and remain alive for the
            // duration of the call. Backup, exclusion, and reserved pointers are intentionally null.
            let replaced_ok = unsafe {
                ReplaceFileW(
                    replaced.as_ptr(),
                    replacement.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if replaced_ok != 0 {
                completed = true;
                break;
            }

            let replace_error = std::io::Error::last_os_error();
            // WSL UNC paths reject ReplaceFileW with ERROR_NOT_SUPPORTED (50).
            // std::fs::rename uses a different replace-existing API on Windows.
            let replace_not_supported =
                replace_error.raw_os_error() == Some(ERROR_NOT_SUPPORTED as i32);
            if replace_error.kind() != std::io::ErrorKind::NotFound && !replace_not_supported {
                last_error = Some(replace_error);
                break;
            }

            match fs::rename(&tmp, path) {
                Ok(()) => {
                    completed = true;
                    break;
                }
                Err(source)
                    if matches!(
                        source.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    last_error = Some(source);
                }
                Err(source) => {
                    last_error = Some(source);
                    break;
                }
            }
        }

        if !completed {
            let source = last_error.unwrap_or_else(std::io::Error::last_os_error);
            let _ = fs::remove_file(&tmp);
            return Err(AppError::IoContext {
                context: format!(
                    "Atomic replacement failed: {} -> {}",
                    tmp.display(),
                    path.display()
                ),
                source,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_data_directory_preserves_existing_data_and_store() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("existing data");
        fs::create_dir_all(&data_dir).unwrap();
        let settings_path = data_dir.join("settings.json");
        fs::write(&settings_path, b"existing preferences").unwrap();
        let store_path = root.path().join("app_paths.json");
        let bytes = serde_json::to_vec(&serde_json::json!({
            "app_config_dir_override": data_dir,
            "retired_metadata": {"preserve": true}
        }))
        .unwrap();
        fs::write(&store_path, &bytes).unwrap();

        assert_eq!(
            read_legacy_app_config_dir(&store_path, root.path()),
            Some(data_dir)
        );
        assert_eq!(fs::read(&store_path).unwrap(), bytes);
        assert_eq!(fs::read(&settings_path).unwrap(), b"existing preferences");
        assert!(!root.path().join(".copilot-bridge-atlas").exists());
    }

    #[test]
    fn legacy_data_directory_accepts_existing_home_relative_paths() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("atlas-data");
        fs::create_dir_all(&data_dir).unwrap();
        let store_path = root.path().join("app_paths.json");

        for raw in ["~/atlas-data", "~\\atlas-data", "  ~/atlas-data  ", "~"] {
            let store = serde_json::json!({"app_config_dir_override": raw});
            fs::write(&store_path, serde_json::to_vec(&store).unwrap()).unwrap();
            assert_eq!(
                read_legacy_app_config_dir(&store_path, root.path()),
                Some(if raw == "~" {
                    root.path().to_path_buf()
                } else {
                    data_dir.clone()
                })
            );
        }
    }

    #[test]
    fn legacy_data_directory_ignores_missing_invalid_or_unusable_selections() {
        let root = tempfile::tempdir().unwrap();
        let store_path = root.path().join("app_paths.json");
        assert_eq!(read_legacy_app_config_dir(&store_path, root.path()), None);
        assert!(!store_path.exists());

        for store in [
            serde_json::json!({}),
            serde_json::json!({"app_config_dir_override": null}),
            serde_json::json!({"app_config_dir_override": 42}),
            serde_json::json!({"app_config_dir_override": ""}),
            serde_json::json!({"app_config_dir_override": "."}),
            serde_json::json!({"app_config_dir_override": root.path().join("missing")}),
            serde_json::json!({"app_config_dir_override": store_path}),
        ] {
            let bytes = serde_json::to_vec(&store).unwrap();
            fs::write(&store_path, &bytes).unwrap();
            assert_eq!(read_legacy_app_config_dir(&store_path, root.path()), None);
            assert_eq!(fs::read(&store_path).unwrap(), bytes);
        }
        fs::write(&store_path, b"not JSON").unwrap();
        assert_eq!(read_legacy_app_config_dir(&store_path, root.path()), None);
        assert_eq!(fs::read(&store_path).unwrap(), b"not JSON");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    fn assert_atomic_write_replaces_existing_file(dir: &Path) {
        let path = dir.join("atomic-write-contract.json");
        std::fs::write(&path, b"old contents").unwrap();

        atomic_write(&path, b"new contents").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new contents");
        let tmp_prefix = "atomic-write-contract.json.tmp.";
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(tmp_prefix))
            .map(|entry| entry.path())
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files remain: {leftovers:?}"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_atomic_write_replaces_existing_file(dir.path());
    }

    #[cfg(windows)]
    #[test]
    fn atomic_write_preserves_destination_when_windows_replace_fails() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, b"old contents").unwrap();
        let held_file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();

        let result = atomic_write(&path, b"new contents");

        assert!(result.is_err());
        drop(held_file);
        assert_eq!(std::fs::read(&path).unwrap(), b"old contents");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn sort_json_keys_sorts_top_level_object() {
        let input = serde_json::json!({
            "z": 1,
            "a": 2,
            "m": 3,
        });
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(serialized, r#"{"a":2,"m":3,"z":1}"#);
    }

    #[test]
    fn sort_json_keys_recurses_into_nested_objects() {
        let input = serde_json::json!({
            "outer_b": {"z": 1, "a": 2},
            "outer_a": {"y": 3, "b": 4},
        });
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(
            serialized,
            r#"{"outer_a":{"b":4,"y":3},"outer_b":{"a":2,"z":1}}"#
        );
    }

    #[test]
    fn sort_json_keys_preserves_array_order() {
        let input = serde_json::json!([3, 1, 2]);
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(serialized, "[3,1,2]");
    }

    #[test]
    fn sort_json_keys_sorts_objects_inside_arrays_but_keeps_array_order() {
        let input = serde_json::json!([
            {"z": 1, "a": 2},
            {"y": 3, "b": 4},
        ]);
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(serialized, r#"[{"a":2,"z":1},{"b":4,"y":3}]"#);
    }

    #[test]
    fn sort_json_keys_passes_through_primitives() {
        let cases = vec![
            serde_json::json!("hello"),
            serde_json::json!(42),
            serde_json::json!(3.5),
            serde_json::json!(true),
            serde_json::json!(null),
        ];
        for value in cases {
            let sorted = sort_json_keys(&value);
            assert_eq!(sorted, value);
        }
    }

    #[test]
    fn sort_json_keys_handles_empty_collections() {
        let empty_obj = serde_json::json!({});
        assert_eq!(
            serde_json::to_string(&sort_json_keys(&empty_obj)).unwrap(),
            "{}"
        );

        let empty_arr = serde_json::json!([]);
        assert_eq!(
            serde_json::to_string(&sort_json_keys(&empty_arr)).unwrap(),
            "[]"
        );
    }

    #[test]
    fn sort_json_keys_produces_identical_output_for_different_insertion_orders() {
        // Core guarantee: Regardless of the insertion order of keys in the same logical configuration, the written byte sequence must be consistent.
        let mut a = Map::new();
        a.insert("env".to_string(), serde_json::json!({"PATH": "/usr/bin"}));
        a.insert("model".to_string(), serde_json::json!("gpt-6-astra"));
        a.insert("permissions".to_string(), serde_json::json!({"allow": []}));

        let mut b = Map::new();
        b.insert("permissions".to_string(), serde_json::json!({"allow": []}));
        b.insert("model".to_string(), serde_json::json!("gpt-6-astra"));
        b.insert("env".to_string(), serde_json::json!({"PATH": "/usr/bin"}));

        let sorted_a = sort_json_keys(&Value::Object(a));
        let sorted_b = sort_json_keys(&Value::Object(b));

        assert_eq!(
            serde_json::to_string(&sorted_a).unwrap(),
            serde_json::to_string(&sorted_b).unwrap(),
        );
    }
}
