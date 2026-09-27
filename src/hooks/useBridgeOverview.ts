import { useEffect } from "react";
import { focusManager, useQuery, useQueryClient } from "@tanstack/react-query";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { usageApi } from "@/lib/api/usage";
import { proxyKeys } from "@/lib/query/proxy";
import { resolveUsageRange } from "@/lib/usageRange";
import { useWindowActive } from "@/lib/windowActivity";

export const bridgeOverviewKey = ["bridge-overview"] as const;
export const OVERVIEW_REQUEST_LIMIT = 5;
const REFRESH_DELAY_MS = 5000;

export function useBridgeOverview() {
  const active = useWindowActive();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: bridgeOverviewKey,
    queryFn: async () => {
      const { startDate, endDate } = resolveUsageRange({ preset: "today" });
      const [summary, recent] = await Promise.all([
        usageApi.getUsageSummary(startDate, endDate, "codex"),
        usageApi.getRequestLogs(
          { appType: "codex" },
          0,
          OVERVIEW_REQUEST_LIMIT,
        ),
      ]);
      return { summary, recent };
    },
    enabled: active,
    refetchInterval: false,
    refetchOnWindowFocus: true,
    staleTime: 0,
  });

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    let unlistenActivity: UnlistenFn | undefined;
    let pending: ReturnType<typeof setTimeout> | undefined;
    let midnight: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => {
      pending = undefined;
      if (disposed || !focusManager.isFocused()) return;
      void queryClient.invalidateQueries(
        { queryKey: bridgeOverviewKey, exact: true },
        { cancelRefetch: false },
      );
      void queryClient.invalidateQueries(
        { queryKey: proxyKeys.status, exact: true },
        { cancelRefetch: false },
      );
    };
    const schedule = () => {
      if (disposed || !focusManager.isFocused()) return;
      if (pending === undefined)
        pending = setTimeout(refresh, REFRESH_DELAY_MS);
    };
    // One day-boundary wake-up keeps "Today" correct without idle SQL polling.
    const nextDay = () => {
      if (disposed) return;
      const tomorrow = new Date();
      tomorrow.setHours(24, 0, 0, 0);
      midnight = setTimeout(
        () => {
          schedule();
          nextDay();
        },
        tomorrow.getTime() - Date.now() + 50,
      );
    };
    nextDay();
    void listen("usage-log-recorded", schedule)
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) => console.error("Cannot observe recent usage:", error));
    void listen<string>("proxy-request-activity", ({ payload }) => {
      if (disposed || !focusManager.isFocused()) return;
      void queryClient.invalidateQueries(
        { queryKey: proxyKeys.status },
        { cancelRefetch: false },
      );
      if (payload === "finished") {
        clearTimeout(pending);
        pending = setTimeout(refresh, 100);
      }
    })
      .then((off) => {
        if (disposed) off();
        else unlistenActivity = off;
      })
      .catch((error) =>
        console.error("Cannot observe active requests:", error),
      );
    return () => {
      disposed = true;
      unlisten?.();
      unlistenActivity?.();
      clearTimeout(pending);
      clearTimeout(midnight);
    };
  }, [active, queryClient]);

  return query;
}
