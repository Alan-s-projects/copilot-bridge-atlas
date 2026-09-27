import { Loader2 } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { UsageDashboard } from "./UsageDashboard";
import { useUsageTableColumns } from "@/hooks/useUsageTableColumns";
import { useUsageDateRange } from "@/hooks/useUsageDateRange";

export function UsagePage() {
  const { settings, isLoading, autoSaveSettings } = useSettings();
  const columns = useUsageTableColumns();
  const dateRange = useUsageDateRange();

  if (isLoading || !settings || columns.isLoading || dateRange.isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="px-4 pb-6 pt-4 sm:px-6">
      <UsageDashboard
        savedRange={dateRange.range}
        onRangeChange={dateRange.change}
        tableColumns={columns.data}
        onTableColumnsChange={columns.change}
        columnsSaving={columns.saving}
        refreshIntervalMs={settings.usageDashboardRefreshIntervalMs}
        onRefreshIntervalChange={async (usageDashboardRefreshIntervalMs) =>
          autoSaveSettings({ usageDashboardRefreshIntervalMs })
        }
      />
    </div>
  );
}
