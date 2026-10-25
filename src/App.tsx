import { useClaudeMappings } from "@/hooks/useClaudeMappings";
import { MappingCountBadge } from "@/components/settings/MappingCountBadge";
import { useEffect, useState } from "react";
import {
  Settings,
  LayoutDashboard,
  BarChart2,
  Plug,
  Loader2,
} from "lucide-react";
import { useProvidersQuery } from "@/lib/query";
import { providersApi } from "@/lib/api";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import { useAvailableRelease } from "@/hooks/useAvailableRelease";
import { isTextEditableTarget } from "@/utils/domUtils";
import { CopilotCard } from "@/components/providers/CopilotCard";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { ReleaseUpdateBadge } from "@/components/settings/ReleaseUpdateBadge";
import { UsagePage } from "@/components/usage/UsagePage";
import { BridgeOverview } from "@/components/overview/BridgeOverview";
import { BridgeWarnings } from "@/components/overview/BridgeWarnings";
import { OverviewRefreshButton } from "@/components/overview/OverviewRefreshButton";
import { ConnectPage } from "@/components/providers/ConnectPage";
import { ProxyToggle } from "@/components/proxy/ProxyToggle";
import { Button } from "@/components/ui/button";

type View = "provider" | "settings" | "setup" | "usage";

const navigation = [
  { view: "provider", label: "Overview", icon: LayoutDashboard },
  { view: "usage", label: "Usage", icon: BarChart2 },
  { view: "setup", label: "Connect", icon: Plug },
  { view: "settings", label: "Settings", icon: Settings },
] as const;

export default function App() {
  const { count: mappingCount } = useClaudeMappings();
  const [settingsEntry, setSettingsEntry] = useState(0);
  const [settingsInitialTab, setSettingsInitialTab] = useState<string>();
  const [view, setView] = useState<View>("provider");
  const { status } = useProxyStatus();
  const { data: availableReleaseVersion } = useAvailableRelease();
  const { data, isLoading, refetch } = useProvidersQuery();
  const provider =
    data?.providers[data.currentProviderId] ??
    Object.values(data?.providers ?? {})[0];

  useEffect(() => {
    let off: (() => void) | undefined;
    let active = true;
    void providersApi
      .onSwitched(() => {
        void refetch();
      })
      .then((unsubscribe) => {
        if (active) off = unsubscribe;
        else unsubscribe();
      });
    return () => {
      active = false;
      off?.();
    };
  }, [refetch]);

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "," && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        setSettingsInitialTab(undefined);
        setView("settings");
      } else if (
        event.key === "Escape" &&
        !event.defaultPrevented &&
        document.body.style.overflow !== "hidden" &&
        !isTextEditableTarget(event.target)
      ) {
        setView("provider");
      }
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  }, []);

  return (
    <div className="relative flex h-full min-h-0 flex-col overflow-hidden bg-background text-foreground">
      <header className="flex h-16 shrink-0 items-center justify-between gap-3 border-b border-border px-6">
        <nav
          aria-label="Main navigation"
          className="flex min-w-0 items-center gap-2 overflow-x-auto"
        >
          {navigation.map(({ view: target, label, icon: Icon }) => (
            <Button
              key={target}
              variant={view === target ? "default" : "ghost"}
              size="sm"
              className="shrink-0"
              aria-current={view === target ? "page" : undefined}
              onClick={() => {
                setView(target);
                if (target === "settings") {
                  setSettingsInitialTab(undefined);
                  setSettingsEntry((value) => value + 1);
                }
              }}
            >
              <Icon aria-hidden className="h-4 w-4" />
              {label}
              {target === "settings" && (
                <MappingCountBadge
                  count={mappingCount}
                  selected={view === target}
                />
              )}
              {target === "settings" && availableReleaseVersion && (
                <ReleaseUpdateBadge selected={view === target} />
              )}
            </Button>
          ))}
        </nav>
        <ProxyToggle />
      </header>
      <main
        className={`flex min-h-0 min-w-0 flex-1 flex-col ${view === "setup" || view === "settings" ? "overflow-hidden" : "overflow-y-auto"}`}
      >
        {view === "settings" ? (
          <SettingsPage
            key={settingsEntry}
            initialTab={
              settingsInitialTab ?? (mappingCount ? "copilot" : "general")
            }
            mappingCount={mappingCount}
            availableReleaseVersion={availableReleaseVersion}
          />
        ) : view === "setup" ? (
          <ConnectPage
            mappingCount={mappingCount}
            onOpenModels={() => {
              setSettingsInitialTab("copilot");
              setSettingsEntry((value) => value + 1);
              setView("settings");
            }}
          />
        ) : view === "usage" ? (
          <UsagePage />
        ) : (
          <div className="space-y-5 px-6 pb-6 pt-4">
            <div className="flex items-start justify-between gap-4">
              <div className="flex min-w-0 flex-col gap-1">
                <div className="flex items-center gap-2">
                  <h1 className="text-2xl font-bold tracking-tight">
                    Overview
                  </h1>
                  <OverviewRefreshButton />
                </div>
              </div>
            </div>
            <BridgeWarnings status={status} />
            {isLoading ? (
              <Loader2 className="h-6 w-6 animate-spin" />
            ) : provider ? (
              <>
                <CopilotCard provider={provider} />
                <BridgeOverview status={status} />
              </>
            ) : (
              <Button variant="outline" onClick={() => void refetch()}>
                Reload GitHub Copilot
              </Button>
            )}
          </div>
        )}
      </main>
    </div>
  );
}
