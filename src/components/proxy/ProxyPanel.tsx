import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  useGlobalProxyConfig,
  useUpdateGlobalProxyConfig,
  useProxyStatusQuery,
} from "@/lib/query/proxy";
import { copyText } from "@/lib/clipboard";
import type { GlobalProxyConfig, ClaudeGatewayConfig } from "@/types/proxy";

const defaultClaude: ClaudeGatewayConfig = {
  address: "127.0.0.1",
  port: 15723,
  sonnet: "",
  opus: "",
  haiku: "",
  fable: "",
};
type Draft = {
  address: string;
  port: string;
  claudeAddress: string;
  claudePort: string;
};
const initial: Draft = {
  address: "127.0.0.1",
  port: "15722",
  claudeAddress: "127.0.0.1",
  claudePort: "15723",
};
function endpoint(address: string, port: string, suffix: string) {
  return `http://${address.includes(":") && !address.startsWith("[") ? `[${address}]` : address}:${port}${suffix}`;
}
export function ProxyPanel() {
  const { data: config } = useGlobalProxyConfig();
  const { data: status } = useProxyStatusQuery();
  const { mutateAsync, isPending } = useUpdateGlobalProxyConfig({
    showSuccessToast: false,
  });
  const [draft, setDraft] = useState<Draft>(initial);
  const [notice, setNotice] = useState("");
  const [invalid, setInvalid] = useState(false);
  const current = useRef(config);
  current.current = config;
  const loaded = useRef(config);
  const latest = useRef(draft);
  const dirty = useRef(false);
  const version = useRef(0),
    queued = useRef(-1),
    mounted = useRef(false);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const running = status?.running ?? false;
  useEffect(() => {
    if (!config || (loaded.current === config && version.current !== 0)) return;
    loaded.current = config;
    if (dirty.current) return;
    const c = config.claude ?? defaultClaude;
    const next = {
      address: config.listenAddress,
      port: String(config.listenPort),
      claudeAddress: c.address,
      claudePort: String(c.port),
    };
    latest.current = next;
    setDraft(next);
  }, [config]);
  const save = useCallback(() => {
    if (!dirty.current || !current.current) return;
    const next = latest.current;
    const a = Number(next.port),
      b = Number(next.claudePort);
    if (
      !next.address.trim() ||
      !next.claudeAddress.trim() ||
      !/^\d+$/.test(next.port) ||
      !/^\d+$/.test(next.claudePort) ||
      a < 1 ||
      a > 65535 ||
      b < 1 ||
      b > 65535 ||
      a === b
    ) {
      if (mounted.current) {
        setInvalid(true);
        setNotice(
          "Use valid addresses and different ports between 1 and 65535.",
        );
      }
      return;
    }
    const rev = version.current;
    if (queued.current === rev) return;
    queued.current = rev;
    if (mounted.current) {
      setInvalid(false);
      setNotice("Saving…");
    }
    queue.current = queue.current
      .catch(() => undefined)
      .then(async () => {
        const base = current.current;
        if (!base) return;
        const value = (base: GlobalProxyConfig): GlobalProxyConfig => ({
          ...base,
          listenAddress: next.address.trim(),
          listenPort: a,
          claude: {
            ...(base.claude ?? defaultClaude),
            address: next.claudeAddress.trim(),
            port: b,
          },
        });
        try {
          await mutateAsync(value);
          if (version.current === rev) {
            dirty.current = false;
            if (mounted.current) setNotice("Saved");
          }
        } catch {
          if (version.current === rev) {
            queued.current = -1;
            if (mounted.current) {
              setInvalid(true);
              setNotice("Could not save. Check the addresses and retry.");
            }
          }
        }
      });
  }, [mutateAsync]);
  const flush = useRef(save);
  flush.current = save;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      flush.current();
    };
  }, []);
  useEffect(() => {
    if (!dirty.current) return;
    const timer = setTimeout(save, 400);
    return () => clearTimeout(timer);
  }, [draft, save]);
  const edit = (patch: Partial<Draft>) => {
    version.current++;
    dirty.current = true;
    const next = { ...latest.current, ...patch };
    latest.current = next;
    setDraft(next);
    setNotice("");
    setInvalid(false);
  };
  return (
    <div className="space-y-5">
      {(
        [
          {
            title: "Responses API",
            address: "address",
            port: "port",
            suffix: "/v1",
            id: "proxy",
          },
          {
            title: "Messages API",
            address: "claudeAddress",
            port: "claudePort",
            suffix: "",
            id: "claude",
          },
        ] as const
      ).map(({ title, address, port, suffix, id }) => (
        <section
          key={id}
          aria-label={title}
          className="space-y-5 rounded-xl border bg-card p-6"
        >
          <div className="flex items-center justify-between">
            <h3 className="font-semibold">{title}</h3>
            <span className="text-sm text-muted-foreground">
              {running ? "Running" : "Stopped"}
            </span>
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor={`${id}-address`}>Listen address</Label>
              <Input
                id={`${id}-address`}
                aria-label={`${title} listen address`}
                value={draft[address]}
                disabled={running || !config}
                onChange={(e) => edit({ [address]: e.target.value })}
                onBlur={save}
                aria-invalid={invalid}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor={`${id}-port`}>Port</Label>
              <Input
                id={`${id}-port`}
                aria-label={`${title} port`}
                value={draft[port]}
                disabled={running || !config}
                onChange={(e) => edit({ [port]: e.target.value })}
                onBlur={save}
                aria-invalid={invalid}
              />
            </div>
          </div>
          {running && (
            <p className="text-xs text-muted-foreground">
              Stop the server to edit.
            </p>
          )}
          <div className="flex items-center justify-between gap-3 rounded-lg bg-muted p-3">
            <code className="break-all text-sm">
              {endpoint(draft[address], draft[port], suffix)}
            </code>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Copy ${title} URL`}
              onClick={() =>
                void copyText(endpoint(draft[address], draft[port], suffix))
              }
            >
              Copy
            </Button>
          </div>
        </section>
      ))}
      {(notice || isPending) && (
        <p
          role={invalid ? "alert" : "status"}
          aria-live="polite"
          className={`text-xs ${invalid ? "text-destructive" : "text-muted-foreground"}`}
        >
          {isPending ? "Saving…" : notice}
        </p>
      )}
    </div>
  );
}
