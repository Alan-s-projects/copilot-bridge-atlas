import { invoke } from "@tauri-apps/api/core";
import type {
  ProxyStatus,
  ProxyServerInfo,
  GlobalProxyConfig,
} from "@/types/proxy";

export const proxyApi = {
  // ========== Proxy Server Control API ==========

  // Start proxy server
  async startProxyServer(): Promise<ProxyServerInfo> {
    return invoke("start_proxy_server");
  },

  // Stop the proxy server (does not restore the taken over configuration)
  async stopProxyServer(): Promise<void> {
    return invoke("stop_proxy_server");
  },

  // Get proxy server status
  async getProxyStatus(): Promise<ProxyStatus> {
    return invoke("get_proxy_status");
  },

  // Get global proxy configuration
  async getGlobalProxyConfig(): Promise<GlobalProxyConfig> {
    return invoke("get_global_proxy_config");
  },

  // Update global proxy configuration
  async updateGlobalProxyConfig(config: GlobalProxyConfig): Promise<void> {
    return invoke("update_global_proxy_config", { config });
  },
};
