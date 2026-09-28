import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Provider } from "@/types";
export const providersApi = {
  getAll: (): Promise<Record<string, Provider>> =>
    invoke("get_providers", { app: "codex" }),
  getCurrent: (): Promise<string> =>
    invoke("get_current_provider", { app: "codex" }),
  update: (provider: Provider): Promise<boolean> =>
    invoke("update_provider", { provider, app: "codex" }),
  onSwitched: (handler: () => void): Promise<UnlistenFn> =>
    listen("provider-switched", handler),
};
