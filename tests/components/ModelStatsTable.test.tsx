import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ModelStatsTable } from "@/components/usage/ModelStatsTable";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("@/lib/query/usage", () => ({
  useModelStats: () => ({
    isLoading: false,
    data: [
      {
        model: "gpt-6-astra",
        requestCount: 3,
        totalTokens: 999999,
        totalInputTokens: 1000,
        totalCacheReadTokens: 8000,
        totalCacheCreationTokens: 1000,
        totalOutputTokens: 500,
        totalCost: "1.5",
        avgCostPerRequest: "0.5",
      },
    ],
  }),
}));

describe("Model stats token details", () => {
  it("uses billing-model order and input-only, token-weighted read hit rate", () => {
    render(
      <ModelStatsTable range={{ preset: "today" }} refreshIntervalMs={0} />,
    );
    expect(
      screen
        .getAllByRole("columnheader")
        .map((header) => header.textContent?.replace(/\s+/g, " ").trim()),
    ).toEqual([
      "Billing Model",
      "Requests",
      "Total Input(fresh/cached/hit)",
      "Total Output",
      "Total Cache Write",
      "Total Cost",
      "Average Cost",
    ]);
    const cells = within(screen.getAllByRole("row")[1]).getAllByRole("cell");
    expect(cells[2]).toHaveTextContent("1.0K / 8.0K / 80.0%");
    expect(cells[3]).toHaveTextContent("500");
    expect(cells[4]).toHaveTextContent("1.0K");
    expect(cells[5]).toHaveTextContent("$1.5000");
    expect(cells[6]).toHaveTextContent("$0.500000");
    expect(screen.queryByText("999,999")).not.toBeInTheDocument();
  });
});
