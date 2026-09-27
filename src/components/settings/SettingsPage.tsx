import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import {
  Loader2,
  HardDriveDownload,
  Settings2,
  Network,
  Users,
  BrainCircuit,
  Info,
} from "lucide-react";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ThemeSettings } from "@/components/settings/ThemeSettings";
import { WindowSettings } from "@/components/settings/WindowSettings";
import { BackupListSection } from "@/components/settings/BackupListSection";
import { AboutSection } from "@/components/settings/AboutSection";
import { ProxyTabContent } from "@/components/settings/ProxyTabContent";
import { AuthCenterPanel } from "@/components/settings/AuthCenterPanel";
import { CopilotSettingsPanel } from "@/components/settings/CopilotSettingsPanel";
import { useSettings } from "@/hooks/useSettings";
import { useTranslation } from "react-i18next";
import type { SettingsFormState } from "@/hooks/useSettings";

export function SettingsPage() {
  const { t } = useTranslation();
  const { settings, isLoading, isSaving, autoSaveSettings } = useSettings();

  const [activeTab, setActiveTab] = useState<string>("general");
  const tabScrollContainerRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (tabScrollContainerRef.current) {
      tabScrollContainerRef.current.scrollTop = 0;
    }
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
    <div className="flex h-full min-h-0 flex-1 flex-col overflow-hidden px-4 pt-4 sm:px-6">
      <div className="mb-4 flex shrink-0 items-center justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-1">
          <h1 className="text-2xl font-bold">Settings</h1>
          <p className="text-sm text-muted-foreground">
            Manage appearance, GitHub Copilot, and proxy settings
          </p>
        </div>
        {isSaving && (
          <p
            role="status"
            aria-live="polite"
            className="shrink-0 text-xs text-muted-foreground"
          >
            {t("settings.saving")}
          </p>
        )}
      </div>
      {isBusy ? (
        <div className="flex flex-1 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <Tabs
          value={activeTab}
          onValueChange={setActiveTab}
          orientation="vertical"
          className="flex min-h-0 flex-1 gap-3 overflow-hidden sm:gap-5"
        >
          <TabsList
            aria-label="Settings sections"
            className="flex w-36 shrink-0 flex-col items-stretch justify-start gap-1 self-start rounded-lg border border-border/60 bg-muted/30 p-2 sm:w-44"
          >
            {[
              {
                value: "general",
                label: t("settings.tabGeneral"),
                icon: Settings2,
              },
              {
                value: "proxy",
                label: t("settings.tabProxy"),
                icon: Network,
              },
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
                className="min-w-0 justify-start gap-2.5 px-3 py-2.5 text-left data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=inactive]:opacity-100 dark:data-[state=active]:bg-background"
              >
                <Icon aria-hidden className="h-4 w-4 shrink-0" />
                {label}
              </TabsTrigger>
            ))}
          </TabsList>

          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            {/* Contain Radix's absolute form inputs inside the scrolling panel. */}
            <div
              ref={tabScrollContainerRef}
              role="region"
              aria-label="Settings content"
              tabIndex={0}
              className="relative min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain pb-4 pr-2"
            >
              <TabsContent value="general" className="space-y-6 mt-0">
                {settings ? (
                  <motion.div
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.3 }}
                    className="space-y-6"
                  >
                    <ThemeSettings />
                    <WindowSettings
                      settings={settings}
                      onChange={handleAutoSave}
                    />
                  </motion.div>
                ) : null}
              </TabsContent>

              <TabsContent value="proxy" className="space-y-6 mt-0 pb-4">
                {settings ? <ProxyTabContent /> : null}
              </TabsContent>

              <TabsContent value="auth" className="space-y-6 mt-0 pb-4">
                <motion.div
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.3 }}
                  className="space-y-6"
                >
                  <AuthCenterPanel />
                </motion.div>
              </TabsContent>

              <TabsContent value="copilot" className="mt-0 pb-4">
                <CopilotSettingsPanel />
              </TabsContent>

              <TabsContent value="about" className="mt-0 space-y-6 pb-4">
                <AboutSection />
                {settings ? (
                  <motion.div
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.3 }}
                    className="space-y-4"
                  >
                    <Accordion
                      type="multiple"
                      defaultValue={[]}
                      className="w-full space-y-4"
                    >
                      <AccordionItem
                        value="backup"
                        className="rounded-xl glass-card overflow-hidden"
                      >
                        <AccordionTrigger className="px-6 py-4 hover:no-underline hover:bg-muted/50 data-[state=open]:bg-muted/50">
                          <div className="flex items-center gap-3">
                            <HardDriveDownload className="h-5 w-5 text-muted-foreground" />
                            <div className="text-left">
                              <h3 className="text-base font-semibold">
                                {t("settings.about.backup.title", {
                                  defaultValue: "Backup & Restore",
                                })}
                              </h3>
                              <p className="text-sm text-muted-foreground font-normal">
                                {t("settings.about.backup.description", {
                                  defaultValue:
                                    "Manage automatic backups, view and restore database snapshots",
                                })}
                              </p>
                            </div>
                          </div>
                        </AccordionTrigger>
                        <AccordionContent className="px-6 pb-6 pt-4 border-t border-border/50">
                          <BackupListSection
                            backupIntervalHours={settings.backupIntervalHours}
                            backupRetainCount={settings.backupRetainCount}
                            onSettingsChange={(updates) =>
                              handleAutoSave(updates)
                            }
                          />
                        </AccordionContent>
                      </AccordionItem>
                    </Accordion>
                  </motion.div>
                ) : null}
              </TabsContent>
            </div>
          </div>
        </Tabs>
      )}
    </div>
  );
}
