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
import { isUnpricedUsage, type RequestLogRow } from "@/types/usage";
import { REQUEST_LOG_COLUMNS, visibleColumns } from "./tableColumns";
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
  columns,
}: {
  logs: RequestLogRow[];
  caption?: string;
  showTokenDetails?: boolean;
  columns?: string[];
}) {
  const { t } = useTranslation();
  const locale = "en-US";
  const visible = new Set(
    visibleColumns(
      columns,
      REQUEST_LOG_COLUMNS.filter(
        ({ id }) =>
          showTokenDetails || !["input", "output", "cacheWrite"].includes(id),
      ),
    ),
  );

  return (
    <div className="overflow-x-auto rounded-lg border border-border/50 bg-card/40 backdrop-blur-sm">
      <Table>
        {caption && <caption className="sr-only">{caption}</caption>}
        <TableHeader>
          <TableRow>
            <TableHead
              hidden={!visible.has("time")}
              className="text-center whitespace-nowrap"
            >
              {t("usage.time", "Time")}
            </TableHead>
            <TableHead
              hidden={!visible.has("model")}
              className="text-center whitespace-nowrap"
            >
              {t("usage.billingModel", "Billing Model")}
            </TableHead>
            <TableHead
              hidden={!visible.has("reasoning")}
              className="text-center whitespace-nowrap"
            >
              <ReasoningEffortHeading />
            </TableHead>
            <TableHead
              hidden={!visible.has("status")}
              className="text-center whitespace-nowrap"
            >
              {t("usage.status", "Status")}
            </TableHead>
            {showTokenDetails && (
              <>
                <TableHead
                  hidden={!visible.has("input")}
                  className="text-center whitespace-nowrap min-w-44"
                >
                  <InputUsageHeading />
                </TableHead>
                <TableHead
                  hidden={!visible.has("output")}
                  className="text-center whitespace-nowrap"
                >
                  {t("usage.outputTokens", "Output")}
                </TableHead>
                <TableHead
                  hidden={!visible.has("cacheWrite")}
                  className="text-center whitespace-nowrap"
                >
                  {t("usage.cacheWrite", "Cache Write")}
                </TableHead>
              </>
            )}
            <TableHead
              hidden={!visible.has("duration")}
              className="text-center whitespace-nowrap"
            >
              {t("usage.timingInfo", "Duration")}
            </TableHead>
            <TableHead
              hidden={!visible.has("pricingTier")}
              className="text-center whitespace-nowrap"
            >
              Pricing Tier
            </TableHead>
            <TableHead
              hidden={!visible.has("cost")}
              className="text-center whitespace-nowrap"
            >
              {t("usage.cost", "Cost")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {logs.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={visible.size}
                className="text-center text-muted-foreground"
              >
                {t("usage.noData", "No data")}
              </TableCell>
            </TableRow>
          ) : (
            logs.map((log) => {
              const pending = "pending" in log;
              const unpriced = !pending && isUnpricedUsage(log);
              const multiplier = pending
                ? null
                : parseFiniteNumber(log.costMultiplier);
              const tps =
                showTokenDetails && !pending
                  ? formatOutputTokensPerSecond(log)
                  : null;
              return (
                <TableRow key={log.requestId}>
                  <TableCell
                    hidden={!visible.has("time")}
                    className="text-center whitespace-nowrap text-xs px-1.5"
                  >
                    {new Date(log.createdAt * 1000).toLocaleString(locale, {
                      month: "2-digit",
                      day: "2-digit",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("model")}
                    className="text-center font-mono text-xs max-w-[200px]"
                  >
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
                  <TableCell
                    hidden={!visible.has("reasoning")}
                    className="text-center whitespace-nowrap px-1.5"
                  >
                    <ReasoningEffortValue log={log} />
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("status")}
                    className="text-center"
                  >
                    <span
                      className={
                        pending
                          ? "text-amber-700 dark:text-amber-400"
                          : log.statusCode >= 200 && log.statusCode < 300
                            ? "text-green-600"
                            : "text-red-600"
                      }
                    >
                      {pending ? "Pending" : log.statusCode}
                    </span>
                  </TableCell>
                  {showTokenDetails && (
                    <>
                      <TableCell
                        hidden={!visible.has("input")}
                        className="text-center whitespace-nowrap px-1.5"
                      >
                        {pending ? "N/A" : <RequestInputValue log={log} />}
                      </TableCell>
                      <TableCell
                        hidden={!visible.has("output")}
                        className="text-center px-1.5"
                      >
                        <div className="tabular-nums">
                          {pending ? "N/A" : fmtInt(log.outputTokens, locale)}
                          {tps != null && (
                            <span className="text-muted-foreground text-xs">
                              /{tps} tps
                            </span>
                          )}
                        </div>
                      </TableCell>
                      <TableCell
                        hidden={!visible.has("cacheWrite")}
                        className="text-center px-1.5 tabular-nums"
                      >
                        {pending
                          ? "N/A"
                          : fmtInt(log.cacheCreationTokens, locale)}
                      </TableCell>
                    </>
                  )}
                  <TableCell
                    hidden={!visible.has("duration")}
                    className="text-center whitespace-nowrap text-xs tabular-nums"
                  >
                    {pending ? "N/A" : `${(log.latencyMs / 1000).toFixed(1)}s`}
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("pricingTier")}
                    className="text-center whitespace-nowrap px-1.5"
                  >
                    <span
                      className="text-xs"
                      title={
                        pending
                          ? "Request is in progress"
                          : log.pricingTier
                            ? "Tier recorded when this request was priced"
                            : "No pricing tier recorded"
                      }
                    >
                      {pending
                        ? "N/A"
                        : log.pricingTier === "long_context"
                          ? "Long context"
                          : log.pricingTier === "default"
                            ? "Default"
                            : "--"}
                    </span>
                  </TableCell>
                  <TableCell
                    hidden={!visible.has("cost")}
                    className="text-center px-1.5"
                    title={pending ? undefined : formatCostBreakdown(log)}
                  >
                    <div
                      className={`font-medium tabular-nums ${unpriced ? "text-muted-foreground" : ""}`}
                    >
                      {pending
                        ? "N/A"
                        : unpriced
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
