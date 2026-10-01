import type { CodexCatalogModel } from "@/types";

export const isValidModelId = (model: string): boolean =>
  model.trim().length > 0 && !/[\s\u0000-\u001f\u007f]/u.test(model.trim());

export function getCopilotModelProtocol(
  endpoints: readonly string[] = [],
): "responses" | "chat_completions" | undefined {
  const paths = new Set(
    endpoints.map((endpoint) =>
      endpoint.trim().split("?")[0].replace(/\/+$/, "").toLowerCase(),
    ),
  );
  // Match the backend router's preference when both protocols are advertised.
  if (paths.has("/responses") || paths.has("/v1/responses")) return "responses";
  if (paths.has("/chat/completions") || paths.has("/v1/chat/completions"))
    return "chat_completions";
  return undefined;
}

export const mapCodexCatalogModelForForm = (item: any): CodexCatalogModel => {
  // Preserve saved capabilities and reasoning preferences through load/save.
  // Accept both saved camelCase fields and catalog snake_case fields.
  const supportsParallelToolCalls =
    typeof item?.supportsParallelToolCalls === "boolean"
      ? item.supportsParallelToolCalls
      : typeof item?.supports_parallel_tool_calls === "boolean"
        ? item.supports_parallel_tool_calls
        : undefined;
  const inputModalities = Array.isArray(item?.inputModalities)
    ? item.inputModalities
    : Array.isArray(item?.input_modalities)
      ? item.input_modalities
      : undefined;
  const baseInstructions =
    typeof item?.baseInstructions === "string"
      ? item.baseInstructions
      : typeof item?.base_instructions === "string"
        ? item.base_instructions
        : undefined;
  const reasoningLevels = Array.isArray(item?.reasoningLevels)
    ? item.reasoningLevels
    : Array.isArray(item?.reasoning_levels)
      ? item.reasoning_levels
      : undefined;
  const defaultReasoningLevel =
    typeof item?.defaultReasoningLevel === "string"
      ? item.defaultReasoningLevel
      : typeof item?.default_reasoning_level === "string"
        ? item.default_reasoning_level
        : undefined;
  const maxContextWindow =
    typeof item?.maxContextWindow === "number"
      ? item.maxContextWindow
      : typeof item?.max_context_window === "number"
        ? item.max_context_window
        : undefined;
  return {
    model: typeof item?.model === "string" ? item.model : "",
    ...(typeof item?.enabled === "boolean" ? { enabled: item.enabled } : {}),
    ...(typeof item?.available === "boolean"
      ? { available: item.available }
      : {}),
    ...(typeof item?.vendor === "string" ? { vendor: item.vendor } : {}),
    ...(Array.isArray(item?.supportedEndpoints)
      ? {
          supportedEndpoints: item.supportedEndpoints.filter(
            (endpoint: unknown): endpoint is string =>
              typeof endpoint === "string",
          ),
        }
      : {}),
    ...(maxContextWindow !== undefined ? { maxContextWindow } : {}),
    ...(typeof item?.maxOutputTokens === "number"
      ? { maxOutputTokens: item.maxOutputTokens }
      : {}),
    ...(typeof item?.supportsToolCalls === "boolean"
      ? { supportsToolCalls: item.supportsToolCalls }
      : {}),
    ...(Array.isArray(item?.supportedReasoningLevels)
      ? { supportedReasoningLevels: item.supportedReasoningLevels }
      : {}),
    displayName:
      typeof item?.displayName === "string"
        ? item.displayName
        : typeof item?.display_name === "string"
          ? item.display_name
          : "",
    contextWindow:
      typeof item?.contextWindow === "string" ||
      typeof item?.contextWindow === "number"
        ? item.contextWindow
        : typeof item?.context_window === "string" ||
            typeof item?.context_window === "number"
          ? item.context_window
          : "",
    ...(supportsParallelToolCalls !== undefined
      ? { supportsParallelToolCalls }
      : {}),
    ...(inputModalities ? { inputModalities } : {}),
    ...(baseInstructions ? { baseInstructions } : {}),
    ...(reasoningLevels ? { reasoningLevels } : {}),
    ...(defaultReasoningLevel ? { defaultReasoningLevel } : {}),
  };
};
