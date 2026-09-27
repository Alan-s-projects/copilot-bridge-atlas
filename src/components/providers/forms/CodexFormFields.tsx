import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { FileJson, Loader2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { CopilotAuthSection } from "./CopilotAuthSection";
import {
  copilotGetModels,
  copilotGetModelsForAccount,
  copilotOpenModelCatalog,
  type CopilotModel,
} from "@/lib/api/copilot";
import { extractErrorMessage } from "@/utils/errorUtils";
import { isValidModelId } from "@/utils/codexModelCatalog";
import type { CodexCatalogModel } from "@/types";

export function isCopilotModelSupportedByCodex(model: CopilotModel): boolean {
  if (
    !isValidModelId(model.id) ||
    !model.model_picker_enabled ||
    model.policy_state === "disabled" ||
    (model.model_type && model.model_type !== "chat")
  )
    return false;
  const endpoints = [
    "/responses",
    "/v1/responses",
    "/chat/completions",
    "/v1/chat/completions",
  ];
  return (model.supported_endpoints ?? []).some((endpoint) =>
    endpoints.includes(
      endpoint.trim().split("?")[0].replace(/\/+$/, "").toLowerCase(),
    ),
  );
}

export function resolveCopilotCatalogContextWindow(
  _current: CodexCatalogModel["contextWindow"],
  reported: number | undefined,
): CodexCatalogModel["contextWindow"] {
  return reported;
}

export function mergeCopilotModelCapabilities(
  model: CopilotModel,
  existing?: CodexCatalogModel,
): CodexCatalogModel {
  return {
    ...existing,
    model: model.id,
    available: true,
    vendor: model.vendor || existing?.vendor,
    displayName: model.name || model.id,
    contextWindow: resolveCopilotCatalogContextWindow(
      existing?.contextWindow,
      model.context_window,
    ),
    maxContextWindow:
      model.max_context_window_tokens ?? existing?.maxContextWindow,
    maxOutputTokens: model.max_output_tokens ?? existing?.maxOutputTokens,
    supportsToolCalls: model.supports_tool_calls ?? existing?.supportsToolCalls,
    // Live capabilities supersede old inferred flags. An omitted declaration
    // preserves the saved value instead of guessing that a feature is absent.
    supportsParallelToolCalls:
      model.supports_parallel_tool_calls ?? existing?.supportsParallelToolCalls,
    inputModalities:
      model.supports_vision === undefined
        ? existing?.inputModalities
        : model.supports_vision
          ? ["text", "image"]
          : ["text"],
    reasoningLevels: model.reasoning_efforts ?? [],
    supportedReasoningLevels: model.reasoning_efforts ?? [],
    defaultReasoningLevel: undefined,
  };
}

interface CodexFormFieldsProps {
  catalogStatus?: ReactNode;
  catalogSavePending?: boolean;
  isCopilotAuthenticated?: boolean;
  selectedGitHubAccountId?: string | null;
  onGitHubAccountSelect?: (id: string | null) => void;
  enableUltraReasoning?: boolean;
  onEnableUltraReasoningChange?: (enabled: boolean) => void;
  catalogModels: CodexCatalogModel[];
  onCatalogModelsChange: (models: CodexCatalogModel[]) => void;
}

export function CodexFormFields({
  catalogStatus,
  catalogSavePending = false,
  isCopilotAuthenticated,
  selectedGitHubAccountId,
  onGitHubAccountSelect,
  enableUltraReasoning = false,
  onEnableUltraReasoningChange,
  catalogModels,
  onCatalogModelsChange,
}: CodexFormFieldsProps) {
  const { t } = useTranslation();
  const [fetching, setFetching] = useState(false);
  const [openingCatalog, setOpeningCatalog] = useState(false);
  const openCatalog = async () => {
    setOpeningCatalog(true);
    try {
      await copilotOpenModelCatalog();
    } catch (error) {
      toast.error(extractErrorMessage(error));
    } finally {
      setOpeningCatalog(false);
    }
  };
  const fetchSequence = useRef(0);
  useEffect(() => {
    fetchSequence.current += 1;
    setFetching(false);
    return () => {
      fetchSequence.current += 1;
    };
  }, [selectedGitHubAccountId, isCopilotAuthenticated]);

  const fetchModels = async () => {
    if (!isCopilotAuthenticated) {
      toast.error("Sign in to GitHub Copilot first.");
      return;
    }
    const sequence = ++fetchSequence.current;
    setFetching(true);
    try {
      const models = selectedGitHubAccountId
        ? await copilotGetModelsForAccount(selectedGitHubAccountId)
        : await copilotGetModels();
      if (sequence !== fetchSequence.current) return;
      const existing = new Map(
        catalogModels.map((model) => [model.model.trim().toLowerCase(), model]),
      );
      const usable = models.filter(isCopilotModelSupportedByCodex);
      if (!usable.length) {
        onCatalogModelsChange(
          catalogModels.map((model) => ({ ...model, available: false })),
        );
        toast.error(
          "No compatible chat models are available to this Copilot account.",
        );
        return;
      }
      const availableIds = new Set(
        usable.map((model) => model.id.trim().toLowerCase()),
      );
      onCatalogModelsChange([
        ...usable.map((model) =>
          mergeCopilotModelCapabilities(
            model,
            existing.get(model.id.trim().toLowerCase()),
          ),
        ),
        ...catalogModels
          .filter(
            (model) =>
              isValidModelId(model.model) &&
              !availableIds.has(model.model.trim().toLowerCase()),
          )
          .map((model) => ({ ...model, available: false })),
      ]);
      toast.success(`Loaded ${usable.length} Copilot models.`);
    } catch (error) {
      if (sequence === fetchSequence.current)
        toast.error(extractErrorMessage(error));
    } finally {
      if (sequence === fetchSequence.current) setFetching(false);
    }
  };
  const updateModel = (index: number, patch: Partial<CodexCatalogModel>) =>
    onCatalogModelsChange(
      catalogModels.map((model, position) =>
        position === index ? { ...model, ...patch } : model,
      ),
    );
  const sortedCatalogModels = catalogModels
    .map((model, index) => ({ model, index }))
    .sort((left, right) => {
      const enabledOrder =
        Number(
          right.model.enabled !== false && right.model.available !== false,
        ) -
        Number(left.model.enabled !== false && left.model.available !== false);
      if (enabledOrder !== 0) return enabledOrder;

      const leftName =
        left.model.displayName?.trim() || left.model.model.trim();
      const rightName =
        right.model.displayName?.trim() || right.model.model.trim();
      return (
        leftName.localeCompare(rightName, "en-US", {
          numeric: true,
          sensitivity: "base",
        }) || left.model.model.localeCompare(right.model.model)
      );
    });
  return (
    <div className="space-y-6">
      <CopilotAuthSection
        mode="select"
        selectedAccountId={selectedGitHubAccountId}
        onAccountSelect={onGitHubAccountSelect}
      />
      <section className="rounded-lg border p-4 space-y-2">
        <div className="flex items-center justify-between gap-4">
          <div className="space-y-0.5">
            <Label
              htmlFor="ultra-reasoning-toggle"
              className="text-sm font-medium"
            >
              Ultra reasoning effort
            </Label>
            <p className="text-xs text-muted-foreground">
              When enabled, exposes the &ldquo;Ultra&rdquo; reasoning level to
              Codex for all enabled models that support reasoning. When Codex
              uses &ldquo;ultra&rdquo; or any unrecognized reasoning effort,
              Atlas automatically falls back to the highest supported effort
              level reported by Copilot for that specific model.
            </p>
          </div>
          <Switch
            id="ultra-reasoning-toggle"
            checked={enableUltraReasoning}
            onCheckedChange={onEnableUltraReasoningChange}
          />
        </div>
      </section>
      <section className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <h3 className="text-sm font-medium">Model catalog</h3>
            {catalogStatus}
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={fetching}
              onClick={() => void fetchModels()}
            >
              {fetching ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="mr-2 h-4 w-4" />
              )}
              Refresh models
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={fetching || openingCatalog || catalogSavePending}
              onClick={() => void openCatalog()}
            >
              {openingCatalog ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <FileJson className="h-4 w-4" />
              )}
              Open generated JSON
            </Button>
          </div>
        </div>
        {catalogModels.length === 0 && (
          <p role="status" className="text-sm text-muted-foreground">
            {isCopilotAuthenticated
              ? t("codexConfig.catalogEmptySignedIn", {
                  defaultValue:
                    "No models are in this catalog yet. Refresh models to load those available to your GitHub Copilot account.",
                })
              : t("codexConfig.catalogEmptySignedOut", {
                  defaultValue:
                    "No models are available while GitHub Copilot is signed out. Sign in to GitHub Copilot, then refresh models to load the models your account can use.",
                })}
          </p>
        )}
        {sortedCatalogModels.map(({ model, index }) => (
          <div
            key={index}
            className="flex items-center justify-between gap-4 rounded-lg border px-4 py-3"
          >
            <div className="min-w-0 flex-1 space-y-1">
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <h4
                  title={model.model}
                  className="break-words text-base font-medium"
                >
                  {model.displayName?.trim() || model.model}
                </h4>
                <span
                  aria-label="Context window"
                  title="Context window: input limit / total tokens"
                  className="text-xs tabular-nums text-muted-foreground"
                >
                  Input / total:{" "}
                  {Number(model.contextWindow) > 0
                    ? Number(model.contextWindow).toLocaleString()
                    : "Not reported"}{" "}
                  / {model.maxContextWindow?.toLocaleString() ?? "Not reported"}
                </span>
              </div>
              <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <span aria-label="Reasoning levels" className="break-words">
                  Reasoning:{" "}
                  {(
                    model.supportedReasoningLevels ?? model.reasoningLevels
                  )?.join(", ") || "Not reported"}
                </span>
                {model.available === false && (
                  <p
                    role="status"
                    className="text-xs text-amber-600 dark:text-amber-400"
                  >
                    Unavailable in the current Copilot catalog
                  </p>
                )}
                <span>
                  {model.vendor ? `${model.vendor} · ` : ""}
                  Images:{" "}
                  {model.inputModalities
                    ? model.inputModalities.includes("image")
                      ? "supported"
                      : "not supported"
                    : "not reported"}{" "}
                  · Parallel tools:{" "}
                  {model.supportsParallelToolCalls === undefined
                    ? "not reported"
                    : model.supportsParallelToolCalls
                      ? "supported"
                      : "not supported"}
                </span>
              </div>
            </div>
            <Switch
              className="shrink-0"
              checked={model.enabled !== false}
              disabled={model.available === false}
              onCheckedChange={(enabled) =>
                updateModel(index, { enabled: enabled ? undefined : false })
              }
              aria-label={t("codexConfig.modelAvailableInCodex", {
                model: model.displayName?.trim() || model.model || index + 1,
              })}
            />
          </div>
        ))}
      </section>
    </div>
  );
}
