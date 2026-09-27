import { settingsApi } from "@/lib/api/settings";
import type { UsageRangeSelection } from "@/types/usage";
import { useSavedPreference } from "./useSavedPreference";

const queryKey = ["settings", "usageDateRange"] as const;
const DEFAULT_RANGE: UsageRangeSelection = { preset: "today" };

export function normalizeUsageDateRange(value: unknown): UsageRangeSelection {
  if (!value || typeof value !== "object") return DEFAULT_RANGE;
  const range = value as UsageRangeSelection;
  if (["today", "1d", "7d", "14d", "30d"].includes(range.preset))
    return { preset: range.preset };
  if (
    range.preset === "custom" &&
    Number.isSafeInteger(range.customStartDate) &&
    (range.liveEndTime ||
      (Number.isSafeInteger(range.customEndDate) &&
        range.customEndDate! >= range.customStartDate!))
  ) {
    return {
      preset: "custom",
      customStartDate: range.customStartDate,
      customEndDate: range.customEndDate,
      liveEndTime: !!range.liveEndTime,
    };
  }
  return DEFAULT_RANGE;
}

export function useUsageDateRange() {
  const { query, mutation } = useSavedPreference({
    queryKey,
    load: settingsApi.getUsageDateRange,
    save: settingsApi.setUsageDateRange,
    optimisticUpdate: (_previous, next: UsageRangeSelection) => next,
    errorMessage: "Could not save the usage date range.",
  });
  return {
    range: normalizeUsageDateRange(query.isError ? undefined : query.data),
    isLoading: query.isLoading,
    change: mutation.mutate,
  };
}
