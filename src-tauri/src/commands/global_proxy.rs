//! Global outbound proxy related commands
//!
//! Provides Tauri commands for getting, setting, and testing global proxies.

use crate::proxy::http_client;
use crate::store::AppState;
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

/// Get global proxy URL
///
/// Returns the currently configured proxy URL, null indicates direct connection.
#[tauri::command]
pub fn get_global_proxy_url(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    let result = state.db.get_global_proxy_url().map_err(|e| e.to_string())?;
    log::debug!(
        "[GlobalProxy] [GP-010] Read from database: {}",
        result
            .as_ref()
            .map(|u| http_client::mask_url(u))
            .unwrap_or_else(|| "None".to_string())
    );
    Ok(result)
}

/// Set global proxy URL
///
/// - Pass in a non-empty string: enable proxy
/// - Pass in an empty string: clear proxy (direct connection)
///
/// Execution order: verify first → write DB → then apply
/// This ensures that when the DB write fails, there will be no inconsistency between the running state and persistence.
#[tauri::command]
pub fn set_global_proxy_url(state: tauri::State<'_, AppState>, url: String) -> Result<(), String> {
    // Debug: Display received URL information (excluding sensitive content)
    let has_auth = url.contains('@') && (url.starts_with("http://") || url.starts_with("socks"));
    log::debug!(
        "[GlobalProxy] [GP-011] Received URL: length={}, has_auth={}",
        url.len(),
        has_auth
    );

    let url_opt = if url.trim().is_empty() {
        None
    } else {
        Some(url.as_str())
    };

    // 1. First verify whether the proxy configuration is valid (do not apply)
    http_client::validate_proxy(url_opt)?;

    // 2. Save to database after successful verification
    state
        .db
        .set_global_proxy_url(url_opt)
        .map_err(|e| e.to_string())?;

    // 3. After the DB is written successfully, apply it to the running state.
    http_client::apply_proxy(url_opt)?;

    log::info!(
        "[GlobalProxy] [GP-009] Configuration updated: {}",
        url_opt
            .map(http_client::mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );

    Ok(())
}

/// Agent test results
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    /// Is the connection successful?
    pub success: bool,
    /// Latency (milliseconds)
    pub latency_ms: u64,
    /// error message
    pub error: Option<String>,
}

/// Test proxy connection
///
/// Sends a test request through the specified proxy URL, returning connection results and latency.
/// Use multiple test targets, any success is considered the agent is available.
#[tauri::command]
pub async fn test_proxy_url(url: String) -> Result<ProxyTestResult, String> {
    if url.trim().is_empty() {
        return Err("Proxy URL is empty".to_string());
    }

    let start = Instant::now();

    // Building a temporary client with a proxy
    let proxy = reqwest::Proxy::all(&url).map_err(|e| format!("Invalid proxy URL: {e}"))?;

    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build client: {e}"))?;

    // Probe the two services needed by the bridge, without an inference request.
    let test_urls = ["https://api.githubcopilot.com", "https://api.github.com"];

    let mut last_error = None;

    for test_url in test_urls {
        match client.head(test_url).send().await {
            Ok(resp) => {
                let latency = start.elapsed().as_millis() as u64;
                log::debug!(
                    "[GlobalProxy] Test successful: {} -> {} via {} ({}ms)",
                    http_client::mask_url(&url),
                    test_url,
                    resp.status(),
                    latency
                );
                return Ok(ProxyTestResult {
                    success: true,
                    latency_ms: latency,
                    error: None,
                });
            }
            Err(e) => {
                log::debug!("[GlobalProxy] Test to {test_url} failed: {e}");
                last_error = Some(e);
            }
        }
    }

    // All test targets failed
    let latency = start.elapsed().as_millis() as u64;
    let error_msg = last_error
        .map(|e| e.to_string())
        .unwrap_or_else(|| "All test targets failed".to_string());

    log::debug!(
        "[GlobalProxy] Test failed: {} -> {} ({}ms)",
        http_client::mask_url(&url),
        error_msg,
        latency
    );

    Ok(ProxyTestResult {
        success: false,
        latency_ms: latency,
        error: Some(error_msg),
    })
}

/// Detected proxy information
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedProxy {
    /// Proxy URL
    pub url: String,
    /// Proxy type (http/socks5)
    pub proxy_type: String,
    /// port
    pub port: u16,
}

/// Common proxy port configurations
/// Format: (port, main type, whether to support both http and socks5)
/// For mixed ports, two protocols will be returned at the same time for the user to choose.
const PROXY_PORTS: &[(u16, &str, bool)] = &[
    (7890, "http", true),     // Clash (mixed mode)
    (7891, "socks5", false),  // Clash SOCKS only
    (1080, "socks5", false),  // Universal SOCKS5
    (8080, "http", false),    // Generic HTTP
    (8888, "http", false),    // Charles/Fiddler
    (3128, "http", false),    // Squid
    (10808, "socks5", false), // V2Ray SOCKS
    (10809, "http", false),   // V2Ray HTTP
];

/// Scan for local proxies
///
/// Check whether there is a proxy service running on common ports.
/// Use asynchronous tasks to avoid blocking the UI thread.
#[tauri::command]
pub async fn scan_local_proxies() -> Vec<DetectedProxy> {
    // Use spawn_blocking to avoid blocking the main thread
    tokio::task::spawn_blocking(|| {
        let mut found = Vec::new();

        for &(port, primary_type, is_mixed) in PROXY_PORTS {
            let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
            if TcpStream::connect_timeout(&addr.into(), Duration::from_millis(100)).is_ok() {
                // Add main type
                found.push(DetectedProxy {
                    url: format!("{primary_type}://127.0.0.1:{port}"),
                    proxy_type: primary_type.to_string(),
                    port,
                });
                // For mixed ports, also add another protocol
                if is_mixed {
                    let alt_type = if primary_type == "http" {
                        "socks5"
                    } else {
                        "http"
                    };
                    found.push(DetectedProxy {
                        url: format!("{alt_type}://127.0.0.1:{port}"),
                        proxy_type: alt_type.to_string(),
                        port,
                    });
                }
            }
        }

        found
    })
    .await
    .unwrap_or_default()
}
