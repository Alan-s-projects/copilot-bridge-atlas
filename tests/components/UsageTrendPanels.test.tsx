import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageTrendChart } from "@/components/usage/UsageTrendChart";

const mocks = vi.hoisted(() => ({
  trends: vi.fn(),
  change: vi.fn(),
  grouping: { interval: 1, unit: "day" },
}));
vi.mock("@/hooks/useUsageTrendGrouping", () => ({
  useUsageTrendGrouping: () => ({
    grouping: mocks.grouping,
    isLoading: false,
    isSaving: false,
    change: mocks.change,
  }),
}));
vi.mock("@/lib/query/usage", () => ({ useUsageTrends: mocks.trends }));
vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: any) => <div>{children}</div>,
  BarChart: ({ children }: any) => <div>{children}</div>,
  LineChart: ({ children }: any) => <div>{children}</div>,
  Bar: ({ dataKey, hide }: any) => (
    <span data-testid={`bar-${dataKey}`} hidden={hide} />
  ),
  Line: ({ dataKey, hide, connectNulls, dot }: any) => (
    <span
      data-testid={`line-${dataKey}`}
      hidden={hide}
      data-connect-nulls={String(connectNulls)}
      data-dot={String(!!dot)}
    />
  ),
  XAxis: () => null,
  YAxis: () => null,
  CartesianGrid: () => null,
  Tooltip: () => null,
}));

beforeEach(() => {
  mocks.change.mockReset();
  mocks.trends.mockReset().mockReturnValue({
    data: { buckets: [], hasIncompleteRollupData: false },
    isLoading: false,
    isError: false,
  });
  mocks.grouping = { interval: 1, unit: "day" };
});

describe("Three usage trend panels", () => {
  it("shares the dashboard range and provides 1 Day grouping without a separate date control", () => {
    const range = { preset: "7d" } as const;
    render(
      <UsageTrendChart
        range={range}
        model="gpt-6-astra"
        refreshIntervalMs={60000}
      />,
    );
    for (const name of ["Cost chart", "Rates chart", "Token counts chart"]) {
      expect(screen.getByRole("region", { name })).toBeVisible();
    }
    expect(
      screen.getByRole("combobox", { name: "Grouping interval" }),
    ).toHaveTextContent("1");
    expect(
      screen.getByRole("combobox", { name: "Grouping unit" }),
    ).toHaveTextContent("Day");
    expect(screen.getAllByRole("combobox")).toHaveLength(2);
    expect(mocks.trends).toHaveBeenCalledWith(
      range,
      expect.objectContaining({ model: "gpt-6-astra" }),
      expect.objectContaining({ refetchInterval: 60000 }),
      { interval: 1, unit: "day" },
    );
  });

  it("toggles individual token bars and rate lines while keeping their legends", () => {
    render(
      <UsageTrendChart range={{ preset: "today" }} refreshIntervalMs={0} />,
    );
    const legend = within(
      screen.getByRole("region", { name: "Token counts chart" }),
    );
    const input = legend.getByRole("button", { name: "Fresh Input" });
    fireEvent.click(input);
    expect(input).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByTestId("bar-inputTokens")).not.toBeVisible();
    expect(screen.getByTestId("bar-cacheReadTokens")).toBeVisible();
    fireEvent.click(input);
    expect(screen.getByTestId("bar-inputTokens")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Read cache hit" }));
    expect(screen.getByTestId("line-cacheHitRate")).not.toBeVisible();
    expect(screen.getByTestId("line-successRate")).toHaveAttribute(
      "data-connect-nulls",
      "false",
    );
    expect(screen.getByTestId("line-successRate")).toHaveAttribute(
      "data-dot",
      "true",
    );
  });

  it("shows the archived-detail limitation instead of silently fabricating finer data", () => {
    mocks.trends.mockReturnValue({
      data: { buckets: [], hasIncompleteRollupData: true },
      isLoading: false,
    });
    render(<UsageTrendChart range={{ preset: "30d" }} refreshIntervalMs={0} />);
    expect(screen.getByRole("status")).toHaveTextContent(
      "daily-only archived data",
    );
  });
});
