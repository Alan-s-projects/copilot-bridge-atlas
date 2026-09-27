import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api/settings";
import { normalizeTrendGrouping } from "@/lib/trendGrouping";
import type { TrendGrouping } from "@/types/usage";

export const trendGroupingKey = ["settings", "usageTrendGrouping"] as const;

export function useUsageTrendGrouping() {
  const client = useQueryClient();
  const query = useQuery({
    queryKey: trendGroupingKey,
    queryFn: settingsApi.getUsageTrendGrouping,
    staleTime: Infinity,
    retry: false,
  });
  const save = useMutation({
    mutationFn: settingsApi.setUsageTrendGrouping,
    onMutate: async (next: TrendGrouping) => {
      await client.cancelQueries({ queryKey: trendGroupingKey });
      const previous = client.getQueryData(trendGroupingKey);
      client.setQueryData(trendGroupingKey, next);
      return { previous };
    },
    onSuccess: (saved) => client.setQueryData(trendGroupingKey, saved),
    onError: (_error, _next, context) => {
      client.setQueryData(trendGroupingKey, context?.previous);
      toast.error("Could not save trend grouping.");
    },
  });
  return {
    grouping: normalizeTrendGrouping(query.isError ? undefined : query.data),
    isLoading: query.isLoading,
    isSaving: save.isPending,
    change: (next: TrendGrouping) => save.mutate(next),
  };
}
