//! Best-effort caller labels, captured before upstream fingerprint replacement.
use http::HeaderMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientIdentity {
    pub client_name: String,
    pub client_source: String,
    pub client_version: Option<String>,
}
impl Default for ClientIdentity {
    fn default() -> Self {
        Self {
            client_name: "Unknown".into(),
            client_source: "unknown".into(),
            client_version: None,
        }
    }
}
fn classify(value: &str) -> Option<&'static str> {
    match value {
        "codex desktop" | "codex_desktop" | "codex_chatgpt_desktop" => Some("Codex Desktop"),
        "codex_exec" | "codex-tui" | "codex_cli_rs" => Some("Codex CLI"),
        "codex_vscode" => Some("Codex VS Code"),
        v if ["codex_", "codex-", "codex "]
            .iter()
            .any(|p| v.starts_with(p)) =>
        {
            Some("Codex — unspecified")
        }
        _ => None,
    }
}
pub fn detect_client(headers: &HeaderMap) -> ClientIdentity {
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    let (ua_id, version) = ua.split_once('/').unwrap_or((ua, ""));
    let ua_name = if version.is_empty() {
        let lower = ua.to_ascii_lowercase();
        ["codex/", "codex_", "codex-", "codex "]
            .iter()
            .any(|p| lower.starts_with(p))
            .then_some("Codex — unspecified")
    } else {
        classify(&ua_id.to_ascii_lowercase()).or_else(|| {
            ua_id
                .eq_ignore_ascii_case("codex")
                .then_some("Codex — unspecified")
        })
    };
    let origin = headers
        .get("originator")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| classify(&v.trim().to_ascii_lowercase()));
    let (name, source) = match (origin, ua_name) {
        (Some(name), _) => (name, "originator"),
        (_, Some(name)) => (name, "user-agent"),
        _ => return ClientIdentity::default(),
    };
    // Never retain arbitrary headers or attach a conflicting client's version.
    let version = version.split_whitespace().next().filter(|v| {
        !v.is_empty()
            && v.len() <= 64
            && v.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_+".contains(&c))
    });
    ClientIdentity {
        client_name: name.into(),
        client_source: source.into(),
        client_version: if ua_name == Some(name) {
            version.map(str::to_owned)
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_rules_and_precedence() {
        for (origin, ua, name, source, version) in [
            (
                " Codex Desktop ",
                "codex_exec/1.2",
                "Codex Desktop",
                "originator",
                None,
            ),
            (
                "CODEX_EXEC",
                "codex_exec/0.163.0-alpha.11 (Mac OS)",
                "Codex CLI",
                "originator",
                Some("0.163.0-alpha.11"),
            ),
            ("codex-tui", "", "Codex CLI", "originator", None),
            ("codex_cli_rs", "", "Codex CLI", "originator", None),
            ("codex_desktop", "", "Codex Desktop", "originator", None),
            (
                "codex_chatgpt_desktop",
                "",
                "Codex Desktop",
                "originator",
                None,
            ),
            ("codex_vscode", "", "Codex VS Code", "originator", None),
            (
                "codex_future",
                "",
                "Codex — unspecified",
                "originator",
                None,
            ),
            (
                "other",
                "codex_vscode/1.2",
                "Codex VS Code",
                "user-agent",
                Some("1.2"),
            ),
            (
                "",
                "Codex Desktop/1.2 (Mac OS)",
                "Codex Desktop",
                "user-agent",
                Some("1.2"),
            ),
            (
                "",
                "codex/1.2",
                "Codex — unspecified",
                "user-agent",
                Some("1.2"),
            ),
            ("", "python-httpx/0.28", "Unknown", "unknown", None),
            ("", "not-codex/1.2", "Unknown", "unknown", None),
            (
                "",
                "codex_future",
                "Codex — unspecified",
                "user-agent",
                None,
            ),
            ("", "", "Unknown", "unknown", None),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("originator", origin.parse().unwrap());
            headers.insert("user-agent", ua.parse().unwrap());
            let got = detect_client(&headers);
            assert_eq!(
                (
                    got.client_name.as_str(),
                    got.client_source.as_str(),
                    got.client_version.as_deref()
                ),
                (name, source, version)
            );
        }
    }
}
