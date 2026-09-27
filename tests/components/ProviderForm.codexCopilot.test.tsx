import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import {
  ProviderForm,
  type ProviderFormValues,
} from "@/components/providers/forms/ProviderForm";
import type { ProviderMeta } from "@/types";
import { createTestQueryClient } from "../utils/testQueryClient";

vi.mock("@/components/providers/forms/CopilotAuthSection", () => ({
  CopilotAuthSection: () => null,
}));
vi.mock("@/components/providers/forms/hooks/useCopilotAuth", () => ({
  useCopilotAuth: () => ({ hasAnyAccount: true }),
}));
const config =
  'model_provider = "copilot"\n[model_providers.copilot]\nbase_url = "https://api.githubcopilot.com"\nwire_api = "responses"\n';
const models = [
  {
    model: "gpt-6-astra",
    contextWindow: 1048576,
    supportsParallelToolCalls: true,
    inputModalities: ["text", "image"],
  },
  {
    model: "gpt-6-luna",
    contextWindow: 872000,
    supportsParallelToolCalls: true,
    inputModalities: ["text", "image"],
  },
];

function renderForm(
  meta?: ProviderMeta,
  autoSave = false,
  settings: Record<string, unknown> = {},
) {
  const onSubmit = vi.fn<(values: ProviderFormValues) => void>();
  render(
    <QueryClientProvider client={createTestQueryClient()}>
      <ProviderForm
        submitLabel="save"
        autoSave={autoSave}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={{
          name: "GitHub Copilot",
          settingsConfig: {
            auth: {},
            config,
            modelCatalog: { models },
            ...settings,
          },
          meta: meta ?? { providerType: "github_copilot" },
        }}
      />
    </QueryClientProvider>,
  );
  return onSubmit;
}

function formatControl() {
  return screen.queryByRole("combobox", { name: "Upstream format" });
}

describe("Codex Copilot provider form", () => {
  it("shows saving and saved next to Model catalog, including the debounce period", async () => {
    const onSubmit = renderForm(undefined, true);
    const header = screen.getByRole("heading", {
      name: "Model catalog",
    }).parentElement!;
    expect(within(header).getByRole("status")).toHaveTextContent(
      "settings.saved",
    );
    let finish!: () => void;
    onSubmit.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    fireEvent.click(
      screen.getByRole("switch", { name: "Ultra reasoning effort" }),
    );
    expect(within(header).getByRole("status")).toHaveTextContent(
      "settings.saving",
    );
    expect(
      screen.getByRole("button", { name: "Open generated JSON" }),
    ).toBeDisabled();
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(within(header).getByRole("status")).toHaveTextContent(
      "settings.saving",
    );
    await act(async () => finish());
    await waitFor(() =>
      expect(within(header).getByRole("status")).toHaveTextContent(
        "settings.saved",
      ),
    );
    expect(screen.getAllByRole("status")).toHaveLength(1);
    expect(
      screen.getByRole("button", { name: "Open generated JSON" }),
    ).toBeEnabled();
  });

  it.each([
    [{ enableUltraReasoning: false, enable_ultra_reasoning: true }, false],
    [{ enableUltraReasoning: true, enable_ultra_reasoning: false }, true],
    [{ enable_ultra_reasoning: true }, true],
    [{}, false],
  ])(
    "loads and saves Ultra with canonical precedence: %j",
    async (settings, enabled) => {
      const onSubmit = renderForm(undefined, true, settings);
      const toggle = screen.getByRole("switch", {
        name: "Ultra reasoning effort",
      });
      expect(toggle.getAttribute("data-state")).toBe(
        enabled ? "checked" : "unchecked",
      );
      fireEvent.click(toggle);
      await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
      expect(
        JSON.parse(onSubmit.mock.calls[0][0].settingsConfig)
          .enableUltraReasoning,
      ).toBe(!enabled);
    },
  );

  it("keeps total context metadata when an unrelated model toggle is saved", async () => {
    const onSubmit = renderForm(undefined, true, {
      modelCatalog: {
        models: [
          {
            model: "grok-4.7",
            contextWindow: 372000,
            maxContextWindow: 500000,
          },
        ],
      },
    });
    expect(screen.getByLabelText("Context window")).toHaveTextContent(
      "372,000 / 500,000",
    );
    fireEvent.click(
      screen.getByRole("switch", { name: /codexConfig.modelAvailableInCodex/ }),
    );
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(
      JSON.parse(onSubmit.mock.calls[0][0].settingsConfig).modelCatalog
        .models[0],
    ).toMatchObject({
      enabled: false,
      contextWindow: 372000,
      maxContextWindow: 500000,
    });
  });

  const scrollIntoViewDescriptor = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "scrollIntoView",
  );
  beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
  });
  afterAll(() => {
    if (scrollIntoViewDescriptor) {
      Object.defineProperty(
        HTMLElement.prototype,
        "scrollIntoView",
        scrollIntoViewDescriptor,
      );
    } else {
      Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    }
  });

  it("shows the model catalog without TOML, provider metadata, or advanced request editors", () => {
    renderForm();
    expect(screen.getByRole("heading", { name: "gpt-6-astra" })).toBeVisible();
    expect(
      screen.queryByLabelText("codexConfig.defaultModelLabel"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("codexConfig.writeCommonConfig"),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "gpt-6-luna" })).toBeVisible();
    expect(screen.getAllByLabelText("Context window")[0]).toHaveTextContent(
      "1,048,576",
    );
    for (const label of ["Provider name", "Notes", "Website", "Icon"]) {
      expect(screen.queryByLabelText(label)).not.toBeInTheDocument();
    }
    expect(formatControl()).not.toBeInTheDocument();
    expect(screen.getByText("Model catalog")).toBeVisible();
    expect(
      screen.queryByText("Advanced request settings"),
    ).not.toBeInTheDocument();
    for (const label of [
      "Custom User-Agent",
      "Header overrides",
      "Body overrides",
      "Prompt cache routing",
      "Supports thinking",
      "Supports reasoning effort",
    ]) {
      expect(screen.queryByText(label)).not.toBeInTheDocument();
    }
    expect(screen.getAllByLabelText("Reasoning levels")).toHaveLength(2);
  });

  it("saves per-model enabled state without deleting catalog rows", async () => {
    const onSubmit = renderForm();
    const switches = screen.getAllByRole("switch", {
      name: /codexConfig.modelAvailableInCodex/,
    });
    expect(switches).toHaveLength(2);
    fireEvent.click(switches[1]);
    fireEvent.click(screen.getByRole("button", { name: "save" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const savedModels = JSON.parse(onSubmit.mock.calls[0][0].settingsConfig)
      .modelCatalog.models;
    expect(savedModels).toHaveLength(2);
    expect(savedModels[0]).toMatchObject({ model: "gpt-6-astra" });
    expect(savedModels[1]).toMatchObject({
      model: "gpt-6-luna",
      enabled: false,
    });
  });

  it("auto-saves Copilot edits without showing Save or Cancel buttons", async () => {
    const onSubmit = renderForm(undefined, true);
    fireEvent.click(
      screen.getByRole("switch", { name: "Ultra reasoning effort" }),
    );

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(
      screen.queryByRole("button", { name: "save" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "common.cancel" }),
    ).not.toBeInTheDocument();

    fireEvent.click(
      screen.getAllByRole("switch", {
        name: /codexConfig.modelAvailableInCodex/,
      })[1],
    );
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(2));
    const saved = onSubmit.mock.calls[1][0];
    expect(saved.meta?.codexCopilotApiFormat).toBeUndefined();
    expect(
      JSON.parse(saved.settingsConfig).modelCatalog.models[1],
    ).toMatchObject({ model: "gpt-6-luna", enabled: false });
  });

  it.each(["auto", "openai_chat", "openai_responses"])(
    "accepts legacy %s metadata without exposing a format control or changing client configuration",
    async (format) => {
      const onSubmit = renderForm({
        providerType: "github_copilot",
        codexCopilotApiFormat: format,
      });
      expect(formatControl()).not.toBeInTheDocument();
      expect(screen.getByText("Model catalog")).toBeVisible();
      fireEvent.click(screen.getByRole("button", { name: "save" }));
      await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
      const saved = onSubmit.mock.calls[0][0];
      expect(saved.meta?.codexCopilotApiFormat).toBe(format);
      expect(saved.meta?.apiFormat).toBeUndefined();
      expect(JSON.parse(saved.settingsConfig).config).toContain(
        'wire_api = "responses"',
      );
      expect(JSON.parse(saved.settingsConfig).modelCatalog.models).toEqual(
        models,
      );
      for (const key of [
        "customUserAgent",
        "localProxyRequestOverrides",
        "promptCacheRouting",
        "codexChatReasoning",
      ]) {
        expect(saved.meta).not.toHaveProperty(key);
      }
    },
  );

  it("keeps legacy cards automatic and shows mapping even with an empty catalog", () => {
    renderForm({ providerType: "github_copilot", apiFormat: "openai_chat" });
    expect(formatControl()).not.toBeInTheDocument();
    expect(screen.getByText("Model catalog")).toBeVisible();
  });

  it("does not offer preset switching while editing an existing Copilot card", () => {
    renderForm({ providerType: "github_copilot", apiFormat: "openai_chat" });
    expect(
      screen.queryByRole("button", { name: /DeepSeek/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /providerPreset.custom/ }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("providerPreset.label")).not.toBeInTheDocument();
  });

  it("leaves a retired explicit protocol opaque when saving catalog changes", async () => {
    const onSubmit = renderForm({
      providerType: "github_copilot",
      apiFormat: "openai_responses",
      codexCopilotApiFormat: "openai_responses",
    });
    expect(formatControl()).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0][0].meta?.codexCopilotApiFormat).toBe(
      "openai_responses",
    );
    expect(onSubmit.mock.calls[0][0].meta?.apiFormat).toBe("openai_responses");
  });

  it("has one model catalog with neither transport options nor other provider presets", () => {
    renderForm();
    expect(formatControl()).not.toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: /Anthropic Messages/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /DeepSeek/ }),
    ).not.toBeInTheDocument();
    expect(formatControl()).not.toBeInTheDocument();
    expect(screen.getByText("Model catalog")).toBeVisible();
  });
});
