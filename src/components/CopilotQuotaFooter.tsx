import type { ProviderMeta } from "@/types";
import { useCopilotQuota } from "@/lib/query/copilot";
import { extractErrorMessage } from "@/utils/errorUtils";

export default function CopilotQuotaFooter({ meta }: { meta?: ProviderMeta }) {
  const accountId =
    meta?.authBinding?.accountId ?? meta?.githubAccountId ?? null;
  const { data, error, isFetching } = useCopilotQuota(accountId);
  const used = Math.max(0, Math.min(100, data?.utilization ?? 0));
  return (
    <div className="space-y-1.5 text-xs">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
        <span className="font-medium">Copilot premium requests</span>
        {!error && data && (
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-muted-foreground">
            <span>
              {used.toFixed(1)}% used · {data.plan}
            </span>
            {data.resetDate && (
              <span>
                Resets {new Date(data.resetDate).toLocaleDateString("en-US")}
              </span>
            )}
          </div>
        )}
      </div>
      {error ? (
        <p role="alert" className="text-destructive">
          {extractErrorMessage(error)}
        </p>
      ) : data ? (
        <div className="h-1.5 overflow-hidden rounded-full bg-muted">
          <div
            className={`h-full ${used >= 90 ? "bg-red-500" : used >= 70 ? "bg-amber-500" : "bg-emerald-500"}`}
            style={{ width: `${used}%` }}
          />
        </div>
      ) : (
        <p className="text-muted-foreground">
          {isFetching
            ? "Loading quota…"
            : "Use Refresh overview to load quota."}
        </p>
      )}
    </div>
  );
}
