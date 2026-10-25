import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { UsageHero } from "./UsageHero";
import { UsageTrendChart } from "./UsageTrendChart";
import { RequestLogTable } from "./RequestLogTable";
import { ModelStatsTable } from "./ModelStatsTable";
import { type UsageRangeSelection } from "@/types/usage";
import { motion } from "framer-motion";
import { AlertTriangle, BarChart3, Coins, ListFilter } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  useUsageClients,
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
import type { UsageTableColumns, UsageTableName } from "@/lib/api/settings";
import { TableColumnsMenu } from "./TableColumnsMenu";
import { MODEL_STATS_COLUMNS, REQUEST_LOG_COLUMNS } from "./tableColumns";

const REFRESH_INTERVAL_MS = 30_000;
const FILTER_TRIGGER_CLASS =
  "h-9 w-full gap-2 bg-background text-sm [&>span:nth-of-type(2)]:min-w-0 [&>span:nth-of-type(2)]:flex-1 [&>span:nth-of-type(2)]:truncate [&>span:nth-of-type(2)]:text-left";

// The "all" sentinel of Select and the user-defined name are in the same value range - there is really a model called "all"
// There will be name collision (duplicate value, clear the filter when selected). Dynamic options are uniformly prefixed to encode isolated value ranges.
const DYNAMIC_OPTION_PREFIX = "v:";
const encodeOptionValue = (name: string) => `${DYNAMIC_OPTION_PREFIX}${name}`;
const decodeOptionValue = (value: string) =>
  value === "all" ? undefined : value.slice(DYNAMIC_OPTION_PREFIX.length);

interface UsageDashboardProps {
  savedRange?: UsageRangeSelection;
  onRangeChange?: (range: UsageRangeSelection) => void;
  tableColumns?: UsageTableColumns;
  onTableColumnsChange?: (
    table: UsageTableName,
    columns: string[],
  ) => Promise<void>;
  columnsSaving?: boolean;
}

export function UsageDashboard({
  tableColumns,
  onTableColumnsChange,
  columnsSaving,
  savedRange,
  onRangeChange,
}: UsageDashboardProps = {}) {
  const { t } = useTranslation();
  const [localRange, setLocalRange] = useState<UsageRangeSelection>({
    preset: "today",
  });
  const range = savedRange ?? localRange;
  const setRange = onRangeChange ?? setLocalRange;
  const appType = "codex";
  const [clientName, setClientName] = useState<string | undefined>(undefined);
  const [model, setModel] = useState<string | undefined>(undefined);
  const [activeTab, setActiveTab] = useState("logs");
  const refreshIntervalMs = REFRESH_INTERVAL_MS;

  // When the backend writes a new log, emit `usage-log-recorded`, this hook immediately invalidates all
  // usage query to achieve real-time refresh (only takes effect when the Dashboard is mounted, and monitoring is automatically canceled when leaving the page)
  useUsageEventBridge();

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

  const optionsRefetch = { refetchInterval: REFRESH_INTERVAL_MS };
  const clientsQuery = useUsageClients();
  const clientOptions = useMemo(() => {
    const names = new Set(clientsQuery.data ?? []);
    if (clientName !== undefined) names.add(clientName);
    return Array.from(names).sort((a, b) => a.localeCompare(b));
  }, [clientsQuery.data, clientName]);
  const { data: modelOptionsData } = useModelStats(
    range,
    { appType },
    optionsRefetch,
  );
  const unpricedUsageQuery = useUnpricedModelUsage(
    range,
    { appType, model, clientName },
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
    <>
      <h1 className="sr-only">{t("usage.title")}</h1>
      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.4 }}
        className="space-y-8 pb-8"
      >
        <div
          className="flex flex-wrap items-center justify-between gap-3"
          aria-label="Usage filters"
        >
          <div className="grid w-full grid-cols-1 gap-2 min-[480px]:grid-cols-2 md:flex md:w-auto">
            <Select
              value={
                clientName !== undefined ? encodeOptionValue(clientName) : "all"
              }
              onValueChange={(value) => setClientName(decodeOptionValue(value))}
            >
              <SelectTrigger
                className={`${FILTER_TRIGGER_CLASS} md:w-[230px]`}
                aria-label={t("usage.filterByClient", "Filter by client")}
                title={clientName ?? t("usage.allClients", "All clients")}
              >
                <span className="shrink-0 text-xs text-muted-foreground">
                  {t("usage.client", "Client")}
                </span>
                <SelectValue>
                  {clientName ?? t("common.all", "All")}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">
                  {t("usage.allClients", "All clients")}
                </SelectItem>
                {clientOptions.map((name) => (
                  <SelectItem
                    key={encodeOptionValue(name)}
                    value={encodeOptionValue(name)}
                  >
                    {name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select
              value={model !== undefined ? encodeOptionValue(model) : "all"}
              onValueChange={(value) => setModel(decodeOptionValue(value))}
            >
              <SelectTrigger
                className={`${FILTER_TRIGGER_CLASS} md:w-[280px]`}
                aria-label={t("usage.filterByModel")}
                title={model ?? t("usage.allModels")}
              >
                <span className="shrink-0 text-xs text-muted-foreground">
                  {t("usage.model", "Model")}
                </span>
                <SelectValue>{model ?? t("common.all", "All")}</SelectValue>
              </SelectTrigger>
              <SelectContent className="max-w-[360px]">
                <SelectItem value="all">{t("usage.allModels")}</SelectItem>
                {modelOptions.map((name) => (
                  <SelectItem
                    key={encodeOptionValue(name)}
                    value={encodeOptionValue(name)}
                    title={name}
                  >
                    {name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="ml-auto shrink-0 [&>button]:min-w-[150px]">
            <UsageDateRangePicker
              selection={range}
              triggerLabel={rangeLabel}
              onApply={setRange}
            />
          </div>
        </div>
        {clientsQuery.isError && (
          <Alert>
            <AlertTriangle className="h-4 w-4" />
            <AlertDescription className="flex items-center justify-between gap-3">
              <span>Could not load client filters.</span>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void clientsQuery.refetch()}
              >
                Retry
              </Button>
            </AlertDescription>
          </Alert>
        )}

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
          clientName={clientName}
          refreshIntervalMs={refreshIntervalMs}
        />

        <UsageTrendChart
          range={range}
          appType={appType}
          model={model}
          clientName={clientName}
          refreshIntervalMs={refreshIntervalMs}
        />

        <div className="space-y-4">
          <Tabs
            value={activeTab}
            onValueChange={setActiveTab}
            className="w-full"
          >
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
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
              {activeTab !== "pricing" && (
                <TableColumnsMenu
                  options={
                    activeTab === "logs"
                      ? REQUEST_LOG_COLUMNS
                      : MODEL_STATS_COLUMNS
                  }
                  selected={
                    activeTab === "logs"
                      ? tableColumns?.requestLogs
                      : tableColumns?.modelStats
                  }
                  saving={columnsSaving}
                  onChange={
                    onTableColumnsChange
                      ? (columns) =>
                          onTableColumnsChange(
                            activeTab === "logs" ? "requestLogs" : "modelStats",
                            columns,
                          )
                      : undefined
                  }
                />
              )}
            </div>

            <motion.div
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.2 }}
            >
              <TabsContent value="logs" className="mt-0">
                <RequestLogTable
                  columns={tableColumns?.requestLogs}
                  range={range}
                  appType={appType}
                  model={model}
                  clientName={clientName}
                  refreshIntervalMs={refreshIntervalMs}
                />
              </TabsContent>
              <TabsContent value="models" className="mt-0">
                <ModelStatsTable
                  columns={tableColumns?.modelStats}
                  range={range}
                  appType={appType}
                  model={model}
                  clientName={clientName}
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
    </>
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
