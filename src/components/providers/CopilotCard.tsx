import { Github } from "lucide-react";
import type { Provider } from "@/types";
import { useCopilotAuth } from "./forms/hooks/useCopilotAuth";
import { Badge } from "@/components/ui/badge";
import CopilotQuotaFooter from "@/components/CopilotQuotaFooter";
import { HealthCheckButton } from "./HealthCheckButton";

export function CopilotCard({ provider }: { provider: Provider }) {
  const auth = useCopilotAuth();
  const accountId =
    provider.meta?.authBinding?.accountId ??
    provider.meta?.githubAccountId ??
    auth.defaultAccountId;
  const account = accountId
    ? auth.accounts.find((item) => item.id === accountId)
    : auth.accounts[0];
  const models =
    (provider.settingsConfig.modelCatalog as { models?: unknown[] } | undefined)
      ?.models ?? [];
  const enabledModelCount = models.filter(
    (model) =>
      !model ||
      typeof model !== "object" ||
      ((model as { enabled?: unknown }).enabled !== false &&
        (model as { available?: unknown }).available !== false),
  ).length;
  const needsSetup = !account || enabledModelCount === 0;
  return (
    <section className="space-y-3 border-b border-border pb-3">
      <h2 className="sr-only">Provider</h2>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <Github aria-hidden className="h-6 w-6 shrink-0" />
          <div className="min-w-0 space-y-0.5">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-base font-semibold">GitHub Copilot</h3>
              {needsSetup && (
                <Badge variant="secondary">
                  {auth.isLoadingStatus ? "Checking account" : "Needs setup"}
                </Badge>
              )}
            </div>
            <p className="break-words text-xs text-muted-foreground">
              {!account
                ? "GitHub Copilot is signed out."
                : enabledModelCount === 0
                  ? "No models are enabled for Codex. Open Settings → Copilot to enable at least one."
                  : `${account.login} · ${enabledModelCount} models available to Codex`}
            </p>
          </div>
        </div>
        <HealthCheckButton providerId={provider.id} />
      </div>
      {account && <CopilotQuotaFooter meta={provider.meta} />}
    </section>
  );
}
