//! Global HTTP client module
//!
//! Provides an HTTP client that supports global proxy configuration.
//! All modules that need to send HTTP requests should use the client provided by this module.

use once_cell::sync::OnceCell;
use reqwest::Client;
use std::env;
use std::net::IpAddr;
use std::sync::RwLock;
use std::time::Duration;

/// Global HTTP client instance
static GLOBAL_CLIENT: OnceCell<RwLock<Client>> = OnceCell::new();

/// Copilot Bridge Atlas proxy server is currently listening on the port
static COPILOT_BRIDGE_ATLAS_PROXY_PORT: OnceCell<RwLock<u16>> = OnceCell::new();

/// Set the listening port of the Copilot Bridge Atlas proxy server
///
/// Should be called when the proxy server starts so that the system proxy detection can correctly identify its own port
pub fn set_proxy_port(port: u16) {
    if let Some(lock) = COPILOT_BRIDGE_ATLAS_PROXY_PORT.get() {
        if let Ok(mut current_port) = lock.write() {
            *current_port = port;
            log::debug!("[GlobalProxy] Updated Copilot Bridge Atlas proxy port to {port}");
        }
    } else {
        let _ = COPILOT_BRIDGE_ATLAS_PROXY_PORT.set(RwLock::new(port));
        log::debug!("[GlobalProxy] Initialized Copilot Bridge Atlas proxy port to {port}");
    }
}

/// Get the listening port of the Copilot Bridge Atlas proxy server
fn get_proxy_port() -> u16 {
    COPILOT_BRIDGE_ATLAS_PROXY_PORT
        .get()
        .and_then(|lock| lock.read().ok())
        .map(|port| *port)
        .unwrap_or(15722) // Default port as fallback
}

/// Initialize the global HTTP client
///
/// Should be called once when the app starts.
///
/// # Arguments
/// * `proxy_url` - Proxy URL, such as `http://127.0.0.1:7890` or `socks5://127.0.0.1:1080`
///   Passing in None or an empty string indicates a direct connection
pub fn init(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let client = build_client(effective_url)?;

    // Try to initialize the global client, log a warning if it already exists and update it with apply_proxy
    if GLOBAL_CLIENT.set(RwLock::new(client.clone())).is_err() {
        log::warn!(
            "[GlobalProxy] [GP-003] Already initialized, updating instead: {}",
            effective_url
                .map(mask_url)
                .unwrap_or_else(|| "direct connection".to_string())
        );
        // Initialized, update using apply_proxy instead
        return apply_proxy(proxy_url);
    }

    log::info!(
        "[GlobalProxy] Initialized: {}",
        effective_url
            .map(mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );

    Ok(())
}

/// Verify proxy configuration (do not apply)
///
/// Only verifies that the proxy URL is valid and does not actually update the global client.
/// Used to verify the validity of the configuration before persisting it.
///
/// # Arguments
/// * `proxy_url` - proxy URL, None or empty string indicates direct connection
///
/// # Returns
/// Ok(()) is returned if the verification is successful, and an error message is returned if it fails.
pub fn validate_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    // Just call build_client to verify, but not apply
    build_client(effective_url)?;
    Ok(())
}

/// Apply proxy configuration (assuming verified)
///
/// Apply proxy configuration directly to the global client without additional verification.
/// Should be called after validate_proxy succeeds.
///
/// # Arguments
/// * `proxy_url` - proxy URL, None or empty string indicates direct connection
pub fn apply_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let new_client = build_client(effective_url)?;

    // Update client
    if let Some(lock) = GLOBAL_CLIENT.get() {
        let mut client = lock.write().map_err(|e| {
            log::error!("[GlobalProxy] [GP-001] Failed to acquire write lock: {e}");
            "Failed to update proxy: lock poisoned".to_string()
        })?;
        *client = new_client;
    } else {
        // If it has not been initialized yet, initialize it
        return init(proxy_url);
    }

    log::info!(
        "[GlobalProxy] Applied: {}",
        effective_url
            .map(mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );

    Ok(())
}

/// Get the global HTTP client
///
/// Returns clients with a proxy configured if a proxy is configured, otherwise returns clients following the system proxy.
pub fn get() -> Client {
    GLOBAL_CLIENT
        .get()
        .and_then(|lock| lock.read().ok())
        .map(|c| c.clone())
        .unwrap_or_else(|| {
            log::warn!("[GlobalProxy] [GP-004] Client not initialized, using fallback");
            build_client(None).unwrap_or_default()
        })
}

/// Building an HTTP client
fn build_client(proxy_url: Option<&str>) -> Result<Client, String> {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Duration::from_secs(60))
        // Disable reqwest automatic decompression: prevent reqwest from overwriting the client's original accept-encoding header.
        // Response decompression is handled manually by response_processor based on content-encoding.
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd();

    // If there is a proxy address, use the proxy, otherwise follow the system proxy.
    if let Some(url) = proxy_url {
        // First verify the URL format and scheme
        let parsed = url::Url::parse(url)
            .map_err(|e| format!("Invalid proxy URL '{}': {}", mask_url(url), e))?;

        let scheme = parsed.scheme();
        if !["http", "https", "socks5", "socks5h"].contains(&scheme) {
            return Err(format!(
                "Invalid proxy scheme '{}' in URL '{}'. Supported: http, https, socks5, socks5h",
                scheme,
                mask_url(url)
            ));
        }

        let proxy = reqwest::Proxy::all(url)
            .map_err(|e| format!("Invalid proxy URL '{}': {}", mask_url(url), e))?;
        builder = builder.proxy(proxy);
        log::debug!("[GlobalProxy] Proxy configured: {}", mask_url(url));
    } else {
        // When the global proxy is not set, let reqwest automatically detect the system proxy (environment variable)
        // If the system proxy points to the local machine, disable the system proxy to avoid self-loop
        if system_proxy_points_to_loopback() {
            builder = builder.no_proxy();
            log::warn!(
                "[GlobalProxy] System proxy points to localhost, bypassing to avoid recursion"
            );
        } else {
            log::debug!("[GlobalProxy] Following system proxy (no explicit proxy configured)");
        }
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Exercise the production transport builder with an isolated loopback proxy.
#[cfg(test)]
pub(super) fn loopback_test_client(proxy_url: &str) -> Result<Client, String> {
    build_client(Some(proxy_url))
}

fn system_proxy_points_to_loopback() -> bool {
    const KEYS: [&str; 6] = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ];

    KEYS.iter()
        .filter_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .any(|value| proxy_points_to_loopback(&value))
}

fn proxy_points_to_loopback(value: &str) -> bool {
    fn host_is_loopback(host: &str) -> bool {
        if host.eq_ignore_ascii_case("localhost") {
            return true;
        }
        host.parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
    }

    // Check if it points to Copilot Bridge Atlas' own proxy port
    // Only the proxy pointing to yourself needs to be skipped to avoid recursion.
    fn is_copilot_bridge_atlas_proxy_port(port: Option<u16>) -> bool {
        let copilot_bridge_atlas_port = get_proxy_port();
        port == Some(copilot_bridge_atlas_port)
    }

    if let Ok(parsed) = url::Url::parse(value) {
        if let Some(host) = parsed.host_str() {
            // Returns true only if the host is a loopback and the port is the port of the Copilot Bridge Atlas
            return host_is_loopback(host) && is_copilot_bridge_atlas_proxy_port(parsed.port());
        }
        return false;
    }

    let with_scheme = format!("http://{value}");
    if let Ok(parsed) = url::Url::parse(&with_scheme) {
        if let Some(host) = parsed.host_str() {
            return host_is_loopback(host) && is_copilot_bridge_atlas_proxy_port(parsed.port());
        }
    }

    false
}

/// Hide sensitive information in URLs (for logging)
pub fn mask_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        // Hide username and password, retain scheme, host and port
        let host = parsed.host_str().unwrap_or("?");
        match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        }
    } else {
        // URL parsing failed, partial content returned. The truncation point falls back to the nearest character boundary,
        // Avoid panic caused by cutting in the middle of multi-byte UTF-8 characters.
        if url.len() > 20 {
            let cut = (0..=20)
                .rev()
                .find(|&i| url.is_char_boundary(i))
                .unwrap_or(0);
            format!("{}...", &url[..cut])
        } else {
            url.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn test_mask_url() {
        assert_eq!(mask_url("http://127.0.0.1:7890"), "http://127.0.0.1:7890");
        assert_eq!(
            mask_url("http://user:pass@127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            mask_url("socks5://admin:secret@proxy.example.com:1080"),
            "socks5://proxy.example.com:1080"
        );
        // Unported URLs should not display ":?"
        assert_eq!(
            mask_url("http://proxy.example.com"),
            "http://proxy.example.com"
        );
        assert_eq!(
            mask_url("https://user:pass@proxy.example.com"),
            "https://proxy.example.com"
        );
    }

    #[test]
    fn test_mask_url_does_not_panic_on_multibyte_boundary() {
        // A string that cannot be parsed by Url::parse and is cut exactly in the middle of a multibyte character at byte 20.
        // Regression: URL masking must not split multi-byte characters.
        let bad = format!("{}€invalid", "x".repeat(19));
        assert!(bad.len() > 20 && !bad.is_char_boundary(20));
        let masked = mask_url(&bad);
        assert!(masked.ends_with("..."));
    }

    #[test]
    fn test_build_client_direct() {
        let result = build_client(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_with_http_proxy() {
        let result = build_client(Some("http://127.0.0.1:7890"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_with_socks5_proxy() {
        let result = build_client(Some("socks5://127.0.0.1:1080"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_invalid_url() {
        // reqwest::Proxy::all will not immediately report an error for some invalid URLs
        // Use an explicitly invalid scheme to trigger an error
        let result = build_client(Some("invalid-scheme://127.0.0.1:7890"));
        assert!(result.is_err(), "Should reject invalid proxy scheme");
    }

    #[test]
    fn test_proxy_points_to_loopback() {
        // Set Copilot Bridge Atlas proxy port to 15722 (default)
        set_proxy_port(15722);

        // Only the loopback address pointing to the Copilot Bridge Atlas' own port returns true
        assert!(proxy_points_to_loopback("http://127.0.0.1:15722"));
        assert!(proxy_points_to_loopback("socks5://localhost:15722"));
        assert!(proxy_points_to_loopback("127.0.0.1:15722"));

        // Other loopback ports should not be skipped (allowing use of other local proxy tools)
        assert!(!proxy_points_to_loopback("http://127.0.0.1:7890"));
        assert!(!proxy_points_to_loopback("socks5://localhost:1080"));

        // Non-loopback addresses should not be skipped
        assert!(!proxy_points_to_loopback("http://192.168.1.10:7890"));
        assert!(!proxy_points_to_loopback("http://192.168.1.10:15722"));
    }

    #[test]
    fn test_system_proxy_points_to_loopback() {
        let _guard = env_lock().lock().unwrap();

        // Setting up the Copilot Bridge Atlas proxy port
        set_proxy_port(15722);

        let keys = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ];

        for key in &keys {
            std::env::remove_var(key);
        }

        // Proxies pointing to Copilot Bridge Atlas ports should be skipped
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:15722");
        assert!(system_proxy_points_to_loopback());

        // Local proxies pointing to other ports should not be skipped
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:7890");
        assert!(!system_proxy_points_to_loopback());

        // Non-loopback addresses should not be skipped
        std::env::set_var("HTTP_PROXY", "http://10.0.0.2:7890");
        assert!(!system_proxy_points_to_loopback());

        for key in &keys {
            std::env::remove_var(key);
        }
    }
}
