import { useTranslation } from "react-i18next";
import type { SettingsFormState } from "@/hooks/useSettings";
import { Power } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { SettingsSection } from "@/components/ui/settings-section";

export function WindowSettings({
  settings,
  onChange,
}: {
  settings: SettingsFormState;
  onChange: (updates: Partial<SettingsFormState>) => void;
}) {
  const { t } = useTranslation();
  return (
    <SettingsSection title={t("settings.windowBehavior")}>
      <label className="flex items-center justify-between gap-4">
        <span className="flex items-center gap-3 text-sm">
          <Power aria-hidden className="h-4 w-4" />
          {t("settings.launchOnStartup")}
        </span>
        <Switch
          checked={!!settings.launchOnStartup}
          onCheckedChange={(value) => onChange({ launchOnStartup: value })}
          aria-label={t("settings.launchOnStartup")}
        />
      </label>
    </SettingsSection>
  );
}
