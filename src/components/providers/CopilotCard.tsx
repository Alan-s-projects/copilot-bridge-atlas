import { Github } from "lucide-react";
import type { Provider } from "@/types";
import { useCopilotAuth } from "./forms/hooks/useCopilotAuth";
import { Badge } from "@/components/ui/badge";
import CopilotQuotaFooter from "@/components/CopilotQuotaFooter";

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
    <section className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border bg-card px-6 py-4 shadow-sm">
      <div className="min-w-0 space-y-2">
        <h2 className="text-sm font-semibold">Provider</h2>
        <div className="flex items-center gap-3">
          <div className="rounded-xl bg-muted p-3">
            <Github className="h-7 w-7" />
          </div>
          <div className="space-y-1.5">
            <div className="flex flex-wrap items-center gap-3">
              <h3 className="text-lg font-semibold">GitHub Copilot</h3>
              {needsSetup && (
                <Badge variant="secondary">
                  {auth.isLoadingStatus ? "Checking account" : "Needs setup"}
                </Badge>
              )}
            </div>
            <p className="break-words text-sm text-muted-foreground">
              {!account
                ? "GitHub Copilot is signed out."
                : enabledModelCount === 0
                  ? "No models are enabled for Codex. Open Settings → Copilot to enable at least one."
                  : `${account.login} · ${enabledModelCount} models available to Codex`}
            </p>
          </div>
        </div>
      </div>
      {account && <CopilotQuotaFooter meta={provider.meta} />}
    </section>
  );
}
