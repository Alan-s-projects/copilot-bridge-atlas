import { describe, expect, it } from "vitest";
import { normalizeCodexCatalogModelsForSave } from "@/components/providers/forms/ProviderForm";
import { mapCodexCatalogModelForForm } from "@/utils/codexModelCatalog";

describe("ProviderForm Codex catalog helpers", () => {
  it.each(["maxContextWindow", "max_context_window"])(
    "preserves the total context limit through load and save from %s",
    (key) => {
      const loaded = mapCodexCatalogModelForForm({
        model: "grok-4.7",
        contextWindow: 372000,
        [key]: 500000,
      });
      expect(normalizeCodexCatalogModelsForSave([loaded])).toEqual([
        {
          model: "grok-4.7",
          contextWindow: 372000,
          maxContextWindow: 500000,
        },
      ]);
    },
  );

  it("normalizes catalog rows and removes empty or duplicate models", () => {
    expect(
      normalizeCodexCatalogModelsForSave([
        { model: " gpt-6-astra ", displayName: " GPT-6 Astra " },
        { model: "GPT-6-ASTRA", displayName: "Duplicate" },
        { model: "", displayName: "Empty" },
        { model: "other-model", displayName: "Future Model" },
        { model: "invalid model", displayName: "Invalid" },
        { model: "gpt-6-luna", contextWindow: "128000 tokens" },
      ]),
    ).toEqual([
      { model: "gpt-6-astra", displayName: "GPT-6 Astra" },
      { model: "other-model", displayName: "Future Model" },
      { model: "gpt-6-luna", contextWindow: 128000 },
    ]);
  });

  it("preserves native-profile overrides (parallel tool calls + input modalities + base instructions)", () => {
    expect(
      normalizeCodexCatalogModelsForSave([
        {
          model: "GPT-6-ASTRA",
          displayName: "GPT-6-ASTRA",
          contextWindow: 1000000,
          supportsParallelToolCalls: true,
          inputModalities: ["text", "image"],
          baseInstructions:
            "  You are Codex, a coding agent based on GPT-6-ASTRA.  ",
        },
        // false must be preserved (not dropped as falsy); empty modalities dropped;
        // empty/whitespace baseInstructions dropped
        {
          model: "gpt-6-luna",
          supportsParallelToolCalls: false,
          inputModalities: [],
          baseInstructions: "   ",
        },
      ]),
    ).toEqual([
      {
        model: "GPT-6-ASTRA",
        displayName: "GPT-6-ASTRA",
        contextWindow: 1000000,
        supportsParallelToolCalls: true,
        inputModalities: ["text", "image"],
        baseInstructions: "You are Codex, a coding agent based on GPT-6-ASTRA.",
      },
      { model: "gpt-6-luna", supportsParallelToolCalls: false },
    ]);
  });

  it("preserves per-model reasoning levels and default level", () => {
    expect(
      normalizeCodexCatalogModelsForSave([
        {
          model: "gpt-6-astra",
          displayName: "GPT-6 Astra",
          reasoningLevels: ["none", "low", "medium", "high", "xhigh", "max"],
          defaultReasoningLevel: " xhigh ",
        },
        // empty levels / whitespace default are dropped
        {
          model: "gpt-test",
          reasoningLevels: [],
          defaultReasoningLevel: "   ",
        },
      ]),
    ).toEqual([
      {
        model: "gpt-6-astra",
        displayName: "GPT-6 Astra",
        reasoningLevels: ["none", "low", "medium", "high", "xhigh", "max"],
        defaultReasoningLevel: "xhigh",
      },
      { model: "gpt-test", reasoningLevels: [] },
    ]);
  });

  it("round-trips a disabled model while omitting the default enabled state", () => {
    const disabled = { model: "gpt-6-luna", enabled: false };
    expect(
      normalizeCodexCatalogModelsForSave([
        mapCodexCatalogModelForForm(disabled),
      ]),
    ).toEqual([disabled]);
    expect(
      normalizeCodexCatalogModelsForSave([
        mapCodexCatalogModelForForm({ model: "gpt-6-astra", enabled: true }),
      ]),
    ).toEqual([{ model: "gpt-6-astra" }]);
  });

  it("round-trips reasoning levels through load and save without loss", () => {
    // Load and save must preserve each GPT model's explicit reasoning list.
    const stored = [
      {
        model: "gpt-6-luna",
        displayName: "GPT-6 Luna",
        reasoningLevels: ["high", "max"],
      },
      // Handwritten/old data may be snake_case, and the loading side must be compatible and the saving side must also be retained.
      { model: "gpt-6-astra", reasoning_levels: ["low", "high", "max"] },
      { model: "gpt-test" }, // Toggle type: no watch, no watch can be made out of thin air during the whole process
    ];

    const roundTripped = normalizeCodexCatalogModelsForSave(
      stored.map(mapCodexCatalogModelForForm),
    );

    expect(roundTripped).toEqual([
      {
        model: "gpt-6-luna",
        displayName: "GPT-6 Luna",
        reasoningLevels: ["high", "max"],
      },
      { model: "gpt-6-astra", reasoningLevels: ["low", "high", "max"] },
      { model: "gpt-test" },
    ]);
  });

  it("preserves a disabled Copilot model through load and save", () => {
    const stored = {
      model: "gpt-6-luna",
      displayName: "GPT-6 Luna",
      enabled: false,
    };

    expect(
      normalizeCodexCatalogModelsForSave([mapCodexCatalogModelForForm(stored)]),
    ).toEqual([stored]);
    expect(
      normalizeCodexCatalogModelsForSave([
        mapCodexCatalogModelForForm({ model: "gpt-6-astra", enabled: true }),
      ]),
    ).toEqual([{ model: "gpt-6-astra" }]);
  });

  it("trims reasoning level values on save", () => {
    // The "high" in the hand-coded JSON must not be dropped into the library/sent to the upstream as it is.
    expect(
      normalizeCodexCatalogModelsForSave([
        { model: "gpt-6-luna", reasoningLevels: [" high ", "max"] },
      ]),
    ).toEqual([{ model: "gpt-6-luna", reasoningLevels: ["high", "max"] }]);
  });
});
