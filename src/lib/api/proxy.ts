import { invoke } from "@tauri-apps/api/core";
import type {
  ProxyStatus,
  ProxyServerInfo,
  GlobalProxyConfig,
} from "@/types/proxy";

export type GlobalConfigUpdate =
  | GlobalProxyConfig
  | ((current: GlobalProxyConfig) => GlobalProxyConfig);
let configWrites: Promise<unknown> = Promise.resolve();

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
  updateGlobalProxyConfig(
    update: GlobalConfigUpdate,
  ): Promise<GlobalProxyConfig> {
    const write = configWrites
      .catch(() => undefined)
      .then(async () => {
        const config =
          typeof update === "function"
            ? update(await proxyApi.getGlobalProxyConfig())
            : update;
        await invoke("update_global_proxy_config", { config });
        return config;
      });
    configWrites = write;
    return write;
  },
};
