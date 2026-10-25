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
// Claude Desktop and Code share claude-cli/<version>. Only known entrypoints
// distinguish the product; generic SDK headers and the prefix alone do not.
fn claude_client(user_agent: &str) -> Option<&'static str> {
    let (product, rest) = user_agent.split_once('/')?;
    if !product.eq_ignore_ascii_case("claude-cli") {
        return None;
    }
    let (_, details) = rest.split_once(char::is_whitespace)?;
    let details = details.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut fields = details.split(',').map(str::trim);
    if !fields.next()?.eq_ignore_ascii_case("external") {
        return None;
    }
    match fields.next()?.to_ascii_lowercase().as_str() {
        "claude-desktop" | "claude-desktop-3p" | "remote_desktop" => Some("Claude Desktop"),
        "cli"
        | "mcp"
        | "bench"
        | "sdk-cli"
        | "sdk-ts"
        | "sdk-py"
        | "claude-vscode"
        | "claude-code-github-action" => Some("Claude Code"),
        _ => None,
    }
}

// Desktop's Electron networking (including gateway probes) is separate from
// the claude-cli agent. Read an exact product token, not an arbitrary substring
// or a generic Electron/Anthropic SDK signature shared by other applications.
fn claude_desktop_version(user_agent: &str) -> Option<&str> {
    user_agent.split_ascii_whitespace().find_map(|token| {
        let (product, version) = token.split_once('/')?;
        (product.eq_ignore_ascii_case("claude")
            && !version.is_empty()
            && version.len() <= 64
            && version.as_bytes()[0].is_ascii_digit()
            && version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_+".contains(&c)))
        .then_some(version)
    })
}

pub fn detect_client(headers: &HeaderMap) -> ClientIdentity {
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    let (ua_id, mut version) = ua.split_once('/').unwrap_or((ua, ""));
    let desktop_version = claude_desktop_version(ua);
    let ua_name = desktop_version
        .map(|_| "Claude Desktop")
        .or_else(|| claude_client(ua))
        .or_else(|| {
            if version.is_empty() {
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
            }
        });
    if let Some(desktop_version) = desktop_version {
        version = desktop_version;
    }
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

pub(crate) fn unknown_client_headers(headers: &HeaderMap) -> Option<String> {
    if detect_client(headers).client_source != "unknown" {
        return None;
    }
    let headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| {
            let normalized = name.as_str().replace('_', "-");
            let credential = matches!(normalized.as_str(), "cookie" | "set-cookie" | "x-auth")
                || [
                    "authorization",
                    "token",
                    "secret",
                    "password",
                    "api-key",
                    "apikey",
                ]
                .iter()
                .any(|suffix| {
                    normalized
                        .strip_suffix(*suffix)
                        .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('-'))
                });
            let value = if credential {
                std::borrow::Cow::Borrowed("[redacted]")
            } else {
                String::from_utf8_lossy(value.as_bytes())
            };
            (name.as_str(), value)
        })
        .collect();
    Some(serde_json::to_string(&headers).expect("header names and values are serializable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_headers_preserve_client_metadata_and_only_redact_credentials() {
        let mut headers = HeaderMap::from_iter([
            (
                "user-agent".parse().unwrap(),
                "new-client/1.2".parse().unwrap(),
            ),
            (
                "originator".parse().unwrap(),
                "new-desktop".parse().unwrap(),
            ),
            ("x-stainless-lang".parse().unwrap(), "js".parse().unwrap()),
            (
                "x-client-info".parse().unwrap(),
                "secret-review token-counter".parse().unwrap(),
            ),
            (
                "x-client-token-count".parse().unwrap(),
                "123".parse().unwrap(),
            ),
            (
                "authorization".parse().unwrap(),
                "Bearer private".parse().unwrap(),
            ),
            (
                "cookie".parse().unwrap(),
                "session=private".parse().unwrap(),
            ),
            ("x-api-key".parse().unwrap(), "private".parse().unwrap()),
            ("x-auth-token".parse().unwrap(), "private".parse().unwrap()),
        ]);
        headers.append("x-client-info", "second value".parse().unwrap());
        let snapshot: Vec<(String, String)> =
            serde_json::from_str(&unknown_client_headers(&headers).unwrap()).unwrap();
        for (name, value) in [
            ("user-agent", "new-client/1.2"),
            ("originator", "new-desktop"),
            ("x-stainless-lang", "js"),
            ("x-client-info", "secret-review token-counter"),
            ("x-client-info", "second value"),
            ("x-client-token-count", "123"),
        ] {
            assert!(snapshot.contains(&(name.into(), value.into())));
        }
        for name in ["authorization", "cookie", "x-api-key", "x-auth-token"] {
            assert!(snapshot.contains(&(name.into(), "[redacted]".into())));
        }
        assert_eq!(snapshot.len(), headers.len());
        headers.insert("originator", "codex_desktop".parse().unwrap());
        assert!(unknown_client_headers(&headers).is_none());
    }
    #[test]
    fn observed_desktop_gateway_probe_user_agent() {
        // Captured from Claude Desktop's Test connection action. Its native
        // Electron probe does not use the claude-cli agent's User-Agent.
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Claude/3.6556.2 Chrome/152.0.7977.167 Electron/44.9.3 Safari/537.36".parse().unwrap());
        let identity = detect_client(&headers);
        assert_eq!(identity.client_name, "Claude Desktop");
        assert_eq!(identity.client_source, "user-agent");
        assert_eq!(identity.client_version.as_deref(), Some("3.6556.2"));
    }

    #[test]
    fn desktop_product_token_is_distinct_from_cli_and_generic_clients() {
        let mut headers = HeaderMap::new();
        for ua in [
            "Claude/1.0.0",
            "Mozilla/5.0 Chrome/130.0.0.0 Claude/1.0.0 Electron/33.0.0 Safari/537.36",
            "Mozilla/5.0 claude/1.0.0",
        ] {
            headers.insert("user-agent", ua.parse().unwrap());
            let identity = detect_client(&headers);
            assert_eq!(identity.client_name, "Claude Desktop");
            assert_eq!(identity.client_source, "user-agent");
            assert_eq!(identity.client_version.as_deref(), Some("1.0.0"));
        }
        for ua in [
            "Mozilla/5.0 Chrome/130.0.0.0 Electron/33.0.0",
            "NotClaude/1.0.0",
            "https://example.com/Claude/1.0.0",
            "Claude/",
            "Claude/not-a-version",
            "Claude/1.0.0!",
            "Anthropic/JS 0.137.0",
        ] {
            headers.insert("user-agent", ua.parse().unwrap());
            assert_eq!(detect_client(&headers), ClientIdentity::default(), "{ua}");
        }
        headers.insert("user-agent", "Claude/1.0.0".parse().unwrap());
        headers.insert("originator", "codex_desktop".parse().unwrap());
        let identity = detect_client(&headers);
        assert_eq!(identity.client_name, "Codex Desktop");
        assert_eq!(identity.client_source, "originator");
        assert_eq!(identity.client_version, None);
    }
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
    #[test]
    fn claude_entrypoints_distinguish_code_and_desktop() {
        for (ua, expected) in [
            ("claude-cli/2.1.220 (external, cli)", "Claude Code"),
            (
                "claude-cli/2.1.220 (external, sdk-ts, agent-sdk/0.2.0)",
                "Claude Code",
            ),
            (
                "claude-cli/2.1.220 (external, sdk-py, agent-sdk/0.2.0)",
                "Claude Code",
            ),
            (
                "claude-cli/2.1.220 (external, claude-vscode)",
                "Claude Code",
            ),
            (
                "claude-cli/2.1.220 (external, claude-desktop)",
                "Claude Desktop",
            ),
            (
                "claude-cli/2.1.220 (external, claude-desktop-3p)",
                "Claude Desktop",
            ),
            (
                "claude-cli/2.1.220 (external, remote_desktop)",
                "Claude Desktop",
            ),
            (
                "  CLAUDE-CLI/2.1.220 (EXTERNAL, CLAUDE-DESKTOP)  ",
                "Claude Desktop",
            ),
            ("claude-cli/2.1.220", "Unknown"),
            ("claude-cli/2.1.220 (external, future-client)", "Unknown"),
            (
                "claude-cli/2.1.220 (external, claude-desktop-imposter)",
                "Unknown",
            ),
            ("claude-cli/2.1.220 (external, claude-desktop", "Unknown"),
            ("claude-cli/2.1.220 (internal, cli)", "Unknown"),
            ("not-claude-cli/2.1.220 (external, cli)", "Unknown"),
            ("Anthropic/Python 0.40.0", "Unknown"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("user-agent", ua.parse().unwrap());
            headers.insert("x-app", "cli".parse().unwrap());
            headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
            let got = detect_client(&headers);
            assert_eq!(got.client_name, expected, "{ua}");
            assert_eq!(
                got.client_source,
                if expected == "Unknown" {
                    "unknown"
                } else {
                    "user-agent"
                }
            );
            assert_eq!(
                got.client_version.as_deref(),
                if expected == "Unknown" {
                    None
                } else {
                    Some("2.1.220")
                }
            );
        }
    }

    #[test]
    fn claude_detection_preserves_originator_precedence_and_version_safety() {
        let mut headers = HeaderMap::new();
        headers.insert("originator", "Codex Desktop".parse().unwrap());
        headers.insert(
            "user-agent",
            "claude-cli/2.1.220 (external, claude-desktop)"
                .parse()
                .unwrap(),
        );
        let got = detect_client(&headers);
        assert_eq!(got.client_name, "Codex Desktop");
        assert_eq!(got.client_source, "originator");
        assert_eq!(got.client_version, None);
        headers.insert("originator", "unrecognized".parse().unwrap());
        assert_eq!(detect_client(&headers).client_name, "Claude Desktop");
        headers.insert(
            "user-agent",
            "claude-cli/2.1.220! (external, cli)".parse().unwrap(),
        );
        assert_eq!(detect_client(&headers).client_version, None);
        headers.insert(
            "user-agent",
            http::HeaderValue::from_bytes(b"\xff").unwrap(),
        );
        assert_eq!(detect_client(&headers), ClientIdentity::default());
    }
}
