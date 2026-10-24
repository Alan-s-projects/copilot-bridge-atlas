import { useTranslation } from "react-i18next";
import type { SettingsFormState } from "@/hooks/useSettings";
import { Power } from "lucide-react";
import { ToggleRow } from "@/components/ui/toggle-row";

export function WindowSettings({
  settings,
  onChange,
}: {
  settings: SettingsFormState;
  onChange: (updates: Partial<SettingsFormState>) => void;
}) {
  const { t } = useTranslation();
  return (
    <section className="space-y-4">
      <h3 className="text-sm font-medium">{t("settings.windowBehavior")}</h3>
      <ToggleRow
        icon={<Power className="h-4 w-4" />}
        title={t("settings.launchOnStartup")}
        checked={!!settings.launchOnStartup}
        onCheckedChange={(value) => onChange({ launchOnStartup: value })}
      />
    </section>
  );
}
