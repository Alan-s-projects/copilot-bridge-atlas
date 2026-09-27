import { http, HttpResponse } from "msw";
import type { Provider, Settings } from "@/types";
import {
  getProviders,
  getCurrentProviderId,
  updateProvider,
  getSettings,
  setSettings,
} from "./state";
const root = "http://tauri.local";
const success = <T>(data: T) => HttpResponse.json(data as never);
const body = async <T>(request: Request): Promise<T> => {
  const text = await request.text();
  return text ? (JSON.parse(text) as T) : ({} as T);
};
export const handlers = [
  http.post(`${root}/get_providers`, () => success(getProviders())),
  http.post(`${root}/get_current_provider`, () =>
    success(getCurrentProviderId()),
  ),
  http.post(`${root}/update_provider`, async ({ request }) => {
    const data = await body<{ provider: Provider }>(request);
    updateProvider(data.provider);
    return success(true);
  }),
  http.post(`${root}/get_settings`, () => success(getSettings())),
  http.post(`${root}/save_settings`, async ({ request }) => {
    setSettings((await body<{ settings: Settings }>(request)).settings);
    return success(true);
  }),
  http.post(`${root}/get_proxy_status`, () =>
    success({
      running: false,
      address: "127.0.0.1",
      port: 15722,
      active_connections: 0,
      total_requests: 0,
      success_requests: 0,
      failed_requests: 0,
      success_rate: 0,
      uptime_seconds: 0,
      current_provider: null,
      current_provider_id: null,
      last_request_at: null,
      last_error: null,
      active_targets: [],
    }),
  ),
  http.post(`${root}/get_global_proxy_config`, () =>
    success({
      proxyEnabled: false,
      listenAddress: "127.0.0.1",
      listenPort: 15722,
    }),
  ),
  ...["update_tray_menu", "set_auto_launch"].map((command) =>
    http.post(`${root}/${command}`, () => success(true)),
  ),
  http.post(`${root}/list_db_backups`, () => success([])),
];
