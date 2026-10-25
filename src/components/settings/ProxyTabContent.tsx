import { ProxyPanel } from "@/components/proxy/ProxyPanel";
import { GlobalProxySettings } from "./GlobalProxySettings";
import { SettingsSection } from "@/components/ui/settings-section";

export function ProxyTabContent() {
  return (
    <div className="space-y-6">
      <ProxyPanel />
      <SettingsSection title="Outbound network">
        <GlobalProxySettings />
      </SettingsSection>
    </div>
  );
}
