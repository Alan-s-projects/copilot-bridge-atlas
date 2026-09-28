import type { ProviderMeta } from "@/types";
import { useCopilotQuota } from "@/lib/query/copilot";
import { extractErrorMessage } from "@/utils/errorUtils";

export default function CopilotQuotaFooter({ meta }: { meta?: ProviderMeta }) {
  const accountId = meta?.authBinding?.accountId ?? null;
  const { data, error, isFetching } = useCopilotQuota(accountId);
  const used = Math.max(0, Math.min(100, data?.utilization ?? 0));
  return (
    <div className="flex max-w-full items-center gap-3 text-sm">
      {error ? (
        <p role="alert" className="text-destructive">
          {extractErrorMessage(error)}
        </p>
      ) : data ? (
        <>
          <div
            role="img"
            aria-label={`Copilot premium requests: ${used.toFixed(1)}% used`}
            className="relative h-16 w-16 shrink-0"
          >
            <svg
              aria-hidden
              viewBox="0 0 64 64"
              className="h-full w-full -rotate-90"
            >
              <circle
                cx="32"
                cy="32"
                r="27"
                fill="none"
                strokeWidth="5"
                className="stroke-muted"
              />
              <circle
                cx="32"
                cy="32"
                r="27"
                fill="none"
                strokeWidth="5"
                pathLength="100"
                strokeDasharray={`${used} 100`}
                strokeLinecap={used > 0 ? "round" : "butt"}
                className={
                  used >= 90
                    ? "stroke-red-500"
                    : used >= 70
                      ? "stroke-amber-500"
                      : "stroke-emerald-500"
                }
              />
            </svg>
            <span className="absolute inset-0 flex items-center justify-center text-xs font-semibold tabular-nums">
              {used.toFixed(1)}%
            </span>
          </div>
          <div className="min-w-0 space-y-1">
            <p className="font-medium">Copilot premium requests</p>
            <p className="text-xs text-muted-foreground">{data.plan}</p>
            {data.resetDate && (
              <p className="text-xs text-muted-foreground">
                Resets {new Date(data.resetDate).toLocaleDateString("en-US")}
              </p>
            )}
          </div>
        </>
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
