import { memo } from "react";
import { useTranslation } from "react-i18next";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { isUnpricedUsage, type RequestLog } from "@/types/usage";
import { InputUsageHeading, RequestInputValue } from "./InputUsage";
import {
  ReasoningEffortHeading,
  ReasoningEffortValue,
} from "./ReasoningEffort";
import {
  formatCostBreakdown,
  formatOutputTokensPerSecond,
  fmtInt,
  fmtUsd,
  parseFiniteNumber,
} from "./format";

export const RequestLogsGrid = memo(function RequestLogsGrid({
  logs,
  caption,
  showTokenDetails = true,
}: {
  logs: RequestLog[];
  caption?: string;
  showTokenDetails?: boolean;
}) {
  const { t } = useTranslation();
  const locale = "en-US";

  return (
    <div className="overflow-x-auto rounded-lg border border-border/50 bg-card/40 backdrop-blur-sm">
      <Table>
        {caption && <caption className="sr-only">{caption}</caption>}
        <TableHeader>
          <TableRow>
            <TableHead className="text-center whitespace-nowrap">
              {t("usage.time", "Time")}
            </TableHead>
            <TableHead className="text-center whitespace-nowrap">
              {t("usage.billingModel", "Billing Model")}
            </TableHead>
            <TableHead className="text-center whitespace-nowrap">
              <ReasoningEffortHeading />
            </TableHead>
            <TableHead className="text-center whitespace-nowrap">
              {t("usage.status", "Status")}
            </TableHead>
            {showTokenDetails && (
              <>
                <TableHead className="text-center whitespace-nowrap min-w-44">
                  <InputUsageHeading />
                </TableHead>
                <TableHead className="text-center whitespace-nowrap">
                  {t("usage.outputTokens", "Output")}
                </TableHead>
                <TableHead className="text-center whitespace-nowrap">
                  {t("usage.cacheWrite", "Cache Write")}
                </TableHead>
              </>
            )}
            <TableHead className="text-center whitespace-nowrap">
              {t("usage.timingInfo", "Duration")}
            </TableHead>
            <TableHead className="text-center whitespace-nowrap">
              {t("usage.cost", "Cost")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {logs.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={showTokenDetails ? 9 : 6}
                className="text-center text-muted-foreground"
              >
                {t("usage.noData", "No data")}
              </TableCell>
            </TableRow>
          ) : (
            logs.map((log) => {
              const unpriced = isUnpricedUsage(log);
              const multiplier = parseFiniteNumber(log.costMultiplier);
              const tps = showTokenDetails
                ? formatOutputTokensPerSecond(log)
                : null;
              return (
                <TableRow key={log.requestId}>
                  <TableCell className="text-center whitespace-nowrap text-xs px-1.5">
                    {new Date(log.createdAt * 1000).toLocaleString(locale, {
                      month: "2-digit",
                      day: "2-digit",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </TableCell>
                  <TableCell className="text-center font-mono text-xs max-w-[200px]">
                    <div
                      className="truncate"
                      title={
                        log.requestModel && log.requestModel !== log.model
                          ? `${log.requestModel} \u2192 ${log.model}`
                          : log.model
                      }
                    >
                      {log.requestModel && log.requestModel !== log.model ? (
                        <span>
                          {log.requestModel}
                          <span className="text-muted-foreground">
                            {" \u2192 "}
                            {log.model}
                          </span>
                        </span>
                      ) : (
                        log.model
                      )}
                    </div>
                  </TableCell>
                  <TableCell className="text-center whitespace-nowrap px-1.5">
                    <ReasoningEffortValue log={log} />
                  </TableCell>
                  <TableCell className="text-center">
                    <span
                      className={
                        log.statusCode >= 200 && log.statusCode < 300
                          ? "text-green-600"
                          : "text-red-600"
                      }
                    >
                      {log.statusCode}
                    </span>
                  </TableCell>
                  {showTokenDetails && (
                    <>
                      <TableCell className="text-center whitespace-nowrap px-1.5">
                        <RequestInputValue log={log} />
                      </TableCell>
                      <TableCell className="text-center px-1.5">
                        <div className="tabular-nums">
                          {fmtInt(log.outputTokens, locale)}
                          {tps != null && (
                            <span className="text-muted-foreground text-xs">
                              /{tps} tps
                            </span>
                          )}
                        </div>
                      </TableCell>
                      <TableCell className="text-center px-1.5 tabular-nums">
                        {fmtInt(log.cacheCreationTokens, locale)}
                      </TableCell>
                    </>
                  )}
                  <TableCell className="text-center whitespace-nowrap text-xs tabular-nums">
                    {(log.latencyMs / 1000).toFixed(1)}s
                  </TableCell>
                  <TableCell
                    className="text-center px-1.5"
                    title={formatCostBreakdown(log)}
                  >
                    <div
                      className={`font-medium tabular-nums ${unpriced ? "text-muted-foreground" : ""}`}
                    >
                      {unpriced
                        ? t("usage.unpriced", "Unpriced")
                        : fmtUsd(log.totalCostUsd, 4)}
                    </div>
                    {multiplier != null && multiplier !== 1 && (
                      <div className="text-[11px] text-muted-foreground">
                        {"\u00d7"}
                        {multiplier.toFixed(2)}
                      </div>
                    )}
                  </TableCell>
                </TableRow>
              );
            })
          )}
        </TableBody>
      </Table>
    </div>
  );
});
