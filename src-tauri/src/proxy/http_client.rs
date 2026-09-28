//! Global HTTP client module
//!
//! Provides an HTTP client that supports global proxy configuration.
//! All modules that need to send HTTP requests should use the client provided by this module.

use once_cell::sync::OnceCell;
use reqwest::Client;
use std::env;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::RwLock;
use std::time::Duration;

/// Global HTTP client instance
static GLOBAL_CLIENT: OnceCell<RwLock<Client>> = OnceCell::new();

/// Copilot Bridge Atlas proxy server is currently listening on the port
static COPILOT_BRIDGE_ATLAS_PROXY_PORT: AtomicU16 = AtomicU16::new(15722);

/// Set the listening port of the Copilot Bridge Atlas proxy server
///
/// Should be called when the proxy server starts so that the system proxy detection can correctly identify its own port
pub fn set_proxy_port(port: u16) {
    COPILOT_BRIDGE_ATLAS_PROXY_PORT.store(port, Ordering::Relaxed);
}

/// Get the listening port of the Copilot Bridge Atlas proxy server
fn get_proxy_port() -> u16 {
    COPILOT_BRIDGE_ATLAS_PROXY_PORT.load(Ordering::Relaxed)
}

fn effective_proxy_url(proxy_url: Option<&str>) -> Option<&str> {
    proxy_url.map(str::trim).filter(|url| !url.is_empty())
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
    // Just call build_client to verify, but not apply
    build_client(effective_proxy_url(proxy_url))?;
    Ok(())
}

/// Build and install the shared transport at startup or after a settings change.
/// Empty settings follow the system proxy, with Atlas loop prevention.
pub fn apply_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = effective_proxy_url(proxy_url);
    let new_client = build_client(effective_url)?;
    let lock = GLOBAL_CLIENT.get_or_init(|| RwLock::new(new_client.clone()));
    let mut client = lock
        .write()
        .map_err(|_| "Failed to update proxy: HTTP client lock poisoned".to_string())?;
    *client = new_client;

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
pub fn get() -> Result<Client, String> {
    client_from(&GLOBAL_CLIENT)
}

fn client_from(cell: &OnceCell<RwLock<Client>>) -> Result<Client, String> {
    let lock = cell.get_or_try_init(|| build_client(None).map(RwLock::new))?;
    lock.read()
        .map(|client| client.clone())
        .map_err(|_| "Failed to read proxy: HTTP client lock poisoned".to_string())
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
        if !parsed.has_host() {
            return Err("Invalid proxy URL: a scheme and host are required".to_string());
        }

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
    // Environment variables may use a bare host:port, including localhost or IPv6.
    let parsed = url::Url::parse(value)
        .ok()
        .filter(url::Url::has_host)
        .or_else(|| url::Url::parse(&format!("http://{value}")).ok());
    parsed.is_some_and(|url| {
        let loopback = match url.host() {
            Some(url::Host::Domain(host)) => {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            }
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => {
                ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback())
            }
            None => false,
        };
        loopback && url.port_or_known_default() == Some(get_proxy_port())
    })
}

/// Hide sensitive information in URLs (for logging)
pub fn mask_url(url: &str) -> String {
    if let Some(parsed) = url::Url::parse(url).ok().filter(url::Url::has_host) {
        // Hide username and password, retain scheme, host and port
        let host = parsed.host_str().unwrap_or("?");
        match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        }
    } else {
        // An unparseable prefix may itself contain credentials.
        "[invalid proxy URL]".to_string()
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
    fn malformed_proxy_urls_never_expose_credential_prefixes() {
        for invalid in [
            "user:secret",
            "user:secret@proxy:bad-port",
            "http://user:secret@proxy:bad-port",
            "http://user:secret@[broken",
            "xxxxxxxxxxxxxxxxxxx€invalid",
        ] {
            assert_eq!(mask_url(invalid), "[invalid proxy URL]");
        }
        let error = build_client(Some("http://user:secret@proxy:bad-port")).unwrap_err();
        assert!(!error.contains("user"));
        assert!(!error.contains("secret"));
    }

    #[test]
    fn shared_client_initialization_is_cached_and_poisoning_is_reported() {
        let cell = OnceCell::new();
        assert!(client_from(&cell).is_ok());
        assert!(cell.get().is_some());
        assert!(client_from(&cell).is_ok());

        let _ = std::panic::catch_unwind(|| {
            let _guard = cell.get().unwrap().write().unwrap();
            panic!("simulate a failed client update");
        });
        assert_eq!(
            client_from(&cell).unwrap_err(),
            "Failed to read proxy: HTTP client lock poisoned"
        );
    }

    #[test]
    fn proxy_settings_trim_surrounding_whitespace() {
        assert!(validate_proxy(Some("  http://127.0.0.1:7890 \n")).is_ok());
        assert_eq!(effective_proxy_url(Some(" \n ")), None);
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
        assert!(proxy_points_to_loopback("socks5://127.0.0.1:15722"));
        assert!(proxy_points_to_loopback("127.0.0.1:15722"));
        assert!(proxy_points_to_loopback("localhost:15722"));
        assert!(proxy_points_to_loopback("http://[::1]:15722"));
        assert!(proxy_points_to_loopback("socks5://[::1]:15722"));
        assert!(proxy_points_to_loopback("[::1]:15722"));
        assert!(proxy_points_to_loopback("http://[::ffff:127.0.0.1]:15722"));

        // Other loopback ports should not be skipped (allowing use of other local proxy tools)
        assert!(!proxy_points_to_loopback("http://127.0.0.1:7890"));
        assert!(!proxy_points_to_loopback("socks5://localhost:1080"));
        assert!(!proxy_points_to_loopback("http://[::1]:7890"));

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
