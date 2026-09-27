import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PricingConfigPanel } from "@/components/usage/PricingConfigPanel";
import type { ModelPricing } from "@/types/usage";

const mocks = vi.hoisted(() => ({
  pricing: vi.fn(),
  remove: vi.fn(),
  reset: vi.fn(),
  openSource: vi.fn(),
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string } | string) =>
      typeof options === "string" ? options : (options?.defaultValue ?? key),
  }),
}));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openSource },
}));
vi.mock("@/lib/query/usage", () => ({
  useModelPricing: mocks.pricing,
  useDeleteModelPricing: () => ({ mutate: mocks.remove, isPending: false }),
  useResetModelPricingToDefaults: () => ({
    mutate: mocks.reset,
    isPending: false,
  }),
}));
vi.mock("@/components/usage/PricingEditModal", () => ({
  PricingEditModal: ({
    model,
    isNew,
    onClose,
  }: {
    model: ModelPricing;
    isNew: boolean;
    onClose: () => void;
  }) => (
    <div role="dialog" aria-label="Pricing editor">
      {isNew ? "New model" : model.modelId}
      <button onClick={onClose}>Close editor</button>
    </div>
  ),
}));

const model: ModelPricing = {
  modelId: "gpt-6-astra",
  displayName: "GPT-6 Astra",
  inputCostPerMillion: "1.25",
  outputCostPerMillion: "5",
  cacheReadCostPerMillion: "0.125",
  cacheCreationCostPerMillion: "0",
};

beforeEach(() => {
  mocks.pricing
    .mockReset()
    .mockReturnValue({ data: [model], isLoading: false });
  mocks.remove
    .mockReset()
    .mockImplementation((_id, options) => options.onSuccess());
  mocks.reset
    .mockReset()
    .mockImplementation((_value, options) => options.onSuccess());
  mocks.openSource.mockReset().mockResolvedValue(undefined);
});

describe("Manual pricing configuration", () => {
  it("shows long-context thresholds and all four rates beside the default model", () => {
    mocks.pricing.mockReturnValue({
      data: [
        {
          ...model,
          longContext: {
            thresholdInputTokens: 272000,
            inputCostPerMillion: "20",
            outputCostPerMillion: "75",
            cacheReadCostPerMillion: "2",
            cacheCreationCostPerMillion: "25",
          },
        },
      ],
      isLoading: false,
    });
    render(<PricingConfigPanel />);
    const tier = screen
      .getByText(/Long context: > 272,000 input tokens/)
      .closest("tr")!;
    for (const price of ["$20", "$75", "$2", "$25"]) {
      expect(within(tier).getByText(price)).toBeVisible();
    }
    expect(screen.getByText(model.modelId)).toBeVisible();
  });

  it("keeps model prices without pricing defaults or sync controls", () => {
    render(<PricingConfigPanel />);
    expect(screen.getByText("gpt-6-astra")).toBeVisible();
    expect(screen.getByText("$1.25")).toBeVisible();
    expect(screen.getByText("$0.125")).toBeVisible();
    expect(screen.getAllByRole("table")).toHaveLength(1);
    expect(
      screen.queryByText(/models\.dev|pricingDefaults/),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "common.save" }),
    ).not.toBeInTheDocument();
  });

  it("still opens editors for existing and new model prices", () => {
    render(<PricingConfigPanel />);
    fireEvent.click(screen.getByTitle("common.edit"));
    expect(
      screen.getByRole("dialog", { name: "Pricing editor" }),
    ).toHaveTextContent(model.modelId);
    fireEvent.click(screen.getByRole("button", { name: "Close editor" }));
    fireEvent.click(screen.getByRole("button", { name: "common.add" }));
    expect(
      screen.getByRole("dialog", { name: "Pricing editor" }),
    ).toHaveTextContent("New model");
  });

  it("still deletes a model only after confirmation", () => {
    render(<PricingConfigPanel />);
    fireEvent.click(screen.getByTitle("common.delete"));
    expect(mocks.remove).not.toHaveBeenCalled();
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "common.delete",
      }),
    );
    expect(mocks.remove).toHaveBeenCalledWith(model.modelId, expect.anything());
  });

  it("opens the bundled defaults source and confirms a scoped pricing reset", () => {
    render(<PricingConfigPanel />);

    const source = screen.getByRole("link", { name: "View built-in prices" });
    expect(source).toHaveAttribute(
      "href",
      "https://github.com/Alan-s-projects/copilot-bridge-atlas/blob/atlas/src-tauri/src/resources/model-pricing.json",
    );
    fireEvent.click(source);
    expect(mocks.openSource).toHaveBeenCalledWith(
      "https://github.com/Alan-s-projects/copilot-bridge-atlas/blob/atlas/src-tauri/src/resources/model-pricing.json",
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Reset to code defaults" }),
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("deletion tombstones");
    expect(dialog).toHaveTextContent(
      "Custom models without a bundled default will become unpriced.",
    );
    expect(dialog).toHaveTextContent(
      "Retired metadata and previously recorded request costs are unchanged.",
    );
    expect(
      within(dialog).getByRole("link", {
        name: "View bundled defaults in model-pricing.json",
      }),
    ).toBeVisible();
    expect(mocks.reset).not.toHaveBeenCalled();

    fireEvent.click(
      within(dialog).getByRole("button", { name: "Reset prices" }),
    );
    expect(mocks.reset).toHaveBeenCalledWith(undefined, expect.anything());
    expect(
      screen.queryByRole("dialog", { name: /Reset prices/ }),
    ).not.toBeInTheDocument();
  });

  it("filters model IDs and display names immediately without changing prices", () => {
    mocks.pricing.mockReturnValue({
      isLoading: false,
      data: [
        model,
        {
          ...model,
          modelId: "gemini-future",
          displayName: "Gemini Future Flash",
        },
        { ...model, modelId: "custom/next", displayName: "New Vendor Agent" },
      ],
    });
    render(<PricingConfigPanel />);
    const search = screen.getByRole("textbox", { name: "Search model prices" });
    fireEvent.change(search, { target: { value: "  GEMINI   flash  " } });
    expect(screen.getByText("gemini-future")).toBeVisible();
    expect(screen.queryByText(model.modelId)).not.toBeInTheDocument();
    expect(screen.queryByText("custom/next")).not.toBeInTheDocument();
    fireEvent.change(search, { target: { value: "custom/next" } });
    expect(screen.getByText("New Vendor Agent")).toBeVisible();
    expect(screen.queryByText("gemini-future")).not.toBeInTheDocument();
    fireEvent.change(search, { target: { value: "new vendor" } });
    expect(screen.getByText("custom/next")).toBeVisible();
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(mocks.reset).not.toHaveBeenCalled();
  });

  it("shows an empty state and clears the filter with the clear button or Escape", () => {
    render(<PricingConfigPanel />);
    const search = screen.getByRole("textbox", { name: "Search model prices" });
    fireEvent.change(search, { target: { value: "unmatched-model" } });
    expect(screen.getByRole("status")).toHaveTextContent(
      "No matching model prices.",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Clear pricing search" }),
    );
    expect(search).toHaveValue("");
    expect(screen.getByText(model.modelId)).toBeVisible();
    fireEvent.change(search, { target: { value: "another-miss" } });
    fireEvent.keyDown(search, { key: "Escape" });
    expect(search).toHaveValue("");
    expect(screen.getByText(model.modelId)).toBeVisible();
  });
});
