import { useTranslation } from "react-i18next";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useModelStats } from "@/lib/query/usage";
import {
  fmtInt,
  fmtUsd,
  formatReadCacheHitRate,
  formatTokensShort,
} from "./format";
import { InputUsageHeading, InputUsageValue } from "./InputUsage";
import type { UsageRangeSelection } from "@/types/usage";

interface ModelStatsTableProps {
  range: UsageRangeSelection;
  appType?: string;
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
}

export function ModelStatsTable({
  range,
  appType,
  providerName,
  model,
  refreshIntervalMs,
}: ModelStatsTableProps) {
  const { t } = useTranslation();
  const { data: stats, isLoading } = useModelStats(
    range,
    { appType, providerName, model },
    {
      refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
    },
  );

  if (isLoading) {
    return <div className="h-[400px] animate-pulse rounded bg-gray-100" />;
  }

  return (
    <div className="rounded-lg border border-border/50 bg-card/40 backdrop-blur-sm overflow-hidden">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("usage.billingModel", "Billing Model")}</TableHead>
            <TableHead className="text-right">
              {t("usage.requests", "Requests")}
            </TableHead>
            <TableHead className="text-right">
              Total <InputUsageHeading />
            </TableHead>
            <TableHead className="text-right">Total Output</TableHead>
            <TableHead className="text-right">Total Cache Write</TableHead>
            <TableHead className="text-right">
              {t("usage.totalCost", "Total Cost")}
            </TableHead>
            <TableHead className="text-right">
              {t("usage.avgCost", "Average Cost")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {stats?.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={7}
                className="text-center text-muted-foreground"
              >
                {t("usage.noData", "No data")}
              </TableCell>
            </TableRow>
          ) : (
            stats?.map((stat) => (
              <TableRow key={stat.model}>
                <TableCell className="font-mono text-sm">
                  {stat.model}
                </TableCell>
                <TableCell className="text-right">
                  {stat.requestCount.toLocaleString()}
                </TableCell>
                <TableCell className="text-right">
                  <InputUsageValue
                    compact
                    compactDecimals={1}
                    fresh={stat.totalInputTokens}
                    cached={stat.totalCacheReadTokens}
                    hit={formatReadCacheHitRate({
                      appType: "codex",
                      inputTokens: stat.totalInputTokens,
                      freshInputTokens: stat.totalInputTokens,
                      cacheReadTokens: stat.totalCacheReadTokens,
                      cacheCreationTokens: stat.totalCacheCreationTokens,
                    })}
                  />
                </TableCell>
                <TableCell
                  className="text-right tabular-nums"
                  title={fmtInt(stat.totalOutputTokens)}
                >
                  {formatTokensShort(stat.totalOutputTokens, 1)}
                </TableCell>
                <TableCell
                  className="text-right tabular-nums"
                  title={fmtInt(stat.totalCacheCreationTokens)}
                >
                  {formatTokensShort(stat.totalCacheCreationTokens, 1)}
                </TableCell>
                <TableCell className="text-right">
                  {fmtUsd(stat.totalCost, 4)}
                </TableCell>
                <TableCell className="text-right">
                  {fmtUsd(stat.avgCostPerRequest, 6)}
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </div>
  );
}
