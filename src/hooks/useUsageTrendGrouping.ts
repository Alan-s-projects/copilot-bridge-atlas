import { settingsApi } from "@/lib/api/settings";
import { normalizeTrendGrouping } from "@/lib/trendGrouping";
import type { TrendGrouping } from "@/types/usage";
import { useSavedPreference } from "./useSavedPreference";

export const trendGroupingKey = ["settings", "usageTrendGrouping"] as const;

export function useUsageTrendGrouping() {
  const { query, mutation, saving } = useSavedPreference({
    queryKey: trendGroupingKey,
    load: settingsApi.getUsageTrendGrouping,
    save: settingsApi.setUsageTrendGrouping,
    optimisticUpdate: (_previous, next: TrendGrouping) => next,
    errorMessage: "Could not save trend grouping.",
  });
  return {
    grouping: normalizeTrendGrouping(query.isError ? undefined : query.data),
    isLoading: query.isLoading,
    isSaving: saving,
    change: mutation.mutate,
  };
}
