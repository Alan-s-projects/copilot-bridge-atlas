import { useCallback, useEffect, useState } from "react";
import { useSettingsQuery } from "@/lib/query";
import type { Settings } from "@/types";

export type SettingsFormState = Settings;

export function useSettingsForm(isSaving = false, completedSaves = 0) {
  const { data, isLoading } = useSettingsQuery();
  const [settings, setSettings] = useState<SettingsFormState | null>(null);

  useEffect(() => {
    // An earlier save's refetch must not replace newer optimistic choices.
    // Once the queue drains, use persisted values to discard failed edits.
    if (data && !isSaving) setSettings(data);
  }, [data, isSaving, completedSaves]);

  const updateSettings = useCallback((updates: Partial<SettingsFormState>) => {
    setSettings((previous) => (previous ? { ...previous, ...updates } : null));
  }, []);

  return { settings, isLoading, updateSettings };
}
