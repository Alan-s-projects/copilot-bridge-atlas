import { useState } from "react";
import { CodexSetupSuggestion } from "./CodexSetupSuggestion";
import { useGlobalProxyConfig } from "@/lib/query/proxy";
import { copyText } from "@/lib/clipboard";
import { Button } from "@/components/ui/button";

export function ConnectPage() {
  const [client, setClient] = useState("codex");
  const { data: config } = useGlobalProxyConfig();
  const selector = (
    <div className="flex items-center gap-3">
      <label htmlFor="connection-client" className="text-sm">
        Client
      </label>
      <select
        id="connection-client"
        className="h-10 rounded-md border bg-background px-3 text-sm"
        value={client}
        onChange={(e) => setClient(e.target.value)}
      >
        <option value="codex">Codex</option>
        <option value="desktop">Claude Desktop</option>
        <option value="cli">Claude Code</option>
      </select>
    </div>
  );
  if (client === "codex")
    return <CodexSetupSuggestion clientSelector={selector} />;
  const raw = config?.claude?.address ?? "127.0.0.1";
  const host = raw === "0.0.0.0" ? "127.0.0.1" : raw === "::" ? "::1" : raw;
  const base = `http://${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${config?.claude?.port ?? 15723}`;
  const shell = `ANTHROPIC_BASE_URL='${base}' ANTHROPIC_API_KEY='atlas-local' ANTHROPIC_MODEL='claude-sonnet-5' claude`;
  return (
    <section className="space-y-5 overflow-auto px-6 pb-6 pt-4">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <h1 className="text-2xl font-bold tracking-tight">Connect</h1>
        {selector}
      </div>
      <section className="space-y-4 rounded-xl border bg-card p-6">
        <h2 className="font-semibold">
          {client === "desktop" ? "Claude Desktop gateway" : "Claude Code"}
        </h2>
        {client === "desktop" ? (
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-4 text-sm">
            <dt>Gateway base URL</dt>
            <dd className="flex items-center justify-between gap-3">
              <code>{base}</code>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void copyText(base)}
              >
                Copy
              </Button>
            </dd>
            <dt>Credential kind</dt>
            <dd>Static API key</dd>
            <dt>Gateway API key</dt>
            <dd>
              <code>atlas-local</code>
            </dd>
            <dt>Auth scheme</dt>
            <dd>
              <code>x-api-key</code>
            </dd>
            <dt>Model discovery</dt>
            <dd>On</dd>
          </dl>
        ) : (
          <>
            <pre className="overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted p-4 text-sm">
              {shell}
            </pre>
            <Button variant="outline" onClick={() => void copyText(shell)}>
              Copy command
            </Button>
          </>
        )}
        <p className="text-xs text-muted-foreground">
          Atlas uses the Copilot account selected in Settings. The local key is
          a placeholder, not a Copilot credential. Keep the gateway on loopback.
        </p>
      </section>
    </section>
  );
}
