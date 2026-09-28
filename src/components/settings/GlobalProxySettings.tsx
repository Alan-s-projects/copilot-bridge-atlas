/**
 * Global outbound proxy settings component
 *
 * Provides an input interface for configuring global agents and supports username and password authentication.
 */

import { useState, useEffect, useMemo, useRef, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Loader2, TestTube2, Search, Eye, EyeOff, X } from "lucide-react";
import {
  useGlobalProxyUrl,
  useSetGlobalProxyUrl,
  useTestProxy,
  useScanProxies,
  type DetectedProxy,
} from "@/hooks/useGlobalProxy";

/** Extract authentication information from full URL */
function extractAuth(url: string): {
  baseUrl: string;
  username: string;
  password: string;
} {
  if (!url.trim()) return { baseUrl: "", username: "", password: "" };

  try {
    const parsed = new URL(url);
    const username = decodeURIComponent(parsed.username || "");
    const password = decodeURIComponent(parsed.password || "");
    // Remove authentication information and obtain base URL
    parsed.username = "";
    parsed.password = "";
    return { baseUrl: parsed.toString(), username, password };
  } catch {
    return { baseUrl: url, username: "", password: "" };
  }
}

/** Merge authentication information into URL */
function mergeAuth(
  baseUrl: string,
  username: string,
  password: string,
): string {
  if (!baseUrl.trim()) return "";
  if (!username.trim()) return baseUrl;

  try {
    const parsed = new URL(baseUrl);
    // The username/password setter of the URL object will automatically perform percent-encoding
    // Do not use encodeURIComponent as this will result in double encoding
    parsed.username = username.trim();
    if (password) {
      parsed.password = password;
    }
    return parsed.toString();
  } catch {
    // URL parsing failed, try to insert manually (manual encoding is required at this time)
    const match = baseUrl.match(/^(\w+:\/\/)(.+)$/);
    if (match) {
      const auth = password
        ? `${encodeURIComponent(username.trim())}:${encodeURIComponent(password)}@`
        : `${encodeURIComponent(username.trim())}@`;
      return `${match[1]}${auth}${match[2]}`;
    }
    return baseUrl;
  }
}

export function GlobalProxySettings() {
  const { t } = useTranslation();
  const { data: savedUrl, isLoading } = useGlobalProxyUrl();
  const { mutateAsync: saveUrl, isPending: isSaving } = useSetGlobalProxyUrl({
    showSuccessToast: false,
  });
  const testMutation = useTestProxy();
  const scanMutation = useScanProxies();

  const [url, setUrl] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saveStatus, setSaveStatus] = useState<
    "idle" | "saving" | "saved" | "error"
  >("idle");
  const [detected, setDetected] = useState<DetectedProxy[]>([]);
  const initialized = useRef(false);
  const editVersion = useRef(0);
  const queuedVersion = useRef(0);
  const failedVersion = useRef(0);
  const saveQueue = useRef<Promise<void>>(Promise.resolve());
  const mounted = useRef(false);
  const flushSave = useRef<() => void>(() => {});

  // Calculate the full URL (including authentication information)
  const fullUrl = useMemo(
    () => mergeAuth(url, username, password),
    [url, username, password],
  );

  // Initialize once; query invalidation after an auto-save must not replace a
  // newer local edit that is still waiting in the queue.
  useEffect(() => {
    if (savedUrl === undefined || initialized.current) return;
    initialized.current = true;
    const { baseUrl, username: u, password: p } = extractAuth(savedUrl || "");
    setUrl(baseUrl);
    setUsername(u);
    setPassword(p);
  }, [savedUrl]);

  const markDirty = () => {
    editVersion.current += 1;
    setDirty(true);
    setSaveStatus("idle");
  };

  const saveCurrent = useCallback(async () => {
    if (!dirty) return;
    const version = editVersion.current;
    if (
      version < queuedVersion.current ||
      (version === queuedVersion.current && failedVersion.current !== version)
    ) {
      await saveQueue.current;
      return;
    }

    queuedVersion.current = version;
    failedVersion.current = 0;
    if (mounted.current) setSaveStatus("saving");
    const request = saveQueue.current
      .catch(() => undefined)
      .then(() => saveUrl(fullUrl));
    saveQueue.current = request.then(
      () => undefined,
      () => undefined,
    );

    try {
      await request;
      if (editVersion.current === version) {
        if (mounted.current) {
          setDirty(false);
          setSaveStatus("saved");
        }
      }
    } catch {
      failedVersion.current = version;
      if (mounted.current && editVersion.current === version) {
        setSaveStatus("error");
      }
    }
  }, [dirty, fullUrl, saveUrl]);

  flushSave.current = () => void saveCurrent();

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      flushSave.current();
    };
  }, []);

  useEffect(() => {
    if (!dirty) return;
    const timer = window.setTimeout(() => void saveCurrent(), 400);
    return () => window.clearTimeout(timer);
  }, [dirty, fullUrl, saveCurrent]);

  const handleTest = async () => {
    if (fullUrl) {
      await testMutation.mutateAsync(fullUrl);
    }
  };

  const handleScan = async () => {
    const result = await scanMutation.mutateAsync();
    setDetected(result);
  };

  const handleSelect = (proxyUrl: string) => {
    const { baseUrl, username: u, password: p } = extractAuth(proxyUrl);
    setUrl(baseUrl);
    setUsername(u);
    setPassword(p);
    markDirty();
    setDetected([]);
  };

  const handleClear = () => {
    setUrl("");
    setUsername("");
    setPassword("");
    markDirty();
  };

  // Only show loading status when loading for the first time and there is no data
  if (isLoading && savedUrl === undefined) {
    return (
      <div className="flex items-center justify-center p-4">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {/* Description */}
      <p className="text-sm text-muted-foreground">
        {t("settings.globalProxy.hint")}
      </p>

      {/* Proxy address input and actions */}
      <div className="flex gap-2">
        <Input
          placeholder="http://127.0.0.1:7890 / socks5://127.0.0.1:1080"
          value={url}
          onChange={(e) => {
            setUrl(e.target.value);
            markDirty();
          }}
          onBlur={() => void saveCurrent()}
          className="font-mono text-sm flex-1"
        />
        <Button
          variant="outline"
          size="icon"
          disabled={scanMutation.isPending}
          onClick={handleScan}
          title={t("settings.globalProxy.scan")}
        >
          {scanMutation.isPending ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Search className="h-4 w-4" />
          )}
        </Button>
        <Button
          variant="outline"
          size="icon"
          disabled={!fullUrl || testMutation.isPending}
          onClick={handleTest}
          title={t("settings.globalProxy.test")}
        >
          {testMutation.isPending ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <TestTube2 className="h-4 w-4" />
          )}
        </Button>
        <Button
          variant="outline"
          size="icon"
          disabled={!url && !username && !password}
          onClick={handleClear}
          title={t("settings.globalProxy.clear")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>
      {(isSaving || saveStatus !== "idle") && (
        <p
          role={saveStatus === "error" ? "alert" : "status"}
          aria-live="polite"
          className="text-xs text-muted-foreground"
        >
          {isSaving || saveStatus === "saving"
            ? t("settings.saving")
            : saveStatus === "saved"
              ? t("settings.saved")
              : t("settings.saveFailedGeneric")}
        </p>
      )}

      {/* Optional username and password */}
      <div className="flex gap-2">
        <Input
          placeholder={t("settings.globalProxy.username")}
          value={username}
          onChange={(e) => {
            setUsername(e.target.value);
            markDirty();
          }}
          onBlur={() => void saveCurrent()}
          className="font-mono text-sm flex-1"
        />
        <div className="relative flex-1">
          <Input
            type={showPassword ? "text" : "password"}
            placeholder={t("settings.globalProxy.password")}
            value={password}
            onChange={(e) => {
              setPassword(e.target.value);
              markDirty();
            }}
            onBlur={() => void saveCurrent()}
            className="font-mono text-sm pr-10"
          />
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
            onClick={() => setShowPassword(!showPassword)}
            tabIndex={-1}
          >
            {showPassword ? (
              <EyeOff className="h-4 w-4 text-muted-foreground" />
            ) : (
              <Eye className="h-4 w-4 text-muted-foreground" />
            )}
          </Button>
        </div>
      </div>

      {/* Scan results */}
      {detected.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {detected.map((p) => (
            <Button
              key={p.url}
              variant="secondary"
              size="sm"
              onClick={() => handleSelect(p.url)}
              className="font-mono text-xs"
            >
              {p.url}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}
