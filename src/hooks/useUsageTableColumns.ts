import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  settingsApi,
  type UsageTableColumns,
  type UsageTableName,
} from "@/lib/api/settings";

const queryKey = ["settings", "usageTableColumns"] as const;

export function useUsageTableColumns() {
  const client = useQueryClient();
  const query = useQuery({
    queryKey,
    queryFn: settingsApi.getUsageTableColumns,
    staleTime: Infinity,
    retry: false,
  });
  const mutation = useMutation({
    mutationFn: ({
      table,
      columns,
    }: {
      table: UsageTableName;
      columns: string[];
    }) => settingsApi.setUsageTableColumns(table, columns),
    onMutate: async ({ table, columns }) => {
      await client.cancelQueries({ queryKey });
      const previous = client.getQueryData<UsageTableColumns>(queryKey);
      client.setQueryData(queryKey, { ...previous, [table]: columns });
      return { previous };
    },
    onSuccess: (saved) => client.setQueryData(queryKey, saved),
    onError: (_error, _variables, context) => {
      client.setQueryData(queryKey, context?.previous);
      toast.error("Could not save column settings.");
    },
  });
  return {
    ...query,
    data: query.isError ? {} : (query.data ?? {}),
    saving: mutation.isPending,
    change: async (table: UsageTableName, columns: string[]) => {
      await mutation.mutateAsync({ table, columns });
    },
  };
}
