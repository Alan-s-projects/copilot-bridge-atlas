import { MappingCountBadge } from "./MappingCountBadge";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import {
  Loader2,
  Settings2,
  Network,
  Users,
  BrainCircuit,
  Info,
} from "lucide-react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  SettingsSection,
  settingsSidebarClassName,
  settingsSidebarItemClassName,
} from "@/components/ui/settings-section";
import { ThemeSettings } from "@/components/settings/ThemeSettings";
import { WindowSettings } from "@/components/settings/WindowSettings";
import { BackupListSection } from "@/components/settings/BackupListSection";
import { AboutSection } from "@/components/settings/AboutSection";
import { ReleaseUpdateBadge } from "@/components/settings/ReleaseUpdateBadge";
import { ProxyTabContent } from "@/components/settings/ProxyTabContent";
import { AuthCenterPanel } from "@/components/settings/AuthCenterPanel";
import { CopilotSettingsPanel } from "@/components/settings/CopilotSettingsPanel";
import { useSettings } from "@/hooks/useSettings";
import { useTranslation } from "react-i18next";
import type { SettingsFormState } from "@/hooks/useSettings";

export function SettingsPage({
  availableReleaseVersion,
  mappingCount,
  initialTab = "general",
}: {
  availableReleaseVersion?: string | null;
  mappingCount?: number;
  initialTab?: string;
}) {
  const { t } = useTranslation();
  const { settings, isLoading, isSaving, autoSaveSettings } = useSettings();
  const [activeTab, setActiveTab] = useState<string>(initialTab);
  const tabScrollContainerRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (tabScrollContainerRef.current)
      tabScrollContainerRef.current.scrollTop = 0;
  }, [activeTab]);

  const handleAutoSave = useCallback(
    async (updates: Partial<SettingsFormState>): Promise<boolean> => {
      try {
        return await autoSaveSettings(updates);
      } catch (error) {
        console.error("[SettingsPage] Failed to autosave settings", error);
        return false;
      }
    },
    [autoSaveSettings],
  );

  const isBusy = isLoading && !settings;
  return (
    <div className="relative flex h-full min-h-0 flex-1 flex-col overflow-hidden px-4 py-4 sm:px-6">
      <h1 className="sr-only">Settings</h1>
      {isSaving && (
        <p
          role="status"
          aria-live="polite"
          className="absolute right-6 top-0 text-xs text-muted-foreground"
        >
          {t("settings.saving")}
        </p>
      )}
      {isBusy ? (
        <div className="flex flex-1 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <Tabs
          value={activeTab}
          onValueChange={setActiveTab}
          orientation="vertical"
          className="flex min-h-0 flex-1 flex-col gap-4 overflow-hidden sm:flex-row sm:gap-5"
        >
          <TabsList
            aria-label="Settings sections"
            className={settingsSidebarClassName}
          >
            {[
              {
                value: "general",
                label: t("settings.tabGeneral"),
                icon: Settings2,
              },
              { value: "proxy", label: t("settings.tabProxy"), icon: Network },
              {
                value: "auth",
                label: t("settings.tabAuth", { defaultValue: "Accounts" }),
                icon: Users,
              },
              { value: "copilot", label: "Models", icon: BrainCircuit },
              { value: "about", label: "About", icon: Info },
            ].map(({ value, label, icon: Icon }) => (
              <TabsTrigger
                key={value}
                value={value}
                className={settingsSidebarItemClassName}
              >
                <Icon aria-hidden className="h-4 w-4 shrink-0" />
                {label}
                {value === "copilot" && (
                  <MappingCountBadge
                    count={mappingCount}
                    selected={activeTab === value}
                    className="ml-auto"
                  />
                )}
                {value === "about" && availableReleaseVersion && (
                  <ReleaseUpdateBadge
                    selected={activeTab === value}
                    className="ml-auto"
                  />
                )}
              </TabsTrigger>
            ))}
          </TabsList>
          {/* Contain Radix's absolute form inputs inside the scrolling panel. */}
          <div
            ref={tabScrollContainerRef}
            role="region"
            aria-label="Settings content"
            tabIndex={0}
            className="relative min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain pb-1 pr-1"
          >
            <TabsContent value="general" className="mt-0 space-y-6">
              {settings && (
                <>
                  <ThemeSettings />
                  <WindowSettings
                    settings={settings}
                    onChange={handleAutoSave}
                  />
                </>
              )}
            </TabsContent>
            <TabsContent value="proxy" className="mt-0 space-y-6">
              {settings && <ProxyTabContent />}
            </TabsContent>
            <TabsContent value="auth" className="mt-0 space-y-6">
              <AuthCenterPanel />
            </TabsContent>
            <TabsContent value="copilot" className="mt-0">
              <CopilotSettingsPanel />
            </TabsContent>
            <TabsContent value="about" className="mt-0 space-y-6">
              <AboutSection availableReleaseVersion={availableReleaseVersion} />
              {settings && (
                <SettingsSection
                  title={t("settings.about.backup.title", {
                    defaultValue: "Backup & Restore",
                  })}
                >
                  <BackupListSection
                    backupIntervalHours={settings.backupIntervalHours}
                    backupRetainCount={settings.backupRetainCount}
                    onSettingsChange={handleAutoSave}
                  />
                </SettingsSection>
              )}
            </TabsContent>
          </div>
        </Tabs>
      )}
    </div>
  );
}
