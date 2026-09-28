/**
 * Global outbound proxy API
 *
 * Provides functionality to get, set, and test global proxies.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * Agent test results
 */
export interface ProxyTestResult {
  success: boolean;
  latencyMs: number;
  error: string | null;
}

/**
 * Detected proxy
 */
export interface DetectedProxy {
  url: string;
  proxyType: string;
  port: number;
}

/**
 * Get global proxy URL
 *
 * @returns proxy URL, null means not configured (direct connection)
 */
export async function getGlobalProxyUrl(): Promise<string | null> {
  return invoke<string | null>("get_global_proxy_url");
}

/**
 * Set global proxy URL
 *
 * @param url - Proxy URL (e.g. http://127.0.0.1:7890 or socks5://127.0.0.1:1080)
 *              An empty string indicates clearing the proxy (direct connection)
 */
export async function setGlobalProxyUrl(url: string): Promise<void> {
  try {
    return await invoke("set_global_proxy_url", { url });
  } catch (error) {
    // Tauri invoke error may be string
    throw new Error(typeof error === "string" ? error : String(error));
  }
}

/**
 * Test proxy connection
 *
 * @param url - the proxy URL to test
 * @returns test results, including success, delay and error information
 */
export async function testProxyUrl(url: string): Promise<ProxyTestResult> {
  return invoke<ProxyTestResult>("test_proxy_url", { url });
}

/**
 * Scan for local proxies
 *
 * @returns list of detected proxies
 */
export async function scanLocalProxies(): Promise<DetectedProxy[]> {
  return invoke<DetectedProxy[]>("scan_local_proxies");
}
