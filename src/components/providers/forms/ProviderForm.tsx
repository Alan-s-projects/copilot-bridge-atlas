import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Loader2 } from "lucide-react";
import type { ManagedAuthProvider } from "@/lib/api";
import type { Provider, ProviderMeta, CodexCatalogModel } from "@/types";
import { CodexFormFields } from "./CodexFormFields";
import { useCopilotAuth } from "./hooks/useCopilotAuth";
import {
  isValidModelId,
  mapCodexCatalogModelForForm,
} from "@/utils/codexModelCatalog";

export const normalizeCodexCatalogModelsForSave = (
  models: CodexCatalogModel[],
): CodexCatalogModel[] => {
  const seen = new Set<string>();
  const normalized: CodexCatalogModel[] = [];

  for (const item of models) {
    const model = item.model.trim();
    const key = model.toLowerCase();
    if (!isValidModelId(model) || seen.has(key)) continue;
    seen.add(key);

    const displayName = item.displayName?.trim();
    const rawContextWindow = String(item.contextWindow ?? "").replace(
      /[^\d]/g,
      "",
    );
    const contextWindow = rawContextWindow
      ? Number.parseInt(rawContextWindow, 10)
      : undefined;
    const maxContextWindow =
      typeof item.maxContextWindow === "number" && item.maxContextWindow > 0
        ? item.maxContextWindow
        : undefined;

    const inputModalities = item.inputModalities?.filter(
      (m) => typeof m === "string" && m.trim(),
    );

    const baseInstructions = item.baseInstructions?.trim();
    const reasoningLevels = item.reasoningLevels
      ?.filter((level) => typeof level === "string" && level.trim())
      .map((level) => level.trim());
    const defaultReasoningLevel = item.defaultReasoningLevel?.trim();

    normalized.push({
      model,
      ...(item.enabled === false ? { enabled: false } : {}),
      ...(typeof item.available === "boolean"
        ? { available: item.available }
        : {}),
      ...(item.vendor ? { vendor: item.vendor } : {}),
      ...(item.maxOutputTokens
        ? { maxOutputTokens: item.maxOutputTokens }
        : {}),
      ...(typeof item.supportsToolCalls === "boolean"
        ? { supportsToolCalls: item.supportsToolCalls }
        : {}),
      ...(item.supportedReasoningLevels
        ? { supportedReasoningLevels: item.supportedReasoningLevels }
        : {}),
      ...(displayName ? { displayName } : {}),
      ...(contextWindow && contextWindow > 0 ? { contextWindow } : {}),
      ...(maxContextWindow ? { maxContextWindow } : {}),
      // Native Responses profile overrides (ignored by the chat/proxy profile).
      ...(typeof item.supportsParallelToolCalls === "boolean"
        ? { supportsParallelToolCalls: item.supportsParallelToolCalls }
        : {}),
      ...(inputModalities && inputModalities.length > 0
        ? { inputModalities }
        : {}),
      ...(baseInstructions ? { baseInstructions } : {}),
      ...(reasoningLevels ? { reasoningLevels } : {}),
      ...(defaultReasoningLevel ? { defaultReasoningLevel } : {}),
    });
  }

  return normalized;
};

export interface ProviderFormProps {
  autoSave?: boolean;
  submitLabel?: string;
  onSubmit: (values: ProviderFormValues) => Promise<void> | void;
  onCancel?: () => void;
  onManageAuthAccounts?: (target: ManagedAuthProvider) => void;
  initialData?: Partial<Provider>;
}

export interface ProviderFormValues {
  name: string;
  settingsConfig: string;
  meta?: ProviderMeta;
}

export function ProviderForm({
  initialData,
  onSubmit,
  onCancel,
  submitLabel,
  autoSave = false,
  onManageAuthAccounts,
}: ProviderFormProps) {
  const { t } = useTranslation();
  const settings = initialData?.settingsConfig ?? {
    auth: {},
    config: "",
    modelCatalog: { models: [] },
  };
  const initialMeta = initialData?.meta;
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<
    "idle" | "saved" | "error" | "invalid" | "auth-required"
  >("idle");
  const [autoSaveVersion, setAutoSaveVersion] = useState(0);
  const saveVersionRef = useRef(0);
  const savedVersionRef = useRef(0);
  const queuedVersionRef = useRef(0);
  const failedVersionRef = useRef(0);
  const pendingSavesRef = useRef(0);
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const onSubmitRef = useRef(onSubmit);
  const mountedRef = useRef(false);
  const flushAutoSaveRef = useRef<() => void>(() => {});
  onSubmitRef.current = onSubmit;
  const [accountId, setAccountId] = useState<string | null>(
    initialMeta?.authBinding?.accountId ?? initialMeta?.githubAccountId ?? null,
  );
  const [enableUltraReasoning, setEnableUltraReasoning] = useState<boolean>(
    () => {
      return (
        ((settings as Record<string, unknown>).enableUltraReasoning ??
          (settings as Record<string, unknown>).enable_ultra_reasoning) === true
      );
    },
  );
  const { hasAnyAccount } = useCopilotAuth();
  const [catalog, setCatalog] = useState<CodexCatalogModel[]>(() => {
    const value = settings.modelCatalog as { models?: unknown[] } | undefined;
    return (value?.models ?? [])
      .map(mapCodexCatalogModelForForm)
      .filter((item) => isValidModelId(item.model));
  });

  const markChanged = useCallback(() => {
    const nextVersion = saveVersionRef.current + 1;
    saveVersionRef.current = nextVersion;
    setAutoSaveVersion(nextVersion);
    setSaveStatus("idle");
  }, []);

  const buildValues = useCallback((): ProviderFormValues => {
    const meta: ProviderMeta = {
      ...initialMeta,
      providerType: "github_copilot",
      githubAccountId: accountId ?? undefined,
      authBinding: {
        source: "managed_account",
        authProvider: "github_copilot",
        accountId: accountId ?? undefined,
      },
    };
    return {
      name: initialData?.name ?? "GitHub Copilot",
      meta,
      settingsConfig: JSON.stringify({
        ...settings,
        enableUltraReasoning,
        modelCatalog: { models: normalizeCodexCatalogModelsForSave(catalog) },
      }),
    };
  }, [
    accountId,
    catalog,
    enableUltraReasoning,
    initialData?.name,
    initialMeta,
    settings,
  ]);

  const enqueueAutoSave = useCallback(
    (version: number, values: ProviderFormValues) => {
      if (
        version < queuedVersionRef.current ||
        (version === queuedVersionRef.current &&
          failedVersionRef.current !== version)
      ) {
        return saveQueueRef.current;
      }

      queuedVersionRef.current = version;
      failedVersionRef.current = 0;
      pendingSavesRef.current += 1;
      if (mountedRef.current) {
        setSaving(true);
        setSaveStatus("idle");
      }

      const request = saveQueueRef.current
        .catch(() => undefined)
        .then(() => onSubmitRef.current(values));
      saveQueueRef.current = request.then(
        () => undefined,
        () => undefined,
      );
      void request
        .then(
          () => {
            savedVersionRef.current = Math.max(
              savedVersionRef.current,
              version,
            );
            if (mountedRef.current && saveVersionRef.current === version) {
              setSaveStatus("saved");
            }
          },
          (error) => {
            failedVersionRef.current = version;
            console.error(
              "[ProviderForm] Failed to auto-save Copilot settings",
              error,
            );
            if (mountedRef.current && saveVersionRef.current === version) {
              setSaveStatus("error");
            }
          },
        )
        .finally(() => {
          pendingSavesRef.current -= 1;
          if (mountedRef.current) {
            setSaving(pendingSavesRef.current > 0);
          }
        });
      return request;
    },
    [],
  );

  flushAutoSaveRef.current = () => {
    if (
      !autoSave ||
      !hasAnyAccount ||
      saveVersionRef.current === savedVersionRef.current ||
      catalog.some((item) => item.model.trim() && !isValidModelId(item.model))
    ) {
      return;
    }
    void enqueueAutoSave(saveVersionRef.current, buildValues());
  };

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      flushAutoSaveRef.current();
    };
  }, []);

  useEffect(() => {
    if (!autoSave || autoSaveVersion === savedVersionRef.current) return;
    if (!hasAnyAccount) {
      setSaveStatus("auth-required");
      return;
    }
    if (
      catalog.some((item) => item.model.trim() && !isValidModelId(item.model))
    ) {
      setSaveStatus("invalid");
      return;
    }

    const timer = window.setTimeout(
      () => void enqueueAutoSave(autoSaveVersion, buildValues()),
      400,
    );
    return () => window.clearTimeout(timer);
  }, [
    autoSave,
    autoSaveVersion,
    buildValues,
    catalog,
    enqueueAutoSave,
    hasAnyAccount,
  ]);

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (autoSave) return;
    if (!hasAnyAccount) {
      toast.error("Sign in to GitHub Copilot first.");
      return;
    }
    if (
      catalog.some((item) => item.model.trim() && !isValidModelId(item.model))
    ) {
      toast.error("Model IDs must not contain whitespace.");
      return;
    }
    setSaving(true);
    try {
      await onSubmit(buildValues());
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <form id="provider-form" onSubmit={submit} className="space-y-6">
      <p className="text-sm text-muted-foreground">
        {t("bridge.providerSettings")}
      </p>
      <CodexFormFields
        isCopilotAuthenticated={hasAnyAccount}
        selectedGitHubAccountId={accountId}
        onGitHubAccountSelect={(id) => {
          setAccountId(id);
          markChanged();
        }}
        onManageAuthAccounts={onManageAuthAccounts}
        enableUltraReasoning={enableUltraReasoning}
        onEnableUltraReasoningChange={(enabled) => {
          setEnableUltraReasoning(enabled);
          markChanged();
        }}
        catalogModels={catalog}
        onCatalogModelsChange={(models) => {
          setCatalog(models);
          markChanged();
        }}
      />
      {autoSave ? (
        <div className="flex min-h-5 justify-end text-xs text-muted-foreground">
          {saving && (
            <span
              role="status"
              aria-live="polite"
              className="inline-flex items-center gap-1.5"
            >
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("settings.saving")}
            </span>
          )}
          {!saving && saveStatus === "saved" && (
            <span role="status" aria-live="polite">
              {t("settings.saved")}
            </span>
          )}
          {!saving && saveStatus === "error" && (
            <span role="alert">{t("settings.saveFailedGeneric")}</span>
          )}
          {!saving && saveStatus === "invalid" && (
            <span role="alert">Model IDs must not contain whitespace.</span>
          )}
          {!saving && saveStatus === "auth-required" && (
            <span role="alert">Sign in to GitHub Copilot to save changes.</span>
          )}
        </div>
      ) : (
        <div className="flex justify-end gap-2">
          {onCancel && (
            <Button
              type="button"
              variant="outline"
              disabled={saving}
              onClick={onCancel}
            >
              {t("common.cancel")}
            </Button>
          )}
          <Button type="submit" disabled={saving}>
            {saving ? t("common.saving") : (submitLabel ?? t("common.save"))}
          </Button>
        </div>
      )}
    </form>
  );
}
