import { memo, useMemo } from "react";
import {
  OVERVIEW_REQUEST_LIMIT,
  useBridgeOverview,
} from "@/hooks/useBridgeOverview";
import type { ProxyStatus } from "@/types/proxy";
import type { RequestLog } from "@/types/usage";
import { RequestLogsGrid } from "@/components/usage/RequestLogsGrid";
import { UsageSummaryCard } from "@/components/usage/UsageHero";

const RecentRequests = memo(function RecentRequests({
  logs,
  active,
}: {
  logs: RequestLog[];
  active: NonNullable<ProxyStatus["active_requests"]>;
}) {
  const rows = useMemo(
    () =>
      [
        ...active.map((request) => ({ ...request, pending: true as const })),
        ...logs,
      ].slice(0, OVERVIEW_REQUEST_LIMIT),
    [active, logs],
  );
  return (
    <RequestLogsGrid
      logs={rows}
      caption={`Latest ${OVERVIEW_REQUEST_LIMIT} requests`}
      showTokenDetails={false}
    />
  );
});

export function BridgeOverview({ status }: { status?: ProxyStatus }) {
  const overview = useBridgeOverview();
  const summary = overview.data?.summary;
  return (
    <section className="space-y-5" aria-label="Bridge overview">
      <UsageSummaryCard
        title="Today's usage"
        summary={summary}
        isLoading={!summary && !overview.error}
        notice={
          overview.error
            ? `Usage could not be refreshed. ${overview.data ? "Showing the last snapshot." : "Open Usage to retry."}`
            : undefined
        }
      />
      <section
        className="space-y-5 rounded-xl border border-border bg-card p-6 shadow-sm"
        aria-labelledby="bridge-requests-title"
      >
        <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2">
          <h2 id="bridge-requests-title" className="text-base font-semibold">
            Requests
          </h2>
          <p className="text-sm text-muted-foreground">
            Active requests:{" "}
            <span className="font-medium text-foreground">
              {status?.active_connections ?? "—"}
            </span>
          </p>
        </div>
        {overview.data || status?.active_requests?.length ? (
          <RecentRequests
            logs={overview.data?.recent.data ?? []}
            active={status?.active_requests ?? []}
          />
        ) : !overview.error ? (
          <p className="text-sm text-muted-foreground">
            Loading recent requests…
          </p>
        ) : null}
      </section>
    </section>
  );
}
