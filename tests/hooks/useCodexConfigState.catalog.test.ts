import { describe, it, expect } from "vitest";
import { mapCodexCatalogModelForForm } from "@/utils/codexModelCatalog";

// Regression: When editing an existing native Responses provider, reading back modelCatalog must preserve hidden fields
// (supportsParallelToolCalls / inputModalities / baseInstructions), otherwise saving will
// Strip them off, causing the generated Codex catalog to lose official base_instructions, parallel tools, and image modalities.
//
// Note: initialData must be a stable reference (the init effect of the hook depends on [initialData]).
// Writing it as an inline literal will generate a new reference each time re-render → effect repeatedly setState → infinite loop OOM.
describe("Copilot model catalog field preservation", () => {
  it("preserves native-profile hidden fields (camelCase, DB SSOT)", () => {
    const initialData = {
      settingsConfig: {
        auth: { OPENAI_API_KEY: "sk-x" },
        config: "",
        modelCatalog: {
          models: [
            {
              model: "gpt-6-astra",
              displayName: "gpt-6-astra",
              contextWindow: 1000000,
              supportsParallelToolCalls: true,
              inputModalities: ["text", "image"],
              baseInstructions: "You are Codex, based on gpt-6-astra.",
            },
          ],
        },
      },
    };

    const models = initialData.settingsConfig.modelCatalog.models.map(
      mapCodexCatalogModelForForm,
    );

    expect(models).toEqual([
      {
        model: "gpt-6-astra",
        displayName: "gpt-6-astra",
        contextWindow: 1000000,
        supportsParallelToolCalls: true,
        inputModalities: ["text", "image"],
        baseInstructions: "You are Codex, based on gpt-6-astra.",
      },
    ]);
  });

  it("maps snake_case hidden fields (live reverse-parse fallback) to camelCase", () => {
    const initialData = {
      settingsConfig: {
        auth: {},
        config: "",
        modelCatalog: {
          models: [
            {
              model: "gpt-6-luna",
              display_name: "GPT-6 Luna",
              context_window: 262144,
              supports_parallel_tool_calls: false,
              input_modalities: ["text"],
              base_instructions: "You are Codex, based on GPT-6 Luna.",
            },
          ],
        },
      },
    };

    const models = initialData.settingsConfig.modelCatalog.models.map(
      mapCodexCatalogModelForForm,
    );

    expect(models).toEqual([
      {
        model: "gpt-6-luna",
        displayName: "GPT-6 Luna",
        contextWindow: 262144,
        supportsParallelToolCalls: false,
        inputModalities: ["text"],
        baseInstructions: "You are Codex, based on GPT-6 Luna.",
      },
    ]);
  });
});
