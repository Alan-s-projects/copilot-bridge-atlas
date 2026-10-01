import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { useUsageSummary } from "@/lib/query/usage";
import { Loader2 } from "lucide-react";
import { fmtUsd, formatTokensShort, parseFiniteNumber } from "./format";
import { InputUsageHeading, InputUsageValue } from "./InputUsage";
import type { UsageRangeSelection, UsageSummary } from "@/types/usage";

interface UsageHeroProps {
  range: UsageRangeSelection;
  appType?: "codex";
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
}

// Size the shared desktop tracks from token values, not the secondary row's labels.
const DETAIL_GRID_CLASS_NAME =
  "grid min-w-0 grid-cols-2 gap-x-4 gap-y-3 sm:grid-cols-[minmax(0,2fr)_repeat(2,minmax(0,1fr))] md:col-span-3 md:grid-cols-subgrid";

export function UsageHero({
  range,
  appType = "codex",
  providerName,
  model,
  refreshIntervalMs,
}: UsageHeroProps) {
  const { data: summary, isLoading } = useUsageSummary(
    range,
    { appType, providerName, model },
    {
      refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
    },
  );

  return <UsageSummaryCard summary={summary} isLoading={isLoading} />;
}

export function UsageSummaryCard({
  summary,
  isLoading = false,
  title,
  notice,
}: {
  summary?: UsageSummary;
  isLoading?: boolean;
  title?: string;
  notice?: string;
}) {
  const { t } = useTranslation();
  const input = summary?.totalInputTokens ?? 0;
  const output = summary?.totalOutputTokens ?? 0;
  const cacheRead = summary?.totalCacheReadTokens ?? 0;
  const cacheWrite = summary?.totalCacheCreationTokens ?? 0;
  const hitRate = summary?.cacheHitRate ?? 0;
  const totalCost = parseFiniteNumber(summary?.totalCost);
  const requests = summary?.totalRequests ?? 0;
  const latency = parseFiniteNumber(summary?.avgLatencyMs);
  const outputTps = parseFiniteNumber(summary?.outputTokensPerSecond);

  const successRate =
    requests > 0 && summary ? `${summary.successRate.toFixed(1)}%` : "--";
  const averageLatency =
    requests > 0 && latency != null ? `${(latency / 1000).toFixed(2)}s` : "--";
  const outputSpeed =
    requests > 0 && outputTps != null && outputTps > 0
      ? `${outputTps.toFixed(1)} tps`
      : "--";

  const hitPercent = Math.max(0, Math.min(100, hitRate * 100));
  const hitPercentLabel = hitPercent.toFixed(hitPercent >= 99.95 ? 0 : 1);
  return (
    <Card
      role="region"
      aria-label={title ?? t("usage.summary", "Usage summary")}
      className="min-w-0"
    >
      <CardContent className="p-0">
        {(title || !isLoading) && (
          <h2
            className={title ? "px-4 pt-4 text-base font-semibold" : "sr-only"}
          >
            {title ?? t("usage.summary", "Usage summary")}
          </h2>
        )}
        {notice && (
          <p role="alert" className="px-4 pt-3 text-sm text-destructive">
            {notice}
          </p>
        )}
        {isLoading ? (
          <div
            role="status"
            aria-label="Loading usage"
            className="flex min-h-24 items-center justify-center"
          >
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground/50" />
          </div>
        ) : (
          <div className="divide-y divide-border md:grid md:grid-cols-[9rem_auto_auto_auto] md:gap-x-4">
            <div className="grid min-w-0 grid-cols-1 items-center gap-4 p-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,5fr)] md:col-span-4 md:grid-cols-subgrid">
              <dl className="min-w-0">
                <SummaryRow
                  label={t("usage.totalCost", "Total Cost")}
                  value={fmtUsd(totalCost, 0)}
                  primary
                />
              </dl>
              <div
                role="group"
                aria-label={t("usage.tokenDetails", "Token details")}
                className="min-w-0 sm:border-l sm:border-border sm:pl-4 md:col-span-3 md:grid md:grid-cols-subgrid"
              >
                <dl className={DETAIL_GRID_CLASS_NAME}>
                  <div className="col-span-2 flex min-w-0 flex-col items-start gap-1 sm:col-span-1">
                    <dt className="w-full text-xs text-muted-foreground md:[contain:inline-size]">
                      <InputUsageHeading inline />
                    </dt>
                    <dd className="text-sm font-medium">
                      <InputUsageValue
                        fresh={input}
                        cached={cacheRead}
                        hit={`${hitPercentLabel}%`}
                        hitRate={
                          input + cacheRead + cacheWrite > 0
                            ? hitPercent / 100
                            : null
                        }
                        compact
                      />
                    </dd>
                  </div>
                  <SummaryRow
                    label={t("usage.output", "Output")}
                    value={formatTokensShort(output)}
                  />
                  <SummaryRow
                    label={t("usage.cacheWrite", "Cache Write")}
                    value={formatTokensShort(cacheWrite)}
                  />
                </dl>
              </div>
            </div>
            <div className="grid min-w-0 grid-cols-1 items-center gap-4 p-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,5fr)] md:col-span-4 md:grid-cols-subgrid">
              <dl className="min-w-0">
                <SummaryRow
                  label={t("usage.requests", "Requests")}
                  value={requests.toLocaleString("en-US")}
                  primary
                />
              </dl>
              <div
                role="group"
                aria-label={t("usage.requestDetails", "Request details")}
                className="min-w-0 sm:border-l sm:border-border sm:pl-4 md:col-span-3 md:grid md:grid-cols-subgrid"
              >
                <dl className={DETAIL_GRID_CLASS_NAME}>
                  <SummaryRow
                    label={t("usage.avgLatency", "Average Latency")}
                    value={averageLatency}
                    className="col-span-2 sm:col-span-1 md:[contain:inline-size]"
                  />
                  <SummaryRow
                    label={t("usage.outputSpeed", "Output Speed")}
                    value={outputSpeed}
                    className="md:[contain:inline-size]"
                    title="Output tokens / generation time. Excludes first-token delay when available; older rollups use request latency."
                  />
                  <SummaryRow
                    label={t("usage.successRate", "Success Rate")}
                    value={successRate}
                    className="md:[contain:inline-size]"
                    percentage
                  />
                </dl>
              </div>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

interface SummaryRowProps {
  label: string;
  value: string;
  percentage?: boolean;
  title?: string;
  primary?: boolean;
  className?: string;
}

function SummaryRow({
  label,
  value,
  percentage = false,
  title,
  primary = false,
  className = "",
}: SummaryRowProps) {
  return (
    <div className={`flex min-w-0 flex-col items-start gap-1 ${className}`}>
      <dt className="w-full text-xs text-muted-foreground md:[contain:inline-size]">
        {label}
      </dt>
      <dd
        title={title}
        className={`tabular-nums ${primary ? "break-all text-2xl font-semibold leading-8" : "text-sm font-medium"} ${
          percentage && value !== "--"
            ? "text-emerald-700 dark:text-emerald-400"
            : ""
        }`}
      >
        {value}
      </dd>
    </div>
  );
}
