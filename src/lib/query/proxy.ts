import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { proxyApi } from "@/lib/api/proxy";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import type { GlobalProxyConfig } from "@/types/proxy";
import { useWindowActive } from "@/lib/windowActivity";

export const proxyKeys = {
  status: ["proxyStatus"] as const,
  globalConfig: ["globalProxyConfig"] as const,
};

// ========== Proxy Server Status Hooks ==========

/**
 * Get proxy server status
 */
export function useProxyStatusQuery() {
  const active = useWindowActive();
  return useQuery({
    queryKey: proxyKeys.status,
    queryFn: () => proxyApi.getProxyStatus(),
    enabled: active,
    // Lightweight in-memory status only; never poll while the window is inactive.
    refetchInterval: (query) =>
      active && query.state.data?.running ? 5000 : false,
    refetchIntervalInBackground: false,
    // Keep previous data to avoid flickering
    placeholderData: (previousData) => previousData,
  });
}

// ========== v3+ Global/Application Level Configuration Hooks ==========

/**
 * Get global proxy configuration
 */
export function useGlobalProxyConfig() {
  return useQuery({
    queryKey: proxyKeys.globalConfig,
    queryFn: () => proxyApi.getGlobalProxyConfig(),
  });
}

/**
 * Update global proxy configuration
 */
export function useUpdateGlobalProxyConfig({
  showSuccessToast = true,
}: { showSuccessToast?: boolean } = {}) {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: (config: GlobalProxyConfig) =>
      proxyApi.updateGlobalProxyConfig(config),
    onSuccess: () => {
      if (showSuccessToast) {
        toast.success(t("proxy.settings.toast.saved"), { closeButton: true });
      }
      queryClient.invalidateQueries({ queryKey: proxyKeys.globalConfig });
      queryClient.invalidateQueries({ queryKey: proxyKeys.status });
    },
    onError: (error: Error) => {
      toast.error(
        t("proxy.settings.toast.saveFailed", { error: error.message }),
      );
    },
  });
}
