import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useRequestLogs } from "@/lib/query/usage";
import type { LogFilters, UsageRangeSelection } from "@/types/usage";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { RequestLogsGrid } from "./RequestLogsGrid";

interface RequestLogTableProps {
  columns?: string[];
  range: UsageRangeSelection;
  rangeLabel?: string;
  appType?: string;
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
  onRangeChange?: (range: UsageRangeSelection) => void;
}

export function RequestLogTable({
  range,
  appType: dashboardAppType,
  providerName,
  model,
  refreshIntervalMs,
  columns,
}: RequestLogTableProps) {
  const { t } = useTranslation();

  // Model and range selection are shared with the dashboard.
  const [page, setPage] = useState(0);
  const [pageInput, setPageInput] = useState("");
  const pageSize = 20;

  const effectiveFilters: LogFilters = {
    appType:
      dashboardAppType && dashboardAppType !== "all"
        ? dashboardAppType
        : undefined,
    providerName,
    model,
  };

  const { data: result, isLoading } = useRequestLogs({
    filters: effectiveFilters,
    range,
    page,
    pageSize,
    options: {
      refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
    },
  });

  const logs = result?.data ?? [];
  const total = result?.total ?? 0;
  const totalPages = Math.ceil(total / pageSize);

  useEffect(() => {
    setPage(0);
  }, [
    dashboardAppType,
    providerName,
    model,
    range.customEndDate,
    range.customStartDate,
    range.preset,
  ]);

  const handleGoToPage = () => {
    const trimmed = pageInput.trim();
    if (!/^\d+$/.test(trimmed)) return;
    const parsed = Number(trimmed);
    if (parsed < 1 || parsed > totalPages) return;
    setPage(parsed - 1);
    setPageInput("");
  };

  return (
    <div className="space-y-4">
      {isLoading ? (
        <div className="h-[400px] animate-pulse rounded bg-gray-100" />
      ) : (
        <>
          <RequestLogsGrid logs={logs} columns={columns} />

          <div className="flex items-center justify-between text-sm text-muted-foreground">
            <span>{t("usage.totalRecords", { total })}</span>
            <div className="flex items-center gap-1">
              <Button
                size="sm"
                variant="outline"
                disabled={page === 0}
                onClick={() => setPage((p) => Math.max(0, p - 1))}
              >
                <ChevronLeft className="h-4 w-4" />
              </Button>
              {(() => {
                const pages: (number | string)[] = [];
                if (totalPages <= 9) {
                  for (let i = 0; i < totalPages; i++) pages.push(i);
                } else {
                  const pageSet = new Set<number>();
                  for (let i = 0; i < 3; i++) pageSet.add(i);
                  for (let i = totalPages - 3; i < totalPages; i++)
                    pageSet.add(i);
                  for (
                    let i = Math.max(0, page - 1);
                    i <= Math.min(totalPages - 1, page + 1);
                    i++
                  )
                    pageSet.add(i);
                  const sorted = Array.from(pageSet).sort((a, b) => a - b);
                  for (let i = 0; i < sorted.length; i++) {
                    if (i > 0 && sorted[i] - sorted[i - 1] > 1) {
                      pages.push(`ellipsis-${i}`);
                    }
                    pages.push(sorted[i]);
                  }
                }
                return pages.map((p) =>
                  typeof p === "string" ? (
                    <span key={p} className="px-2 text-muted-foreground">
                      ...
                    </span>
                  ) : (
                    <Button
                      key={p}
                      variant={p === page ? "default" : "outline"}
                      size="sm"
                      className="h-8 w-8 p-0"
                      onClick={() => setPage(p)}
                    >
                      {p + 1}
                    </Button>
                  ),
                );
              })()}
              <Button
                size="sm"
                variant="outline"
                disabled={page >= totalPages - 1}
                onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
              >
                <ChevronRight className="h-4 w-4" />
              </Button>
              <div className="flex items-center gap-1 ml-2">
                <Input
                  type="text"
                  value={pageInput}
                  onChange={(e) => setPageInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleGoToPage();
                  }}
                  placeholder={t("usage.pageInputPlaceholder")}
                  className="h-8 w-16 text-center text-xs"
                />
                <Button variant="outline" size="sm" onClick={handleGoToPage}>
                  {t("usage.goToPage")}
                </Button>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
