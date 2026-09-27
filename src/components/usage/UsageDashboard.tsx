import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { UsageHero } from "./UsageHero";
import { UsageTrendChart } from "./UsageTrendChart";
import { RequestLogTable } from "./RequestLogTable";
import { ModelStatsTable } from "./ModelStatsTable";
import { type UsageRangeSelection } from "@/types/usage";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  BarChart3,
  Coins,
  ListFilter,
  RefreshCw,
} from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useQueryClient } from "@tanstack/react-query";
import {
  usageKeys,
  useModelStats,
  useUnpricedModelUsage,
} from "@/lib/query/usage";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";
import { PricingConfigPanel } from "@/components/usage/PricingConfigPanel";
import { getUsageRangePresetLabel, resolveUsageRange } from "@/lib/usageRange";
import { UsageDateRangePicker } from "./UsageDateRangePicker";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { formatTokensShort } from "./format";
import type { UnpricedModelUsage } from "@/types/usage";

const DEFAULT_REFRESH_INTERVAL_MS = 30000;
const REFRESH_INTERVAL_OPTIONS_MS = [0, 5000, 10000, 30000, 60000] as const;
type RefreshIntervalOption = (typeof REFRESH_INTERVAL_OPTIONS_MS)[number];

const isRefreshIntervalOption = (
  value: number | undefined,
): value is RefreshIntervalOption =>
  REFRESH_INTERVAL_OPTIONS_MS.includes(value as RefreshIntervalOption);

const normalizeRefreshInterval = (value: number | undefined) =>
  isRefreshIntervalOption(value) ? value : DEFAULT_REFRESH_INTERVAL_MS;

// Select 的 "all" 哨兵和用户自定义名称同处一个值域——真有模型叫 "all"
// 就会撞名（重复 value、选中即清空筛选）。动态选项统一加前缀编码隔离值域。
const DYNAMIC_OPTION_PREFIX = "v:";
const encodeOptionValue = (name: string) => `${DYNAMIC_OPTION_PREFIX}${name}`;
const decodeOptionValue = (value: string) =>
  value === "all" ? undefined : value.slice(DYNAMIC_OPTION_PREFIX.length);

interface UsageDashboardProps {
  refreshIntervalMs?: number;
  onRefreshIntervalChange?: (next: number) => Promise<boolean> | boolean | void;
}

export function UsageDashboard({
  refreshIntervalMs: savedRefreshIntervalMs,
  onRefreshIntervalChange,
}: UsageDashboardProps = {}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [range, setRange] = useState<UsageRangeSelection>({ preset: "today" });
  const appType = "codex";
  const [model, setModel] = useState<string | undefined>(undefined);
  const [activeTab, setActiveTab] = useState("logs");
  const [refreshIntervalMs, setRefreshIntervalMs] = useState(() =>
    normalizeRefreshInterval(savedRefreshIntervalMs),
  );

  useEffect(() => {
    setRefreshIntervalMs(normalizeRefreshInterval(savedRefreshIntervalMs));
  }, [savedRefreshIntervalMs]);

  // 后端写入新日志时 emit `usage-log-recorded`，本 hook 立刻 invalidate 所有
  // usage 查询，实现实时刷新（仅在 Dashboard 挂载时生效，离开页面自动取消监听）
  useUsageEventBridge();

  const changeRefreshInterval = async (next: number) => {
    const normalized = normalizeRefreshInterval(next);
    const previous = refreshIntervalMs;
    setRefreshIntervalMs(normalized);
    queryClient.invalidateQueries({ queryKey: usageKeys.all });
    try {
      const saved = await onRefreshIntervalChange?.(normalized);
      if (saved === false) {
        setRefreshIntervalMs(previous);
      }
    } catch (error) {
      console.error(
        "[UsageDashboard] Failed to persist refresh interval",
        error,
      );
      setRefreshIntervalMs(previous);
    }
  };

  const locale = "en-US";
  const resolvedRange = useMemo(() => resolveUsageRange(range), [range]);
  const rangeLabel = useMemo(() => {
    if (range.preset !== "custom") {
      return getUsageRangePresetLabel(range.preset, t);
    }

    const startStr = new Date(resolvedRange.startDate * 1000).toLocaleString(
      locale,
    );

    if (range.liveEndTime) {
      return `${startStr} → ${t("usage.liveEndTimeNow", "Now")}`;
    }

    const endStr = new Date(resolvedRange.endDate * 1000).toLocaleString(
      locale,
    );
    return `${startStr} - ${endStr}`;
  }, [locale, range, resolvedRange.endDate, resolvedRange.startDate, t]);

  // The options query shares the model table's cache key, so it must respect
  // the dashboard interval even when automatic refresh is disabled.
  const optionsRefetch = {
    refetchInterval:
      refreshIntervalMs > 0 ? refreshIntervalMs : (false as const),
  };
  const { data: modelOptionsData } = useModelStats(
    range,
    { appType },
    optionsRefetch,
  );
  const unpricedUsageQuery = useUnpricedModelUsage(
    range,
    { appType, model },
    optionsRefetch,
  );

  const modelOptions = useMemo(() => {
    const names = new Set<string>();
    for (const stat of modelOptionsData ?? []) {
      names.add(stat.model);
    }
    if (model) names.add(model);
    return Array.from(names);
  }, [modelOptionsData, model]);

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4 }}
      className="space-y-8 pb-8"
    >
      <div className="flex flex-col lg:flex-row lg:items-end justify-between gap-4 mb-2">
        <div className="flex flex-col gap-1">
          <h1 className="text-2xl font-bold tracking-tight">
            {t("usage.title")}
          </h1>
          <p className="text-sm text-muted-foreground">{t("usage.subtitle")}</p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Select
            value={model != null ? encodeOptionValue(model) : "all"}
            onValueChange={(v) => setModel(decodeOptionValue(v))}
          >
            <SelectTrigger
              className="h-9 w-[100px] bg-background text-xs focus:border-border-default [&>span]:min-w-0 [&>span]:truncate"
              title={model ?? t("usage.filterByModel")}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent className="max-w-[280px]">
              <SelectItem value="all">{t("usage.allModels")}</SelectItem>
              {modelOptions.map((name) => (
                <SelectItem
                  key={name}
                  value={encodeOptionValue(name)}
                  title={name}
                  className="[&>span]:min-w-0 [&>span]:truncate"
                >
                  {name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          <div className="flex items-center gap-2 ml-auto lg:ml-0">
            <Select
              value={String(refreshIntervalMs)}
              onValueChange={(v) => changeRefreshInterval(Number(v))}
            >
              <SelectTrigger
                className="h-9 w-[100px] bg-background text-xs focus:border-border-default"
                title={t("usage.refreshInterval")}
                aria-label={t("usage.refreshInterval")}
              >
                <span className="flex items-center gap-2">
                  <RefreshCw className="h-3.5 w-3.5 shrink-0" />
                  <SelectValue />
                </span>
              </SelectTrigger>
              <SelectContent>
                {REFRESH_INTERVAL_OPTIONS_MS.map((ms) => (
                  <SelectItem key={ms} value={String(ms)}>
                    {ms > 0 ? `${ms / 1000}s` : t("usage.refreshOff")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <UsageDateRangePicker
              selection={range}
              triggerLabel={rangeLabel}
              onApply={(nextRange) => setRange(nextRange)}
            />
          </div>
        </div>
      </div>

      {unpricedUsageQuery.isError ? (
        <Alert className="border-amber-500/40 bg-amber-500/10">
          <AlertTriangle className="h-4 w-4" />
          <AlertDescription className="flex flex-wrap items-center justify-between gap-3">
            <p>
              {t(
                "usage.priceCheckFailed",
                "Could not check prices for this usage range. No missing-price result is available.",
              )}
            </p>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={unpricedUsageQuery.isFetching}
              onClick={() => void unpricedUsageQuery.refetch()}
            >
              {unpricedUsageQuery.isFetching
                ? t("common.loading", "Loading...")
                : t("usage.retryPriceCheck", "Retry price check")}
            </Button>
          </AlertDescription>
        </Alert>
      ) : (
        unpricedUsageQuery.data &&
        unpricedUsageQuery.data.length > 0 && (
          <UnpricedModelsAlert
            models={unpricedUsageQuery.data}
            onReviewPricing={() => setActiveTab("pricing")}
          />
        )
      )}

      <UsageHero
        range={range}
        appType={appType}
        model={model}
        refreshIntervalMs={refreshIntervalMs}
      />

      <UsageTrendChart
        range={range}
        rangeLabel={rangeLabel}
        appType={appType}
        model={model}
        refreshIntervalMs={refreshIntervalMs}
      />

      <div className="space-y-4">
        <Tabs value={activeTab} onValueChange={setActiveTab} className="w-full">
          <div className="flex items-center justify-between mb-4">
            <TabsList className="bg-muted/50">
              <TabsTrigger value="logs" className="gap-2">
                <ListFilter className="h-4 w-4" />
                {t("usage.requestLogs")}
              </TabsTrigger>
              <TabsTrigger value="models" className="gap-2">
                <BarChart3 className="h-4 w-4" />
                {t("usage.modelStats")}
              </TabsTrigger>
              <TabsTrigger value="pricing" className="gap-2">
                <Coins className="h-4 w-4" />
                {t("usage.costPricing", "Cost Pricing")}
              </TabsTrigger>
            </TabsList>
          </div>

          <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.2 }}
          >
            <TabsContent value="logs" className="mt-0">
              <RequestLogTable
                range={range}
                rangeLabel={rangeLabel}
                appType={appType}
                model={model}
                refreshIntervalMs={refreshIntervalMs}
                onRangeChange={setRange}
              />
            </TabsContent>
            <TabsContent value="models" className="mt-0">
              <ModelStatsTable
                range={range}
                appType={appType}
                model={model}
                refreshIntervalMs={refreshIntervalMs}
              />
            </TabsContent>
            <TabsContent value="pricing" className="mt-0">
              <PricingConfigPanel />
            </TabsContent>
          </motion.div>
        </Tabs>
      </div>
    </motion.div>
  );
}

function UnpricedModelsAlert({
  models,
  onReviewPricing,
}: {
  models: UnpricedModelUsage[];
  onReviewPricing: () => void;
}) {
  const { t } = useTranslation();

  return (
    <Alert className="border-amber-500/40 bg-amber-500/10">
      <AlertTriangle className="h-4 w-4" />
      <AlertDescription className="min-w-0 space-y-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <p className="font-medium">
              {t("usage.unpricedModelsTitle", "Unpriced model usage")}
            </p>
            <p>
              {t(
                "usage.unpricedModelsMessage",
                "No bundled or custom price matches these models. Their usage cost is recorded as $0 until a matching price is added.",
              )}
            </p>
          </div>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={onReviewPricing}
          >
            {t("usage.reviewPricing", "Review pricing")}
          </Button>
        </div>

        <div className="space-y-2">
          {models.map((usage) => (
            <dl
              key={usage.model}
              className="grid min-w-0 grid-cols-2 gap-x-3 gap-y-2 border-t border-amber-500/20 pt-2 sm:grid-cols-3 xl:grid-cols-[minmax(0,1.6fr)_repeat(5,minmax(0,1fr))]"
            >
              <div className="min-w-0">
                <dt className="text-xs text-muted-foreground">
                  {t("usage.model", "Model")}
                </dt>
                <dd className="break-all font-mono text-sm font-medium">
                  {usage.model}
                </dd>
                <dd className="text-xs text-muted-foreground">
                  {t("usage.requestCount", {
                    count: usage.requestCount,
                    defaultValue: "{{count}} requests",
                  })}
                </dd>
              </div>
              <UnpricedMetric
                label={t("usage.freshInput", "Fresh Input")}
                value={formatTokensShort(usage.freshInputTokens)}
              />
              <UnpricedMetric
                label={t("usage.cacheRead", "Cached Input")}
                value={formatTokensShort(usage.cacheReadTokens)}
              />
              <UnpricedMetric
                label={t("usage.output", "Output")}
                value={formatTokensShort(usage.outputTokens)}
              />
              <UnpricedMetric
                label={t("usage.cacheWrite", "Cache Write")}
                value={formatTokensShort(usage.cacheCreationTokens ?? 0)}
              />
              <UnpricedMetric
                label={t("usage.cacheHitRate", "Read Cache Hit Rate")}
                value={`${(usage.cacheHitRate * 100).toFixed(1)}%`}
              />
            </dl>
          ))}
        </div>

        <p className="text-xs text-muted-foreground">
          {t(
            "usage.unpricedModelsCatalogNote",
            "New Copilot models may not have a bundled price yet. No price is inferred from another model or vendor.",
          )}
        </p>
      </AlertDescription>
    </Alert>
  );
}

function UnpricedMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="break-words text-sm font-medium tabular-nums">{value}</dd>
    </div>
  );
}
