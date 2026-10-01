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
import { getReadCacheHitRate, type UsageRangeSelection } from "@/types/usage";
import { MODEL_STATS_COLUMNS, visibleColumns } from "./tableColumns";

interface ModelStatsTableProps {
  columns?: string[];
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
  columns,
}: ModelStatsTableProps) {
  const { t } = useTranslation();
  const visible = new Set(visibleColumns(columns, MODEL_STATS_COLUMNS));
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
            <TableHead hidden={!visible.has("model")}>
              {t("usage.billingModel", "Billing Model")}
            </TableHead>
            <TableHead hidden={!visible.has("requests")} className="text-right">
              {t("usage.requests", "Requests")}
            </TableHead>
            <TableHead hidden={!visible.has("input")} className="text-right">
              Total <InputUsageHeading />
            </TableHead>
            <TableHead hidden={!visible.has("output")} className="text-right">
              Total Output
            </TableHead>
            <TableHead
              hidden={!visible.has("cacheWrite")}
              className="text-right"
            >
              Total Cache Write
            </TableHead>
            <TableHead hidden={!visible.has("cost")} className="text-right">
              {t("usage.totalCost", "Total Cost")}
            </TableHead>
            <TableHead
              hidden={!visible.has("averageCost")}
              className="text-right"
            >
              {t("usage.avgCost", "Average Cost")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {stats?.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={visible.size}
                className="text-center text-muted-foreground"
              >
                {t("usage.noData", "No data")}
              </TableCell>
            </TableRow>
          ) : (
            stats?.map((stat) => {
              const inputUsage = {
                freshInputTokens: stat.totalInputTokens,
                cacheReadTokens: stat.totalCacheReadTokens,
                cacheCreationTokens: stat.totalCacheCreationTokens,
              };
              return (
                <TableRow key={stat.model}>
                  <TableCell
                    hidden={!visible.has("model")}
                    className="font-mono text-sm"
                  >
                    {stat.model}
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("requests")}
                    className="text-right"
                  >
                    {stat.requestCount.toLocaleString()}
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("input")}
                    className="text-right"
                  >
                    <InputUsageValue
                      compact
                      compactDecimals={1}
                      fresh={stat.totalInputTokens}
                      cached={stat.totalCacheReadTokens}
                      hit={formatReadCacheHitRate(inputUsage)}
                      hitRate={getReadCacheHitRate(inputUsage)}
                    />
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("output")}
                    className="text-right tabular-nums"
                    title={fmtInt(stat.totalOutputTokens)}
                  >
                    {formatTokensShort(stat.totalOutputTokens, 1)}
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("cacheWrite")}
                    className="text-right tabular-nums"
                    title={fmtInt(stat.totalCacheCreationTokens)}
                  >
                    {formatTokensShort(stat.totalCacheCreationTokens, 1)}
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("cost")}
                    className="text-right"
                  >
                    {fmtUsd(stat.totalCost, 4)}
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("averageCost")}
                    className="text-right"
                  >
                    {fmtUsd(stat.avgCostPerRequest, 6)}
                  </TableCell>
                </TableRow>
              );
            })
          )}
        </TableBody>
      </Table>
    </div>
  );
}
