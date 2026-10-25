import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { focusManager, useQueryClient } from "@tanstack/react-query";
import { usageKeys } from "@/lib/query/usage";
import { useWindowActive } from "@/lib/windowActivity";

export function useUsageEventBridge() {
  const queryClient = useQueryClient();
  const active = useWindowActive();

  useEffect(() => {
    if (!active) return;
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    let refreshing = false;
    let dirty = false;
    const filters = { queryKey: usageKeys.all };

    const refresh = async () => {
      dirty = true;
      if (refreshing) return;
      refreshing = true;
      try {
        while (dirty && !disposed && focusManager.isFocused()) {
          dirty = false;
          // An existing fetch may have read before this event's write.
          const alreadyFetching = queryClient.isFetching(filters) > 0;
          await queryClient.invalidateQueries(filters, {
            cancelRefetch: false,
          });
          dirty ||= alreadyFetching;
        }
      } finally {
        refreshing = false;
      }
    };

    void listen("usage-log-recorded", () => {
      if (disposed || !focusManager.isFocused()) return;
      void refresh().catch((error) =>
        console.error("Cannot refresh usage:", error),
      );
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) => console.error("Cannot observe usage:", error));

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active, queryClient]);
}
