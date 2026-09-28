import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api";
import { useSaveSettingsMutation } from "@/lib/query";
import type { Settings } from "@/types";
import { useSettingsForm, type SettingsFormState } from "./useSettingsForm";

interface UseSettingsResult {
  settings: SettingsFormState | null;
  isLoading: boolean;
  isSaving: boolean;
  autoSaveSettings: (updates: Partial<SettingsFormState>) => Promise<boolean>;
}

export type { SettingsFormState };

// Keep writes ordered across separate Settings and Usage consumers.
let settingsSaveQueue: Promise<void> = Promise.resolve();

export function useSettings(): UseSettingsResult {
  const { t } = useTranslation();
  const saveMutation = useSaveSettingsMutation();
  const [saveState, setSaveState] = useState({ pending: 0, completed: 0 });
  const { settings, isLoading, updateSettings } = useSettingsForm(
    saveState.pending > 0,
    saveState.completed,
  );

  const runSave = useCallback(async <T>(save: () => Promise<T>): Promise<T> => {
    setSaveState((state) => ({ ...state, pending: state.pending + 1 }));
    const result = settingsSaveQueue.then(save);
    settingsSaveQueue = result.then(
      () => undefined,
      () => undefined,
    );
    try {
      return await result;
    } finally {
      setSaveState((state) => ({
        pending: state.pending - 1,
        completed: state.completed + 1,
      }));
    }
  }, []);

  const savePreferences = useCallback(
    async (updates: Partial<SettingsFormState>) => {
      // Read inside the queue so concurrent edits cannot overwrite one another.
      const previous = await settingsApi.get();
      const payload: Settings = { ...previous, ...updates };
      const startupChanged =
        payload.launchOnStartup !== undefined &&
        payload.launchOnStartup !== previous.launchOnStartup;

      if (startupChanged) {
        await settingsApi.setAutoLaunch(payload.launchOnStartup!);
      }
      try {
        await saveMutation.mutateAsync(payload);
      } catch (error) {
        if (startupChanged) {
          try {
            await settingsApi.setAutoLaunch(previous.launchOnStartup ?? false);
          } catch (rollbackError) {
            console.error("Failed to restore auto-launch:", rollbackError);
            toast.error(
              t("settings.autoLaunchFailed", {
                defaultValue: "Failed to set auto-launch",
              }),
            );
          }
        }
        throw error;
      }
    },
    [saveMutation, t],
  );

  const autoSaveSettings = useCallback(
    async (updates: Partial<SettingsFormState>): Promise<boolean> => {
      if (!settings) return false;
      const changes = { ...updates };
      updateSettings(changes);

      try {
        await runSave(() => savePreferences(changes));
        return true;
      } catch (error) {
        console.error("[useSettings] Failed to auto-save settings", error);
        toast.error(
          t("notifications.settingsSaveFailed", {
            defaultValue: "Failed to save settings: {{error}}",
            error: (error as Error)?.message ?? String(error),
          }),
        );
        throw error;
      }
    },
    [runSave, savePreferences, settings, t, updateSettings],
  );

  return {
    settings,
    isLoading,
    isSaving: saveState.pending > 0,
    autoSaveSettings,
  };
}
