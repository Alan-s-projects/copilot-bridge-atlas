import type { TrendGrouping, TrendUnit } from "@/types/usage";

export const TREND_INTERVALS = [1, 3, 5, 7, 10, 15, 30, 60] as const;
export const TREND_UNITS: { value: TrendUnit; label: string }[] = [
  { value: "minute", label: "Minute" },
  { value: "hour", label: "Hour" },
  { value: "day", label: "Day" },
  { value: "week", label: "Week" },
  { value: "month", label: "Month" },
];
export const DEFAULT_TREND_GROUPING: TrendGrouping = {
  interval: 1,
  unit: "day",
};

export function normalizeTrendGrouping(value: unknown): TrendGrouping {
  if (typeof value !== "object" || value === null)
    return DEFAULT_TREND_GROUPING;
  const candidate = value as TrendGrouping;
  return TREND_INTERVALS.some((interval) => interval === candidate.interval) &&
    TREND_UNITS.some(({ value }) => value === candidate.unit)
    ? { interval: candidate.interval, unit: candidate.unit }
    : DEFAULT_TREND_GROUPING;
}
