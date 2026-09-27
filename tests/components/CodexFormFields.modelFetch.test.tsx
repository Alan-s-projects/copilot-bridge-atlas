import { useEffect, useState, type ComponentProps } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { CodexFormFields } from "@/components/providers/forms/CodexFormFields";
import {
  copilotGetModels,
  copilotGetModelsForAccount,
  copilotOpenModelCatalog,
  type CopilotModel,
} from "@/lib/api/copilot";
import type { CodexCatalogModel } from "@/types";

vi.mock("@/lib/api/copilot", () => ({
  copilotGetModels: vi.fn(),
  copilotGetModelsForAccount: vi.fn(),
  copilotOpenModelCatalog: vi.fn(),
}));
vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
}));
vi.mock("@/components/providers/forms/CopilotAuthSection", () => ({
  CopilotAuthSection: () => null,
}));

type Props = ComponentProps<typeof CodexFormFields>;
function props(overrides: Partial<Props> = {}): Props {
  return {
    isCopilotAuthenticated: true,
    selectedGitHubAccountId: "account-a",
    catalogModels: [],
    onCatalogModelsChange: vi.fn(),
    ...overrides,
  };
}
function Harness(input: Props) {
  const [models, setModels] = useState<CodexCatalogModel[]>(
    input.catalogModels ?? [],
  );
  useEffect(() => {
    setModels(input.catalogModels ?? []);
  }, [input.catalogModels]);
  return (
    <>
      <CodexFormFields
        {...input}
        catalogModels={models}
        onCatalogModelsChange={(next) => {
          input.onCatalogModelsChange?.(next);
          setModels(next);
        }}
      />
    </>
  );
}
function model(id: string, endpoint = "/responses"): CopilotModel {
  return {
    id,
    name: id,
    vendor: "OpenAI",
    model_picker_enabled: true,
    supported_endpoints: [endpoint],
    context_window: 1048576,
  };
}
const fetchButton = () =>
  screen.getAllByRole("button", { name: "Refresh models" })[0];

describe("Copilot model catalog import", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    HTMLElement.prototype.scrollIntoView = vi.fn();
  });

  it("opens the generated JSON using the native file command", async () => {
    vi.mocked(copilotOpenModelCatalog).mockResolvedValue(undefined);
    render(<Harness {...props()} />);
    const buttons = screen.getAllByRole("button");
    const refresh = buttons.findIndex((button) =>
      button.textContent?.includes("Refresh models"),
    );
    expect(buttons[refresh + 1]).toHaveTextContent("Open generated JSON");
    fireEvent.click(
      screen.getByRole("button", { name: "Open generated JSON" }),
    );
    await waitFor(() =>
      expect(copilotOpenModelCatalog).toHaveBeenCalledTimes(1),
    );
  });

  it("reports a missing generated catalog and re-enables the open button", async () => {
    vi.mocked(copilotOpenModelCatalog).mockRejectedValue(
      new Error("Refresh models first"),
    );
    render(<Harness {...props()} />);
    const button = screen.getByRole("button", { name: "Open generated JSON" });
    fireEvent.click(button);
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("Refresh models first"),
    );
    expect(button).toBeEnabled();
  });

  it("orders enabled models first by name and toggles availability instead of removing rows", () => {
    const input = props({
      catalogModels: [
        { model: "gpt-zulu", displayName: "Zulu" },
        { model: "gpt-beta", displayName: "Beta", enabled: false },
        { model: "gpt-alpha", displayName: "Alpha" },
        { model: "gpt-charlie", displayName: "Charlie", enabled: false },
      ],
    });
    render(<Harness {...input} />);

    const displayNames = () =>
      screen
        .getAllByRole("heading", { level: 4 })
        .map((field) => field.textContent);
    expect(displayNames()).toEqual(["Alpha", "Zulu", "Beta", "Charlie"]);
    expect(
      screen
        .getAllByRole("switch", { name: /codexConfig.modelAvailableInCodex/ })
        .map((toggle) => toggle.getAttribute("data-state")),
    ).toEqual(["checked", "checked", "unchecked", "unchecked"]);
    expect(
      screen.queryByRole("button", { name: /Remove model/ }),
    ).not.toBeInTheDocument();

    fireEvent.click(
      screen.getAllByRole("switch", {
        name: /codexConfig.modelAvailableInCodex/,
      })[0],
    );
    expect(input.onCatalogModelsChange).toHaveBeenLastCalledWith([
      { model: "gpt-zulu", displayName: "Zulu" },
      { model: "gpt-beta", displayName: "Beta", enabled: false },
      { model: "gpt-alpha", displayName: "Alpha", enabled: false },
      { model: "gpt-charlie", displayName: "Charlie", enabled: false },
    ]);
    expect(displayNames()).toEqual(["Zulu", "Alpha", "Beta", "Charlie"]);
  });

  it("imports models from all vendors and both supported transports without a format selector", async () => {
    vi.mocked(copilotGetModelsForAccount).mockResolvedValue([
      model("gpt-6-astra"),
      model("gemini-future", "/chat/completions"),
      model("grok-future"),
      model("future-vendor/model"),
      model("unsupported-transport", "/messages"),
    ]);
    const input = props();
    render(<Harness {...input} />);
    fireEvent.click(fetchButton());
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "gpt-6-astra" }),
      ).toBeVisible(),
    );
    for (const id of ["gemini-future", "grok-future", "future-vendor/model"]) {
      expect(screen.getByRole("heading", { name: id })).toBeVisible();
    }
    expect(
      screen.queryByRole("heading", { name: "unsupported-transport" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("combobox", { name: "Upstream format" }),
    ).not.toBeInTheDocument();
    expect(copilotGetModelsForAccount).toHaveBeenCalledWith("account-a");
  });

  it("replaces explicit limits and imports live image support", async () => {
    vi.mocked(copilotGetModelsForAccount).mockResolvedValue([
      model("gpt-existing"),
      { ...model("gpt-new"), supports_vision: true },
    ]);
    const input = props({
      catalogModels: [
        {
          model: "gpt-existing",
          contextWindow: 400000,
          inputModalities: ["text"],
        },
      ],
    });
    render(<Harness {...input} />);
    fireEvent.click(fetchButton());
    await waitFor(() =>
      expect(input.onCatalogModelsChange).toHaveBeenCalledWith([
        expect.objectContaining({
          model: "gpt-existing",
          contextWindow: 1048576,
          inputModalities: ["text"],
        }),
        expect.objectContaining({
          model: "gpt-new",
          inputModalities: ["text", "image"],
          reasoningLevels: [],
          defaultReasoningLevel: undefined,
        }),
      ]),
    );
  });

  it("replaces reasoning overrides on refresh, matching model IDs without case sensitivity", async () => {
    vi.mocked(copilotGetModelsForAccount).mockResolvedValue([
      {
        ...model("gpt-6-astra"),
        supports_parallel_tool_calls: true,
        supports_vision: true,
        reasoning_efforts: ["low", "medium", "high"],
      },
      { ...model("gpt-6-luna"), reasoning_efforts: ["low", "high"] },
    ]);
    const input = props({
      catalogModels: [
        {
          model: "GPT-6-ASTRA",
          supportsParallelToolCalls: false,
          inputModalities: ["text"],
          reasoningLevels: ["low", "high", "ultra"],
          defaultReasoningLevel: "ultra",
        },
      ],
    });
    render(<Harness {...input} />);
    fireEvent.click(fetchButton());
    await waitFor(() =>
      expect(input.onCatalogModelsChange).toHaveBeenCalledWith([
        expect.objectContaining({
          model: "gpt-6-astra",
          supportsParallelToolCalls: true,
          inputModalities: ["text", "image"],
          reasoningLevels: ["low", "medium", "high"],
          defaultReasoningLevel: undefined,
        }),
        expect.objectContaining({
          model: "gpt-6-luna",
          reasoningLevels: ["low", "high"],
          defaultReasoningLevel: undefined,
        }),
      ]),
    );
    expect(screen.getAllByLabelText("Reasoning levels")[0]).toHaveTextContent(
      "low, medium, high",
    );
    expect(
      screen.getAllByLabelText("Reasoning levels")[0],
    ).not.toHaveTextContent("ultra");
  });

  it("discards results from an account that is no longer selected", async () => {
    let resolve!: (value: CopilotModel[]) => void;
    vi.mocked(copilotGetModelsForAccount).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const input = props();
    const { rerender } = render(<Harness {...input} />);
    fireEvent.click(fetchButton());
    rerender(<Harness {...input} selectedGitHubAccountId="account-b" />);
    await act(async () => {
      resolve([model("stale-model")]);
    });
    expect(
      screen.queryByRole("heading", { name: "stale-model" }),
    ).not.toBeInTheDocument();
    expect(input.onCatalogModelsChange).not.toHaveBeenCalled();
  });

  it("makes Copilot metadata read-only with only the enable switch editable", () => {
    const input = props({
      catalogModels: [
        {
          model: "gpt-6-astra",
          displayName: "GPT-6 Astra",
          contextWindow: 1050000,
          reasoningLevels: ["low", "medium"],
          defaultReasoningLevel: "medium",
        },
      ],
    });
    render(<Harness {...input} />);
    const reasoning = screen.getByLabelText("Reasoning levels");
    expect(reasoning).toHaveTextContent("low, medium");
    expect(
      screen.getByRole("heading", { name: "GPT-6 Astra" }),
    ).toHaveAttribute("title", "gpt-6-astra");
    expect(screen.getByLabelText("Context window")).toHaveTextContent(
      "1,050,000 / Not reported",
    );
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.queryByRole("spinbutton")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("combobox", { name: "Reasoning levels" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Add model" }),
    ).not.toBeInTheDocument();
    expect(input.onCatalogModelsChange).not.toHaveBeenCalled();
  });

  it("reports failures and releases the loading state", async () => {
    const failure = new Error("offline");
    vi.mocked(copilotGetModelsForAccount).mockRejectedValue(failure);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    render(<Harness {...props()} />);
    fireEvent.click(fetchButton());
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("offline"));
    await waitFor(() => expect(fetchButton()).not.toBeDisabled());
    warn.mockRestore();
  });

  it("reports an empty model list", async () => {
    vi.mocked(copilotGetModelsForAccount).mockResolvedValue([]);
    render(<Harness {...props()} />);
    fireEvent.click(fetchButton());
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        "No compatible chat models are available to this Copilot account.",
      ),
    );
  });

  it("explains the empty catalog when signed out and requires sign-in to fetch", () => {
    render(<Harness {...props({ isCopilotAuthenticated: false })} />);
    expect(screen.getByRole("status")).toHaveTextContent(
      /GitHub Copilot is signed out/,
    );
    fireEvent.click(fetchButton());
    expect(toast.error).toHaveBeenCalled();
    expect(copilotGetModelsForAccount).not.toHaveBeenCalled();
  });

  it("explains how to load models for a signed-in account with an empty catalog", () => {
    render(<Harness {...props()} />);
    expect(screen.getByRole("status")).toHaveTextContent(
      /No models are in this catalog yet/,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      /available to your GitHub Copilot account/,
    );
  });

  it("uses the default Copilot account when no account is pinned", async () => {
    vi.mocked(copilotGetModels).mockResolvedValue([
      model("gpt-default-account"),
    ]);
    render(<Harness {...props({ selectedGitHubAccountId: null })} />);
    fireEvent.click(fetchButton());
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "gpt-default-account" }),
      ).toBeVisible(),
    );
    expect(copilotGetModels).toHaveBeenCalledTimes(1);
  });

  it("preserves missing models and disabled choices across catalog refreshes", async () => {
    vi.mocked(copilotGetModelsForAccount).mockResolvedValue([
      model("gemini-new", "/chat/completions"),
    ]);
    const input = props({
      catalogModels: [
        { model: "grok-returning", enabled: false, reasoningLevels: ["high"] },
      ],
    });
    render(<Harness {...input} />);
    fireEvent.click(fetchButton());
    await waitFor(() => expect(input.onCatalogModelsChange).toHaveBeenCalled());
    expect(input.onCatalogModelsChange).toHaveBeenLastCalledWith([
      expect.objectContaining({ model: "gemini-new", available: true }),
      expect.objectContaining({
        model: "grok-returning",
        enabled: false,
        available: false,
        reasoningLevels: ["high"],
      }),
    ]);
    expect(
      screen.getAllByRole("switch", {
        name: /codexConfig.modelAvailableInCodex/,
      })[1],
    ).toBeDisabled();
    vi.mocked(copilotGetModelsForAccount).mockResolvedValue([
      model("gemini-new", "/chat/completions"),
      model("grok-returning"),
    ]);
    fireEvent.click(fetchButton());
    await waitFor(() =>
      expect(
        screen.getAllByRole("switch", {
          name: /codexConfig.modelAvailableInCodex/,
        })[1],
      ).toBeEnabled(),
    );
    expect(
      screen.getAllByRole("switch", {
        name: /codexConfig.modelAvailableInCodex/,
      })[1],
    ).not.toBeChecked();
  });

  it("marks saved models unavailable when a successful refresh returns no eligible models", async () => {
    vi.mocked(copilotGetModelsForAccount).mockResolvedValue([]);
    const input = props({ catalogModels: [{ model: "future-model" }] });
    render(<Harness {...input} />);
    fireEvent.click(fetchButton());
    await waitFor(() =>
      expect(input.onCatalogModelsChange).toHaveBeenCalledWith([
        { model: "future-model", available: false },
      ]),
    );
  });
});
