import { useMemo, useState } from "react";
import {
  Bar,
  BarChart,
  Line,
  LineChart,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
} from "recharts";
import { Loader2, RefreshCw } from "lucide-react";
import { useUsageTrends } from "@/lib/query/usage";
import { useUsageTrendGrouping } from "@/hooks/useUsageTrendGrouping";
import { TREND_INTERVALS, TREND_UNITS } from "@/lib/trendGrouping";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { fmtInt, fmtUsd, parseFiniteNumber } from "./format";
import { resolveUsageRange } from "@/lib/usageRange";
import { extractErrorMessage } from "@/utils/errorUtils";
import type { TrendUnit, UsageRangeSelection } from "@/types/usage";

interface UsageTrendChartProps {
  range: UsageRangeSelection;
  appType?: string;
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
}

interface UsageTrendStatLike {
  date: string;
  startDate?: number;
  endDate?: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
  totalCost: string | number;
  readCacheHitRate?: number | null;
  successRate?: number | null;
  incomplete?: boolean;
}

interface UsageTrendChartPoint {
  xKey: string;
  rawDate: string;
  label: string;
  tooltipLabel: string;
  hour: number;
  inputTokens: number | null;
  outputTokens: number | null;
  cacheCreationTokens: number | null;
  cacheReadTokens: number | null;
  cost: number | null;
  cacheHitRate: number | null;
  successRate: number | null;
}

export function buildUsageTrendChartData(
  trends: UsageTrendStatLike[] | undefined,
  options: {
    isHourly: boolean;
    dateLocale: string;
    startDate: number;
    endDate: number;
  },
): UsageTrendChartPoint[] {
  const { isHourly, dateLocale, startDate, endDate } = options;
  const endYear = new Date(endDate * 1000).getFullYear();
  const spansYears = new Date(startDate * 1000).getFullYear() !== endYear;
  const fullFormatter = new Intl.DateTimeFormat(dateLocale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    timeZoneName: "short",
  });
  const fullDate = (timestamp: number) =>
    fullFormatter.format(new Date(timestamp * 1000));
  const tickOptions: Intl.DateTimeFormatOptions = {
    month: "2-digit",
    day: "2-digit",
    hour: isHourly ? "2-digit" : undefined,
    minute: isHourly ? "2-digit" : undefined,
  };
  const shortTick = new Intl.DateTimeFormat(dateLocale, tickOptions);
  const yearTick = new Intl.DateTimeFormat(dateLocale, {
    ...tickOptions,
    year: "2-digit",
  });
  const rate = (value: number | null | undefined) =>
    value == null || !Number.isFinite(value)
      ? null
      : Math.max(0, Math.min(100, value * 100));
  return (trends ?? []).map((stat) => {
    const date = new Date(stat.date);
    const showYear = spansYears || date.getFullYear() !== endYear;
    const label = (showYear ? yearTick : shortTick).format(date);
    const tooltipLabel =
      stat.startDate != null && stat.endDate != null
        ? `${fullDate(Math.max(startDate, stat.startDate))} - ${fullDate(Math.min(endDate, stat.endDate - 1))}`
        : fullDate(date.getTime() / 1000);
    return {
      xKey: stat.date,
      rawDate: stat.date,
      label,
      tooltipLabel,
      hour: date.getHours(),
      inputTokens: stat.incomplete ? null : stat.totalInputTokens,
      outputTokens: stat.incomplete ? null : stat.totalOutputTokens,
      cacheCreationTokens: stat.incomplete
        ? null
        : stat.totalCacheCreationTokens,
      cacheReadTokens: stat.incomplete ? null : stat.totalCacheReadTokens,
      cost: stat.incomplete ? null : parseFiniteNumber(stat.totalCost),
      cacheHitRate: stat.incomplete ? null : rate(stat.readCacheHitRate),
      successRate: stat.incomplete ? null : rate(stat.successRate),
    };
  });
}

export function formatUsageTrendTickLabel(
  xKey: string,
  chartData: UsageTrendChartPoint[],
): string {
  return chartData.find((point) => point.xKey === xKey)?.label ?? xKey;
}

export function createUsageTrendTokenTickFormatter(
  locale: string,
): Intl.NumberFormat {
  return new Intl.NumberFormat(locale, {
    notation: "compact",
    compactDisplay: "short",
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  });
}

export function formatUsageTrendTokenTickLabel(
  value: unknown,
  formatter: Intl.NumberFormat,
): string {
  const number = parseFiniteNumber(value);
  if (number == null) return "--";
  return number === 0 ? "0" : formatter.format(number);
}

export function formatUsageTrendCostTickLabel(value: unknown): string {
  const number = parseFiniteNumber(value);
  if (number == null) return "--";
  const magnitude = Math.abs(number);
  const digits =
    magnitude > 0 && magnitude < 1
      ? Math.min(8, Math.max(2, 1 - Math.floor(Math.log10(magnitude))))
      : 2;
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    notation: magnitude >= 1000 ? "compact" : "standard",
    minimumFractionDigits: 0,
    maximumFractionDigits: digits,
  }).format(number);
}

const TOKEN_SERIES = [
  { key: "inputTokens", label: "Fresh Input", color: "#3b82f6" },
  { key: "cacheReadTokens", label: "Cached Input", color: "#8b62cf" },
  { key: "outputTokens", label: "Output", color: "#059669" },
  { key: "cacheCreationTokens", label: "Cache Write", color: "#c48a20" },
] as const;
const RATE_SERIES = [
  { key: "cacheHitRate", label: "Read cache hit", color: "#8b62cf" },
  { key: "successRate", label: "Success rate", color: "#059669" },
] as const;
type SeriesKey =
  | (typeof TOKEN_SERIES)[number]["key"]
  | (typeof RATE_SERIES)[number]["key"]
  | "cost";
const COST_SERIES = [{ key: "cost", label: "Cost", color: "#d94670" }] as const;

function SeriesLegend({
  series,
  hidden,
  onToggle,
}: {
  series: readonly { key: SeriesKey; label: string; color: string }[];
  hidden: Set<SeriesKey>;
  onToggle: (key: SeriesKey) => void;
}) {
  return (
    <div className="flex min-h-7 flex-wrap items-center gap-x-4 gap-y-1">
      {series.map(({ key, label, color }) => (
        <button
          key={key}
          type="button"
          aria-pressed={!hidden.has(key)}
          title={`${hidden.has(key) ? "Show" : "Hide"} ${label}`}
          onClick={() => onToggle(key)}
          className={`inline-flex items-center gap-2 py-1 text-xs ${hidden.has(key) ? "text-muted-foreground/50 line-through" : "text-muted-foreground"}`}
        >
          <span
            aria-hidden
            className="h-2.5 w-2.5 shrink-0 rounded-sm"
            style={{
              backgroundColor: color,
              opacity: hidden.has(key) ? 0.25 : 1,
            }}
          />
          {label}
        </button>
      ))}
    </div>
  );
}

function TrendTooltip({
  active,
  payload,
  rates = false,
}: {
  active?: boolean;
  payload?: readonly {
    dataKey?: string | number;
    name?: string | number;
    color?: string;
    value?: number;
    payload?: UsageTrendChartPoint;
  }[];
  rates?: boolean;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="max-w-sm rounded-md border bg-background p-3 text-xs shadow-md">
      <p className="mb-2 font-medium">{payload[0].payload?.tooltipLabel}</p>
      {payload
        .filter((item) => item.value != null)
        .map((item) => (
          <p key={item.dataKey} className="flex justify-between gap-4 py-0.5">
            <span style={{ color: item.color }}>{item.name}</span>
            <span className="font-medium tabular-nums">
              {rates
                ? `${item.value!.toFixed(1)}%`
                : item.dataKey === "cost"
                  ? fmtUsd(item.value, 6)
                  : fmtInt(item.value, "en-US")}
            </span>
          </p>
        ))}
    </div>
  );
}

export function UsageTrendChart({
  range,
  appType,
  providerName,
  model,
  refreshIntervalMs,
}: UsageTrendChartProps) {
  const preference = useUsageTrendGrouping();
  const { grouping } = preference;
  const fallbackRange = resolveUsageRange(range);
  const trends = useUsageTrends(
    range,
    { appType, providerName, model },
    {
      refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
      enabled: !preference.isLoading,
    },
    grouping,
  );
  const [hidden, setHidden] = useState<Set<SeriesKey>>(() => new Set());
  const startDate = trends.data?.startDate ?? fallbackRange.startDate;
  const endDate = trends.data?.endDate ?? fallbackRange.endDate;
  const toggle = (key: SeriesKey) =>
    setHidden((previous) => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const hourly = grouping.unit === "minute" || grouping.unit === "hour";
  const chartData = useMemo(
    () =>
      buildUsageTrendChartData(trends.data?.buckets, {
        isHourly: hourly,
        dateLocale: "en-US",
        startDate,
        endDate,
      }),
    [trends.data, hourly, startDate, endDate],
  );
  const tokenFormatter = useMemo(
    () => createUsageTrendTokenTickFormatter("en-US"),
    [],
  );
  const totalCost = trends.data?.hasIncompleteRollupData
    ? null
    : (parseFiniteNumber(trends.data?.totalCost) ??
      chartData.reduce((total, row) => total + (row.cost ?? 0), 0));
  const axisTick = { fill: "hsl(var(--muted-foreground))", fontSize: 11 };
  const xAxis = () => (
    <XAxis
      dataKey="xKey"
      tick={axisTick}
      axisLine={false}
      tickLine={false}
      tickMargin={10}
      minTickGap={24}
      tickFormatter={(value) =>
        formatUsageTrendTickLabel(String(value), chartData)
      }
      allowDuplicatedCategory={false}
    />
  );
  const grid = () => (
    <CartesianGrid
      vertical={false}
      stroke="hsl(var(--border))"
      opacity={0.55}
    />
  );
  const loading = preference.isLoading || trends.isLoading;

  return (
    <section
      aria-label="Usage Trends"
      className="min-w-0 overflow-hidden rounded-lg border border-border bg-card"
    >
      <div className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-4">
        <h3 className="text-lg font-semibold">Usage Trends</h3>
        <div className="flex items-center gap-2">
          <span className="mr-1 text-sm text-muted-foreground">Group by</span>
          <Select
            value={String(grouping.interval)}
            disabled={preference.isSaving || preference.isLoading}
            onValueChange={(value) =>
              preference.change({ ...grouping, interval: Number(value) })
            }
          >
            <SelectTrigger aria-label="Grouping interval" className="h-8 w-20">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TREND_INTERVALS.map((value) => (
                <SelectItem key={value} value={String(value)}>
                  {value}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={grouping.unit}
            disabled={preference.isSaving || preference.isLoading}
            onValueChange={(unit) =>
              preference.change({ ...grouping, unit: unit as TrendUnit })
            }
          >
            <SelectTrigger aria-label="Grouping unit" className="h-8 w-28">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TREND_UNITS.map(({ value, label }) => (
                <SelectItem key={value} value={value}>
                  {label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      {loading ? (
        <div
          role="status"
          aria-label="Loading trends"
          className="flex h-[1038px] items-center justify-center md:h-[712px]"
        >
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </div>
      ) : trends.isError ? (
        <div
          role="alert"
          className="flex min-h-40 items-center justify-center gap-3 p-6 text-sm"
        >
          <p>{extractErrorMessage(trends.error)}</p>
          <Button
            variant="outline"
            size="icon"
            title="Retry usage trends"
            aria-label="Retry usage trends"
            onClick={() => void trends.refetch()}
          >
            <RefreshCw className="h-4 w-4" />
          </Button>
        </div>
      ) : (
        <>
          {trends.data?.hasIncompleteRollupData && (
            <p
              role="status"
              className="border-b bg-amber-500/10 px-4 py-3 text-sm text-amber-800 dark:text-amber-300"
            >
              Some intervals contain daily-only archived data. Finer or
              partial-day values are unavailable; select complete days with Day,
              Week, or Month grouping.
            </p>
          )}
          <div className="grid min-w-0 md:grid-cols-2">
            <section
              aria-label="Cost chart"
              className="min-w-0 border-b p-4 md:border-r"
            >
              <div className="mb-2 flex items-center justify-between gap-3">
                <h4 className="text-sm font-semibold">Cost</h4>
                <span className="text-sm font-semibold tabular-nums">
                  {fmtUsd(totalCost, 2)}
                </span>
              </div>
              <SeriesLegend
                series={COST_SERIES}
                hidden={hidden}
                onToggle={toggle}
              />
              <div className="mt-3 h-56 w-full">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart
                    data={chartData}
                    margin={{ top: 8, right: 8, left: 0, bottom: 4 }}
                    barCategoryGap="25%"
                  >
                    {grid()}
                    {xAxis()}
                    <YAxis
                      width={72}
                      tick={axisTick}
                      axisLine={false}
                      tickLine={false}
                      tickFormatter={formatUsageTrendCostTickLabel}
                    />
                    <Tooltip content={<TrendTooltip />} />
                    <Bar
                      dataKey="cost"
                      name="Cost"
                      fill="#d94670"
                      hide={hidden.has("cost")}
                      maxBarSize={40}
                      radius={[3, 3, 0, 0]}
                      isAnimationActive={false}
                    />
                  </BarChart>
                </ResponsiveContainer>
              </div>
            </section>
            <section aria-label="Rates chart" className="min-w-0 border-b p-4">
              <div className="mb-2 flex items-center justify-between">
                <h4 className="text-sm font-semibold">Rates</h4>
                <span className="text-xs text-muted-foreground">0–100%</span>
              </div>
              <SeriesLegend
                series={RATE_SERIES}
                hidden={hidden}
                onToggle={toggle}
              />
              <div className="mt-3 h-56 w-full">
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart
                    data={chartData}
                    margin={{ top: 8, right: 12, left: 0, bottom: 4 }}
                  >
                    {grid()}
                    {xAxis()}
                    <YAxis
                      width={48}
                      domain={[0, 100]}
                      ticks={[0, 50, 100]}
                      tick={axisTick}
                      axisLine={false}
                      tickLine={false}
                      tickFormatter={(value) => `${value}%`}
                    />
                    <Tooltip content={<TrendTooltip rates />} />
                    {RATE_SERIES.map(({ key, label, color }) => (
                      <Line
                        key={key}
                        type="linear"
                        dataKey={key}
                        name={label}
                        stroke={color}
                        strokeWidth={2}
                        strokeDasharray={
                          key === "cacheHitRate" ? "4 3" : undefined
                        }
                        dot={{ r: 3, fill: color, strokeWidth: 0 }}
                        activeDot={{ r: 5 }}
                        hide={hidden.has(key)}
                        connectNulls={false}
                        isAnimationActive={false}
                      />
                    ))}
                  </LineChart>
                </ResponsiveContainer>
              </div>
            </section>
            <section
              aria-label="Token counts chart"
              className="min-w-0 p-4 md:col-span-2"
            >
              <h4 className="mb-2 text-sm font-semibold">Token counts</h4>
              <SeriesLegend
                series={TOKEN_SERIES}
                hidden={hidden}
                onToggle={toggle}
              />
              <div className="mt-3 h-64 w-full">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart
                    data={chartData}
                    margin={{ top: 8, right: 8, left: 0, bottom: 4 }}
                    barCategoryGap="25%"
                  >
                    {grid()}
                    {xAxis()}
                    <YAxis
                      width={64}
                      tick={axisTick}
                      axisLine={false}
                      tickLine={false}
                      tickFormatter={(value) =>
                        formatUsageTrendTokenTickLabel(value, tokenFormatter)
                      }
                    />
                    <Tooltip content={<TrendTooltip />} />
                    {TOKEN_SERIES.map(({ key, label, color }) => (
                      <Bar
                        key={key}
                        stackId="tokens"
                        dataKey={key}
                        name={label}
                        fill={color}
                        hide={hidden.has(key)}
                        maxBarSize={40}
                        isAnimationActive={false}
                      />
                    ))}
                  </BarChart>
                </ResponsiveContainer>
              </div>
            </section>
          </div>
          <div className="flex justify-between gap-3 px-4 pb-4 text-xs text-muted-foreground">
            <span>
              {chartData.length.toLocaleString("en-US")}{" "}
              {chartData.length === 1 ? "interval" : "intervals"}
            </span>
            <span>Local time</span>
          </div>
        </>
      )}
    </section>
  );
}
