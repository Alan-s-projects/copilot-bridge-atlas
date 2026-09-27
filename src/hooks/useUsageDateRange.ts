import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api/settings";
import type { UsageRangeSelection } from "@/types/usage";

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
  const client = useQueryClient();
  const query = useQuery({
    queryKey,
    queryFn: settingsApi.getUsageDateRange,
    retry: false,
    staleTime: Infinity,
  });
  const save = useMutation({
    mutationKey: queryKey,
    scope: { id: "usage-date-range" },
    mutationFn: settingsApi.setUsageDateRange,
    onMutate: async (range: UsageRangeSelection) => {
      await client.cancelQueries({ queryKey });
      const previous = client.getQueryData(queryKey);
      client.setQueryData(queryKey, range);
      return { previous };
    },
    onSuccess: (saved) => {
      if (client.isMutating({ mutationKey: queryKey }) === 1)
        client.setQueryData(queryKey, saved);
    },
    onError: (_error, _range, context) => {
      if (client.isMutating({ mutationKey: queryKey }) === 1)
        client.setQueryData(queryKey, context?.previous);
      toast.error("Could not save the usage date range.");
    },
    onSettled: () => {
      if (client.isMutating({ mutationKey: queryKey }) === 1)
        return client.invalidateQueries({ queryKey });
    },
  });
  return {
    range: normalizeUsageDateRange(query.isError ? undefined : query.data),
    isLoading: query.isLoading,
    change: save.mutate,
  };
}
