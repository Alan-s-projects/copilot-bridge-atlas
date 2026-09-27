import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  useGlobalProxyConfig,
  useUpdateGlobalProxyConfig,
  useProxyStatusQuery,
} from "@/lib/query/proxy";
import { copyText } from "@/lib/clipboard";

export function ProxyPanel() {
  const { t } = useTranslation();
  const { data: config } = useGlobalProxyConfig();
  const { data: status } = useProxyStatusQuery();
  const { mutateAsync, isPending } = useUpdateGlobalProxyConfig({
    showSuccessToast: false,
  });
  const [address, setAddress] = useState("127.0.0.1");
  const [port, setPort] = useState("15722");
  const [dirty, setDirty] = useState(false);
  const [saveStatus, setSaveStatus] = useState<
    "idle" | "saving" | "saved" | "error" | "invalid"
  >("idle");
  const configRef = useRef(config);
  const loadedConfigRef = useRef<typeof config>();
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const editVersionRef = useRef(0);
  const queuedVersionRef = useRef(0);
  const failedVersionRef = useRef(0);
  const mountedRef = useRef(false);
  const flushSaveRef = useRef<() => void>(() => {});

  useEffect(() => {
    if (!config || loadedConfigRef.current === config) return;
    loadedConfigRef.current = config;
    configRef.current = config;
    if (!dirty) {
      setAddress(config.listenAddress);
      setPort(String(config.listenPort));
    }
  }, [config, dirty]);
  const running = status?.running ?? false;
  const endpoint = `http://${address.includes(":") && !address.startsWith("[") ? `[${address}]` : address}:${port}/v1`;
  const persistConfig = useCallback(
    (patch: Partial<NonNullable<typeof config>>) => {
      const request = saveQueueRef.current
        .catch(() => undefined)
        .then(async () => {
          const current = configRef.current;
          if (!current) return;
          const next = { ...current, ...patch };
          await mutateAsync(next);
          configRef.current = next;
        });
      saveQueueRef.current = request.then(
        () => undefined,
        () => undefined,
      );
      return request;
    },
    [mutateAsync],
  );
  const saveAddress = useCallback(async () => {
    if (!dirty || !configRef.current || running) return;
    const value = Number(port);
    if (
      !address.trim() ||
      !Number.isInteger(value) ||
      value < 1 ||
      value > 65535
    ) {
      if (mountedRef.current) setSaveStatus("invalid");
      return;
    }
    const version = editVersionRef.current;
    if (
      version < queuedVersionRef.current ||
      (version === queuedVersionRef.current &&
        failedVersionRef.current !== version)
    ) {
      await saveQueueRef.current;
      return;
    }

    queuedVersionRef.current = version;
    failedVersionRef.current = 0;
    if (mountedRef.current) setSaveStatus("saving");
    try {
      await persistConfig({
        listenAddress: address.trim(),
        listenPort: value,
      });
      if (editVersionRef.current === version) {
        if (mountedRef.current) {
          setDirty(false);
          setSaveStatus("saved");
        }
      }
    } catch {
      failedVersionRef.current = version;
      if (mountedRef.current && editVersionRef.current === version) {
        setSaveStatus("error");
      }
    }
  }, [address, dirty, port, persistConfig, running]);

  flushSaveRef.current = () => void saveAddress();

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      flushSaveRef.current();
    };
  }, []);

  useEffect(() => {
    if (!dirty || !config || running) return;
    const timer = window.setTimeout(() => void saveAddress(), 400);
    return () => window.clearTimeout(timer);
  }, [address, config, dirty, port, running, saveAddress]);

  const markAddressDirty = () => {
    editVersionRef.current += 1;
    setDirty(true);
    setSaveStatus("idle");
  };

  return (
    <section className="space-y-5 rounded-xl border bg-card p-6">
      <div className="flex items-center justify-between">
        <h3 className="font-semibold">Local OpenAI-compatible server</h3>
        <span className="text-sm text-muted-foreground">
          {running ? "Running" : "Stopped"}
        </span>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor="proxy-address">Listen address</Label>
          <Input
            id="proxy-address"
            value={address}
            disabled={running || !config}
            onChange={(e) => {
              setAddress(e.target.value);
              markAddressDirty();
            }}
            onBlur={() => void saveAddress()}
            aria-invalid={saveStatus === "invalid"}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="proxy-port">Port</Label>
          <Input
            id="proxy-port"
            value={port}
            disabled={running || !config}
            onChange={(e) => {
              setPort(e.target.value);
              markAddressDirty();
            }}
            onBlur={() => void saveAddress()}
            aria-invalid={saveStatus === "invalid"}
          />
        </div>
      </div>
      {saveStatus === "invalid" && (
        <p role="alert" className="text-xs text-destructive">
          Enter an address and a port between 1 and 65535.
        </p>
      )}
      {(isPending ||
        saveStatus === "saving" ||
        saveStatus === "saved" ||
        saveStatus === "error") && (
        <p
          role={saveStatus === "error" ? "alert" : "status"}
          aria-live="polite"
          className="text-xs text-muted-foreground"
        >
          {isPending || saveStatus === "saving"
            ? t("settings.saving")
            : saveStatus === "saved"
              ? t("settings.saved")
              : t("settings.saveFailedGeneric")}
        </p>
      )}
      <p className="text-sm text-muted-foreground">
        Stop the proxy before changing its address. Review the Codex connection
        suggestion after any change.
      </p>
      <div className="flex items-center justify-between gap-3 rounded-lg bg-muted p-3">
        <code className="break-all text-sm">{endpoint}</code>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void copyText(endpoint)}
        >
          Copy
        </Button>
      </div>
    </section>
  );
}
