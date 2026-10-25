import { useProvidersQuery } from "@/lib/query";
import { useGlobalProxyConfig } from "@/lib/query/proxy";
import { isValidModelId } from "@/utils/codexModelCatalog";
import type { ClaudeGatewayConfig } from "@/types/proxy";

export const claudeRoles = ["sonnet", "opus", "haiku", "fable"] as const;
export type ClaudeRole = (typeof claudeRoles)[number];
export const defaultClaude: ClaudeGatewayConfig = {
  address: "127.0.0.1",
  port: 15723,
  sonnet: "",
  opus: "",
  haiku: "",
  fable: "",
};

export function useClaudeMappings() {
  const config = useGlobalProxyConfig();
  const providers = useProvidersQuery();
  const provider =
    providers.data?.providers[providers.data.currentProviderId] ??
    Object.values(providers.data?.providers ?? {})[0];
  const models: string[] = (
    provider?.settingsConfig?.modelCatalog?.models ?? []
  )
    .filter(
      (m: { model?: unknown; enabled?: boolean; available?: boolean }) =>
        typeof m.model === "string" &&
        isValidModelId(m.model) &&
        m.enabled !== false &&
        m.available !== false,
    )
    .map((m: { model: string }) => m.model);
  const mappings = config.data?.claude ?? defaultClaude;
  const ready =
    !!config.data && !!providers.data && !config.isError && !providers.isError;
  const missing = claudeRoles.filter(
    (role) => !mappings[role] || !models.includes(mappings[role]),
  );
  return {
    mappings,
    models,
    ready,
    count: ready ? missing.length : undefined,
    error: config.isError || providers.isError,
    retry: () => Promise.all([config.refetch(), providers.refetch()]),
  };
}
