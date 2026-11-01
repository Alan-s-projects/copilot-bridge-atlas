import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  CodexFormFields,
  isCopilotModelManageable,
  isCopilotModelSupportedByCodex,
  mergeCopilotModelCapabilities,
} from "@/components/providers/forms/CodexFormFields";
import {
  ProviderForm,
  normalizeCodexCatalogModelsForSave,
} from "@/components/providers/forms/ProviderForm";
import {
  isDecisionCatalogModel,
  mapCodexCatalogModelForForm,
} from "@/utils/codexModelCatalog";
import type { CopilotModel } from "@/lib/api/copilot";
import type { CodexCatalogModel } from "@/types";

const mocks = vi.hoisted(() => ({
  getModels: vi.fn(),
  getAccountModels: vi.fn(),
}));
vi.mock("@/components/providers/forms/CopilotAuthSection", () => ({
  CopilotAuthSection: () => null,
}));
vi.mock("@/components/providers/forms/hooks/useCopilotAuth", () => ({
  useCopilotAuth: () => ({ hasAnyAccount: true }),
}));
vi.mock("@/lib/api/copilot", () => ({
  copilotGetModels: mocks.getModels,
  copilotGetModelsForAccount: mocks.getAccountModels,
  copilotOpenModelCatalog: vi.fn(),
}));

const decision: CopilotModel = {
  id: "microsoft-decision-1",
  name: "Microsoft Decision-1",
  vendor: "Experimental",
  model_type: "decisions",
  model_picker_enabled: false,
  supported_endpoints: ["/v1/systemone"],
};
const chat: CodexCatalogModel = {
  model: "gpt-6-luna",
  displayName: "GPT-6 Luna",
  contextWindow: 872000,
  maxContextWindow: 1000000,
  supportedEndpoints: ["/responses"],
  supportedReasoningLevels: ["none", "low", "high"],
  reasoningLevels: ["none", "low", "high"],
};
const decisionRow = mergeCopilotModelCapabilities(decision);

beforeEach(() => {
  mocks.getModels.mockReset().mockResolvedValue([
    {
      id: chat.model,
      name: chat.displayName,
      vendor: "OpenAI",
      model_picker_enabled: true,
      model_type: "chat",
      context_window: 872000,
      max_context_window_tokens: 1000000,
      supported_endpoints: ["/responses"],
      reasoning_efforts: ["none", "low", "high"],
    },
    decision,
  ]);
  mocks.getAccountModels.mockReset().mockImplementation(mocks.getModels);
});

describe("Decision models in Enabled models", () => {
  it("includes a hidden decision model for management without making it a chat model", () => {
    expect(isCopilotModelManageable(decision)).toBe(true);
    expect(isCopilotModelSupportedByCodex(decision)).toBe(false);
    expect(
      isCopilotModelManageable({ ...decision, policy_state: "disabled" }),
    ).toBe(false);
    expect(
      isCopilotModelManageable({ ...decision, model_type: "embeddings" }),
    ).toBe(false);
    expect(
      isCopilotModelManageable({ ...decision, supported_endpoints: [] }),
    ).toBe(false);
  });

  it("leaves the existing chat row markup unchanged when a decision row is added", () => {
    const props = {
      catalogModels: [chat],
      onCatalogModelsChange: vi.fn(),
      isCopilotAuthenticated: true,
    };
    const { rerender } = render(<CodexFormFields {...props} />);
    const chatNode = screen
      .getByRole("heading", { name: "GPT-6 Luna" })
      .closest(".rounded-lg")!;
    const before = chatNode.innerHTML;
    rerender(
      <CodexFormFields {...props} catalogModels={[chat, decisionRow]} />,
    );
    expect(
      screen
        .getByRole("heading", { name: "GPT-6 Luna" })
        .closest(".rounded-lg"),
    ).toBe(chatNode);
    expect(chatNode.innerHTML).toBe(before);
    const row = screen
      .getByRole("heading", { name: "Microsoft Decision-1" })
      .closest(".rounded-lg")!;
    expect(within(row as HTMLElement).getByText("Decisions")).toBeVisible();
    expect(within(row as HTMLElement).getByText("/v1/systemone")).toBeVisible();
    expect(
      within(row as HTMLElement).queryByLabelText("Context window"),
    ).not.toBeInTheDocument();
    expect(
      within(row as HTMLElement).queryByLabelText("Reasoning levels"),
    ).not.toBeInTheDocument();
    expect(
      within(row as HTMLElement).getByRole("switch", {
        name: "Enable Microsoft Decision-1 for SystemOne",
      }),
    ).toBeChecked();
  });

  it("preserves a disabled decision toggle through refresh, save and form reload", async () => {
    const save = vi.fn();
    const initialData = {
      name: "GitHub Copilot",
      settingsConfig: { modelCatalog: { models: [chat, decisionRow] } },
      meta: { providerType: "github_copilot" },
    };
    const view = render(
      <ProviderForm autoSave initialData={initialData} onSubmit={save} />,
    );
    fireEvent.click(
      screen.getByRole("switch", {
        name: "Enable Microsoft Decision-1 for SystemOne",
      }),
    );
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    let settings = JSON.parse(save.mock.calls[0][0].settingsConfig);
    expect(
      settings.modelCatalog.models.find(
        (m: CodexCatalogModel) => m.model === decision.id,
      ),
    ).toMatchObject({
      modelType: "decisions",
      enabled: false,
      supportedEndpoints: ["/v1/systemone"],
    });
    fireEvent.click(screen.getByRole("button", { name: "Refresh models" }));
    await waitFor(() => expect(mocks.getModels).toHaveBeenCalledOnce());
    await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
    expect(
      screen.getByRole("switch", {
        name: "Enable Microsoft Decision-1 for SystemOne",
      }),
    ).not.toBeChecked();
    settings = JSON.parse(save.mock.calls[1][0].settingsConfig);
    expect(
      settings.modelCatalog.models.find(
        (m: CodexCatalogModel) => m.model === chat.model,
      ),
    ).toMatchObject(chat);
    view.unmount();
    render(
      <ProviderForm
        autoSave
        initialData={{ ...initialData, settingsConfig: settings }}
        onSubmit={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("switch", {
        name: "Enable Microsoft Decision-1 for SystemOne",
      }),
    ).not.toBeChecked();
  });

  it("round-trips decision classification and recognizes endpoint-only legacy rows", () => {
    const saved = normalizeCodexCatalogModelsForSave([
      { ...decisionRow, enabled: false },
    ]);
    const restored = mapCodexCatalogModelForForm(saved[0]);
    expect(restored.modelType).toBe("decisions");
    expect(restored.enabled).toBe(false);
    expect(isDecisionCatalogModel(restored)).toBe(true);
    expect(
      isDecisionCatalogModel({ supportedEndpoints: ["/v1/systemone"] }),
    ).toBe(true);
    expect(isDecisionCatalogModel(chat)).toBe(false);
    expect(isDecisionCatalogModel({})).toBe(false);
  });
});
