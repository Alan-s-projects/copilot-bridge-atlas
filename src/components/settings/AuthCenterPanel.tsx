import { CopilotAuthSection } from "@/components/providers/forms/CopilotAuthSection";
import { SettingsSection } from "@/components/ui/settings-section";

export function AuthCenterPanel() {
  return (
    <SettingsSection title="GitHub Copilot">
      <CopilotAuthSection />
    </SettingsSection>
  );
}
