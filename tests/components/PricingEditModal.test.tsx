import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { PricingEditModal } from "@/components/usage/PricingEditModal";
import type { ModelPricing } from "@/types/usage";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: string | { defaultValue?: string }) =>
      typeof options === "string" ? options : (options?.defaultValue ?? key),
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

const updatePrice = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock("@/lib/query/usage", () => ({
  useUpdateModelPricing: () => ({
    mutateAsync: updatePrice,
    isPending: false,
  }),
}));

vi.mock("@/components/common/FullScreenPanel", () => ({
  FullScreenPanel: ({
    children,
    footer,
  }: {
    children: React.ReactNode;
    footer?: React.ReactNode;
  }) => (
    <div>
      {children}
      {footer}
    </div>
  ),
}));

const model: ModelPricing = {
  modelId: "gpt-6-astra",
  displayName: "GPT-6 Astra",
  inputCostPerMillion: "1",
  outputCostPerMillion: "3",
  cacheReadCostPerMillion: "0.0028",
  cacheCreationCostPerMillion: "0",
};

const PRICE_FIELDS = [
  { id: "inputCost", label: "Enter cost" },
  { id: "outputCost", label: "Output cost" },
  { id: "cacheReadCost", label: "Cache read cost" },
  { id: "cacheCreationCost", label: "cache write cost" },
] as const;

describe("PricingEditModal", () => {
  it("retains the explicit policy and warns that edits are user overrides", async () => {
    const metadata = {
      source: "foundry_reference_estimate" as const,
      policy: "all_input_at_base_rate" as const,
      actualCopilotPriceUnpublished: true,
      sourceUrl:
        "https://commandline.microsoft.com/microsoft-decision-1-model-foundry/",
      verifiedAt: "2026-11-01",
    };
    render(
      <PricingEditModal
        open
        model={{ ...model, modelId: "microsoft-decision-1", metadata }}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText(/Saving creates a User override/)).toBeVisible();
    expect(
      screen.getByText(/Policy retained: all input at base rate/),
    ).toBeVisible();
    fireEvent.change(document.getElementById("inputCost")!, {
      target: { value: "0.1" },
    });
    fireEvent.submit(document.getElementById("pricing-form")!);
    await waitFor(() =>
      expect(updatePrice).toHaveBeenLastCalledWith(
        expect.objectContaining({ inputCost: "0.1", metadata }),
      ),
    );
  });

  it.each([false, true])(
    "allows an optional long-context tier in isNew=%s",
    async (isNew) => {
      render(
        <PricingEditModal
          open
          isNew={isNew}
          model={model}
          onClose={() => {}}
        />,
      );
      const toggle = screen.getByRole("switch", {
        name: "Long-context pricing",
      });
      expect(toggle).not.toBeChecked();
      fireEvent.click(toggle);
      fireEvent.change(screen.getByLabelText("Input tokens (strictly above)"), {
        target: { value: "200000" },
      });
      fireEvent.change(screen.getByLabelText("Input (USD / 1M)"), {
        target: { value: "4" },
      });
      fireEvent.submit(document.getElementById("pricing-form")!);
      await waitFor(() =>
        expect(updatePrice).toHaveBeenCalledWith(
          expect.objectContaining({
            longContext: expect.objectContaining({
              thresholdInputTokens: 200000,
              inputCostPerMillion: "4",
            }),
          }),
        ),
      );
      fireEvent.click(toggle);
      expect(
        screen.queryByLabelText("Input tokens (strictly above)"),
      ).not.toBeInTheDocument();
      fireEvent.submit(document.getElementById("pricing-form")!);
      await waitFor(() =>
        expect(updatePrice).toHaveBeenLastCalledWith(
          expect.objectContaining({
            longContext: undefined,
          }),
        ),
      );
    },
  );

  it("preserves the long-context tier when editing a default price", async () => {
    const longContext = {
      thresholdInputTokens: 272000,
      inputCostPerMillion: "20",
      outputCostPerMillion: "75",
      cacheReadCostPerMillion: "2",
      cacheCreationCostPerMillion: "25",
    };
    render(
      <PricingEditModal
        open
        model={{ ...model, longContext }}
        onClose={() => {}}
      />,
    );
    fireEvent.change(document.getElementById("inputCost")!, {
      target: { value: "11" },
    });
    fireEvent.submit(document.getElementById("pricing-form")!);
    await waitFor(() =>
      expect(updatePrice).toHaveBeenCalledWith(
        expect.objectContaining({
          inputCost: "11",
          longContext,
        }),
      ),
    );
  });

  it("all price inputs have step=0.0001", () => {
    render(<PricingEditModal open model={model} onClose={() => {}} />);

    for (const { id } of PRICE_FIELDS) {
      const input = screen.getByLabelText(
        /per million tokens/i as unknown as string,
        {
          selector: `#${id}`,
        },
      ) as HTMLInputElement;
      expect(input).toHaveAttribute("step", "0.0001");
    }
  });

  it("accepts precise cache read cost like 0.0028", () => {
    render(<PricingEditModal open model={model} onClose={() => {}} />);

    const cacheReadInput = document.getElementById(
      "cacheReadCost",
    ) as HTMLInputElement;
    expect(cacheReadInput.value).toBe("0.0028");
    expect(cacheReadInput.checkValidity()).toBe(true);
  });

  it("allows user to input sub-cent prices via change event", () => {
    render(<PricingEditModal open model={model} onClose={() => {}} isNew />);

    const cacheReadInput = document.getElementById(
      "cacheReadCost",
    ) as HTMLInputElement;

    fireEvent.change(cacheReadInput, { target: { value: "0.0015" } });
    expect(cacheReadInput.value).toBe("0.0015");
    expect(
      screen.queryByRole("button", { name: /models\.dev/i }),
    ).not.toBeInTheDocument();
  });
});
