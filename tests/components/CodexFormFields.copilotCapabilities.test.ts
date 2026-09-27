import { describe, expect, it } from "vitest";
import {
  isCopilotModelSupportedByCodex,
  resolveCopilotCatalogContextWindow,
  mergeCopilotModelCapabilities,
} from "@/components/providers/forms/CodexFormFields";
import type { CopilotModel } from "@/lib/api/copilot";
import endpointCases from "../fixtures/copilot-endpoint-cases.json";

function model(supportedEndpoints?: string[]): CopilotModel {
  return {
    id: "gpt-test",
    name: "Model",
    vendor: "vendor",
    model_picker_enabled: true,
    supported_endpoints: supportedEndpoints,
  };
}

describe("Codex Copilot capabilities", () => {
  it.each(endpointCases)(
    "matches the backend endpoint contract: $name",
    ({ endpoints, formats }) => {
      expect(isCopilotModelSupportedByCodex(model(endpoints))).toBe(
        formats.includes("auto"),
      );
    },
  );

  it("uses the Copilot context window instead of saved overrides", () => {
    expect(resolveCopilotCatalogContextWindow("", 400_000)).toBe(400_000);
    expect(resolveCopilotCatalogContextWindow(undefined, 1_000_000)).toBe(
      1_000_000,
    );
    expect(resolveCopilotCatalogContextWindow(200_000, 400_000)).toBe(400_000);
    expect(resolveCopilotCatalogContextWindow(1_000_000, 872_000)).toBe(
      872_000,
    );
  });

  it("refreshes live capabilities and replaces saved reasoning overrides", () => {
    const saved = {
      model: "gpt-6-luna",
      contextWindow: 1_000_000,
      supportsParallelToolCalls: false,
      inputModalities: ["text"],
      reasoningLevels: ["low", "high", "ultra"],
      defaultReasoningLevel: "ultra",
    };
    const live: CopilotModel = {
      ...model(["/responses"]),
      id: "gpt-6-luna",
      context_window: 872_000,
      supports_parallel_tool_calls: true,
      supports_vision: true,
      reasoning_efforts: ["low", "medium", "high", "xhigh", "max"],
    };
    const refreshed = mergeCopilotModelCapabilities(live, saved);
    expect(refreshed).toMatchObject({
      contextWindow: 872_000,
      supportsParallelToolCalls: true,
      inputModalities: ["text", "image"],
      reasoningLevels: live.reasoning_efforts,
      defaultReasoningLevel: undefined,
    });
    expect(mergeCopilotModelCapabilities(live, refreshed)).toEqual(refreshed);
    expect(
      mergeCopilotModelCapabilities(
        {
          ...live,
          supports_parallel_tool_calls: false,
          supports_vision: false,
        },
        {
          ...saved,
          supportsParallelToolCalls: true,
          inputModalities: ["text", "image"],
        },
      ),
    ).toMatchObject({
      supportsParallelToolCalls: false,
      inputModalities: ["text"],
    });
    expect(mergeCopilotModelCapabilities(model(), saved)).toMatchObject({
      supportsParallelToolCalls: false,
      inputModalities: ["text"],
      reasoningLevels: [],
      defaultReasoningLevel: undefined,
    });
  });

  it.each([{ reasoningLevels: undefined }, { reasoningLevels: [] }])(
    "initializes an unset reasoning list from Copilot without forcing a default",
    ({ reasoningLevels }) => {
      const live = {
        ...model(["/responses"]),
        reasoning_efforts: ["low", "medium", "high"],
      };
      const refreshed = mergeCopilotModelCapabilities(live, {
        model: live.id,
        reasoningLevels,
      });
      expect(refreshed.reasoningLevels).toEqual(live.reasoning_efforts);
      expect(refreshed.defaultReasoningLevel).toBeUndefined();
      expect(mergeCopilotModelCapabilities(live).reasoningLevels).toEqual(
        live.reasoning_efforts,
      );
    },
  );

  it("does not invent reasoning efforts for a new model without a declaration", () => {
    const refreshed = mergeCopilotModelCapabilities(model(["/responses"]));
    expect(refreshed.reasoningLevels).toEqual([]);
    expect(refreshed.supportedReasoningLevels).toEqual([]);
    expect(refreshed.defaultReasoningLevel).toBeUndefined();
  });

  it("keeps an automatic default when the saved levels differ from Copilot", () => {
    const refreshed = mergeCopilotModelCapabilities(
      { ...model(["/responses"]), reasoning_efforts: ["low", "medium"] },
      { model: "gpt-test", reasoningLevels: ["high", "ultra"] },
    );
    expect(refreshed.reasoningLevels).toEqual(["low", "medium"]);
    expect(refreshed.defaultReasoningLevel).toBeUndefined();
  });

  it("rejects an absent capabilities field", () => {
    expect(isCopilotModelSupportedByCodex(model())).toBe(false);
  });

  it("accepts current and future model names without a vendor allowlist", () => {
    const compatible = model(["/responses"]);
    expect(
      isCopilotModelSupportedByCodex({ ...compatible, id: " GPT-6-ASTRA " }),
    ).toBe(true);
    for (const id of [
      "grok-4.7",
      "gemini-3.8-flash",
      "mai-code-1.1-flash",
      "future-vendor/model-v2",
    ]) {
      expect(isCopilotModelSupportedByCodex({ ...compatible, id })).toBe(true);
    }
    for (const id of ["", "   ", "model with spaces"]) {
      expect(isCopilotModelSupportedByCodex({ ...compatible, id })).toBe(false);
    }
  });

  it("respects model type, policy and picker eligibility", () => {
    const compatible = model(["/responses"]);
    for (const patch of [
      { model_type: "embeddings" },
      { model_type: "completion" },
      { model_picker_enabled: false },
      { policy_state: "disabled" },
    ]) {
      expect(isCopilotModelSupportedByCodex({ ...compatible, ...patch })).toBe(
        false,
      );
    }
  });

  it("preserves enabled choices but reports only live reasoning for future models", () => {
    const refreshed = mergeCopilotModelCapabilities(
      {
        ...model(["/chat/completions"]),
        id: "future-vendor/agent",
        vendor: "New Vendor",
        max_output_tokens: 16384,
        supports_tool_calls: false,
        reasoning_efforts: ["low"],
      },
      {
        model: "future-vendor/agent",
        enabled: false,
        reasoningLevels: ["low", "ultra"],
      },
    );
    expect(refreshed).toMatchObject({
      available: true,
      enabled: false,
      vendor: "New Vendor",
      maxOutputTokens: 16384,
      supportsToolCalls: false,
      reasoningLevels: ["low"],
      supportedReasoningLevels: ["low"],
    });
  });
});
