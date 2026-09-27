import {
  settingsApi,
  type UsageTableColumns,
  type UsageTableName,
} from "@/lib/api/settings";
import { useSavedPreference } from "./useSavedPreference";

const queryKey = ["settings", "usageTableColumns"] as const;

export function useUsageTableColumns() {
  const { query, mutation, saving } = useSavedPreference({
    queryKey,
    load: settingsApi.getUsageTableColumns,
    save: ({ table, columns }: { table: UsageTableName; columns: string[] }) =>
      settingsApi.setUsageTableColumns(table, columns),
    optimisticUpdate: (
      previous: UsageTableColumns | undefined,
      { table, columns },
    ) => ({ ...previous, [table]: columns }),
    errorMessage: "Could not save column settings.",
  });
  return {
    ...query,
    data: query.isError ? {} : (query.data ?? {}),
    saving,
    change: async (table: UsageTableName, columns: string[]) => {
      await mutation.mutateAsync({ table, columns });
    },
  };
}
