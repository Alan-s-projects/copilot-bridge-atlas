import { useCallback } from "react";
import { Loader2 } from "lucide-react";
import { useProvidersQuery, useUpdateProviderMutation } from "@/lib/query";
import { providersApi } from "@/lib/api";
import {
  ProviderForm,
  type ProviderFormValues,
} from "@/components/providers/forms/ProviderForm";
import { Button } from "@/components/ui/button";

export function CopilotSettingsPanel() {
  const { data, isLoading, error, refetch } = useProvidersQuery();
  const { mutateAsync } = useUpdateProviderMutation({
    showSuccessToast: false,
  });
  const provider =
    data?.providers[data.currentProviderId] ??
    Object.values(data?.providers ?? {})[0];
  const handleSubmit = useCallback(
    async (values: ProviderFormValues) => {
      if (!provider) return;
      await mutateAsync({
        provider: {
          ...provider,
          settingsConfig: JSON.parse(values.settingsConfig),
          meta: values.meta,
        },
      });
      try {
        await providersApi.updateTrayMenu();
      } catch (error) {
        console.warn(
          "[CopilotSettingsPanel] Failed to refresh tray menu",
          error,
        );
      }
    },
    [mutateAsync, provider],
  );

  if (isLoading) {
    return <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />;
  }
  if (!provider) {
    return (
      <div className="space-y-3">
        {error && <p role="alert">{String(error)}</p>}
        <Button variant="outline" onClick={() => void refetch()}>
          Reload GitHub Copilot
        </Button>
      </div>
    );
  }

  return (
    <ProviderForm
      key={provider.id}
      initialData={provider}
      autoSave
      onSubmit={handleSubmit}
    />
  );
}
