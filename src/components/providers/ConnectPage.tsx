import { useState } from "react";
import { Code2, Monitor, Terminal } from "lucide-react";
import { CodexSetupSuggestion } from "./CodexSetupSuggestion";
import { useGlobalProxyConfig } from "@/lib/query/proxy";
import { copyText } from "@/lib/clipboard";
import { Button } from "@/components/ui/button";
import { MappingCountBadge } from "@/components/settings/MappingCountBadge";
import {
  SettingsSection,
  settingsSidebarClassName,
  settingsSidebarItemClassName,
} from "@/components/ui/settings-section";

export function ConnectPage({
  onOpenModels,
  mappingCount,
}: {
  onOpenModels?: () => void;
  mappingCount?: number;
}) {
  const [client, setClient] = useState("codex");
  const { data: config } = useGlobalProxyConfig();
  const sidebar = (
    <nav aria-label="Clients" className={settingsSidebarClassName}>
      {(
        [
          ["codex", "Codex", Code2],
          ["desktop", "Claude Desktop", Monitor],
          ["cli", "Claude Code", Terminal],
        ] as const
      ).map(([id, label, Icon]) => (
        <Button
          key={id}
          variant="ghost"
          aria-current={client === id ? "page" : undefined}
          className={settingsSidebarItemClassName}
          onClick={() => setClient(id)}
        >
          <Icon aria-hidden className="h-4 w-4 shrink-0" />
          {label}
        </Button>
      ))}
    </nav>
  );
  if (client === "codex")
    return <CodexSetupSuggestion clientNavigation={sidebar} />;
  const raw = config?.claude?.address ?? "127.0.0.1";
  const host = raw === "0.0.0.0" ? "127.0.0.1" : raw === "::" ? "::1" : raw;
  const base = `http://${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${config?.claude?.port ?? 15723}`;
  const shell = `ANTHROPIC_BASE_URL='${base}' ANTHROPIC_API_KEY='atlas-local' ANTHROPIC_MODEL='claude-sonnet-5' claude`;
  return (
    <section className="flex min-h-0 flex-1 flex-col overflow-hidden px-4 py-4 sm:px-6">
      <h1 className="sr-only">Connect</h1>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-4 sm:flex-row sm:gap-5">
        {sidebar}
        <div className="min-h-0 min-w-0 flex-1 space-y-6 overflow-y-auto overscroll-contain pb-1 pr-1">
          <SettingsSection
            title={
              client === "desktop" ? "Claude Desktop gateway" : "Claude Code"
            }
          >
            {client === "desktop" ? (
              <dl className="space-y-2 text-sm">
                {[
                  ["Gateway base URL", base, true],
                  ["Credential kind", "Static API key", false],
                  ["Gateway API key", "atlas-local", true],
                  ["Auth scheme", "x-api-key", false],
                  ["Model discovery", "On", false],
                ].map(([label, value, canCopy]) => (
                  <div
                    key={String(label)}
                    className="grid gap-1 py-1 sm:grid-cols-[160px_minmax(0,1fr)] sm:items-center sm:gap-4"
                  >
                    <dt>{label}</dt>
                    <dd className="flex min-w-0 items-center justify-between gap-3">
                      <span
                        className={
                          label === "Credential kind" ||
                          label === "Model discovery"
                            ? ""
                            : "break-all font-mono"
                        }
                      >
                        {value}
                      </span>
                      {canCopy && (
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-label={`Copy ${String(label).toLowerCase()}`}
                          onClick={() => void copyText(String(value))}
                        >
                          Copy
                        </Button>
                      )}
                    </dd>
                  </div>
                ))}
              </dl>
            ) : (
              <>
                <pre className="overflow-auto whitespace-pre-wrap break-all rounded-lg border bg-background p-4 text-sm">
                  {shell}
                </pre>
                <Button variant="outline" onClick={() => void copyText(shell)}>
                  Copy command
                </Button>
              </>
            )}
          </SettingsSection>
          {onOpenModels && (
            <SettingsSection title="Models & account">
              <p className="text-sm text-muted-foreground">
                Uses the GitHub account and explicit Claude model mappings
                selected in Models.
              </p>
              <Button variant="outline" onClick={onOpenModels}>
                Open Models <MappingCountBadge count={mappingCount} />
              </Button>
            </SettingsSection>
          )}
        </div>
      </div>
    </section>
  );
}
