use serde::Serialize;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Serialize)]
pub struct InitErrorPayload {
    pub path: String,
    pub error: String,
    /// An incompatible database opens the recovery view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// user_version of the database on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_version: Option<i32>,
    /// Schema version required by Atlas 6.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_version: Option<i32>,
}

static INIT_ERROR: OnceLock<RwLock<Option<InitErrorPayload>>> = OnceLock::new();

fn cell() -> &'static RwLock<Option<InitErrorPayload>> {
    INIT_ERROR.get_or_init(|| RwLock::new(None))
}

pub fn set_init_error(payload: InitErrorPayload) {
    #[allow(clippy::unwrap_used)]
    if let Ok(mut guard) = cell().write() {
        *guard = Some(payload);
    }
}

pub fn get_init_error() -> Option<InitErrorPayload> {
    cell().read().ok()?.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_error_roundtrip() {
        let previous = get_init_error();
        let payload = InitErrorPayload {
            path: r"C:\Atlas\copilot-bridge-atlas.db".into(),
            error: "database version is newer than supported".into(),
            kind: Some("db_schema_incompatible".into()),
            db_version: Some(999),
            supported_version: Some(crate::database::SCHEMA_VERSION),
        };
        set_init_error(payload.clone());
        let got = get_init_error().expect("should get payload back");
        assert_eq!(got.path, payload.path);
        assert_eq!(got.error, payload.error);
        assert_eq!(got.kind, payload.kind);
        assert_eq!(got.db_version, payload.db_version);
        assert_eq!(got.supported_version, payload.supported_version);
        *cell().write().expect("restore initialization status") = previous;
    }
}
