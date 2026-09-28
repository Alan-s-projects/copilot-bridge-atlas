import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { focusManager, useQueryClient } from "@tanstack/react-query";
import { usageKeys } from "@/lib/query/usage";
import { useWindowActive } from "@/lib/windowActivity";

/**
 * Listen to the backend `usage-log-recorded` event and immediately invalidate everything after receiving it.
 * UsageDashboard related queries so that users do not need to wait for the 30s polling cycle.
 *
 * The backend will emit this event when `proxy_request_logs` writes a new line (200ms anti-shake merge),
 * Proxy usage changes invalidate the active dashboard queries.
 *
 * This hook is only hung on UsageDashboard to avoid meaningless triggering elsewhere in the main interface.
 */
export function useUsageEventBridge() {
  const queryClient = useQueryClient();
  const active = useWindowActive();

  useEffect(() => {
    if (!active) return;
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    (async () => {
      const off = await listen("usage-log-recorded", () => {
        if (disposed || !focusManager.isFocused()) return;
        // invalidate the entire usage namespace: summary / trends /
        // modelStats / logs all follow the re-pull
        queryClient.invalidateQueries({ queryKey: usageKeys.all });
      });

      if (disposed) {
        off();
      } else {
        unlisten = off;
      }
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active, queryClient]);
}
