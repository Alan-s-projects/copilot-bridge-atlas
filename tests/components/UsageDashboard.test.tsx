import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ComponentProps } from "react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageDashboard } from "@/components/usage/UsageDashboard";

const useModelStatsMock = vi.hoisted(() => vi.fn());
const useUnpricedModelUsageMock = vi.hoisted(() => vi.fn());
const usageHeroMock = vi.hoisted(() => vi.fn());
const retryPriceCheckMock = vi.hoisted(() => vi.fn());

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string | { defaultValue?: string }) =>
      typeof fallback === "string" ? fallback : (fallback?.defaultValue ?? key),
    i18n: {
      resolvedLanguage: "en",
      language: "en",
    },
  }),
}));

vi.mock("framer-motion", () => ({
  motion: {
    div: ({ children, ...props }: any) => <div {...props}>{children}</div>,
  },
}));

vi.mock("@/hooks/useUsageEventBridge", () => ({
  useUsageEventBridge: () => {},
}));

vi.mock("@/lib/query/usage", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/query/usage")>(
      "@/lib/query/usage",
    );
  return {
    ...actual,
    useModelStats: (...args: unknown[]) => useModelStatsMock(...args),
    useUnpricedModelUsage: (...args: unknown[]) =>
      useUnpricedModelUsageMock(...args),
  };
});

vi.mock("@/components/usage/UsageHero", () => ({
  UsageHero: (props: unknown) => {
    usageHeroMock(props);
    return <div data-testid="usage-hero" />;
  },
}));

vi.mock("@/components/usage/UsageTrendChart", () => ({
  UsageTrendChart: () => <div data-testid="usage-trend" />,
}));

vi.mock("@/components/usage/RequestLogTable", () => ({
  RequestLogTable: () => <div data-testid="request-log-table" />,
}));

vi.mock("@/components/usage/ModelStatsTable", () => ({
  ModelStatsTable: () => <div data-testid="model-stats-table" />,
}));

vi.mock("@/components/usage/PricingConfigPanel", () => ({
  PricingConfigPanel: () => <div data-testid="pricing-config-panel" />,
}));

vi.mock("@/components/usage/UsageDateRangePicker", () => ({
  UsageDateRangePicker: () => <button type="button">date-range</button>,
}));

vi.mock("@/components/ui/select", () => ({
  Select: ({ value, onValueChange, children }: any) => (
    <div data-testid={`select-${value}`}>
      {children}
      <button type="button" onClick={() => onValueChange?.("5000")}>
        choose-5000
      </button>
    </div>
  ),
  SelectTrigger: ({ children, ...props }: any) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  SelectValue: () => null,
  SelectContent: ({ children }: any) => <div>{children}</div>,
  SelectItem: ({ children, ...props }: any) => <div {...props}>{children}</div>,
}));

const renderDashboard = (props: ComponentProps<typeof UsageDashboard> = {}) => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <UsageDashboard {...props} />
    </QueryClientProvider>,
  );
};

describe("UsageDashboard", () => {
  beforeEach(() => {
    useModelStatsMock.mockReset();
    useUnpricedModelUsageMock.mockReset();
    retryPriceCheckMock.mockReset();
    usageHeroMock.mockReset();
    useModelStatsMock.mockReturnValue({ data: [] });
    useUnpricedModelUsageMock.mockReturnValue({
      data: [],
      isError: false,
      isFetching: false,
      refetch: retryPriceCheckMock,
    });
  });

  it("uses the saved refresh interval when mounted", () => {
    renderDashboard({ refreshIntervalMs: 5000 });

    expect(screen.getByTestId("select-5000")).toBeInTheDocument();
  });

  it("keeps the dashboard scoped to Codex without exposing retired clients", async () => {
    renderDashboard();

    expect(
      screen.queryByRole("button", { name: "usage.appFilter.pi" }),
    ).not.toBeInTheDocument();

    expect(useModelStatsMock).toHaveBeenLastCalledWith(
      expect.anything(),
      { appType: "codex" },
      expect.anything(),
    );
    expect(usageHeroMock).toHaveBeenLastCalledWith(
      expect.objectContaining({ appType: "codex" }),
    );
    expect(screen.queryByTitle("Codex")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("img", { name: "Codex" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("usage.allSources")).not.toBeInTheDocument();
    expect(screen.queryByTitle("usage.filterBySource")).not.toBeInTheDocument();
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "usage.requestLogs",
      "usage.modelStats",
      "Cost Pricing",
    ]);
    expect(screen.getByTestId("request-log-table")).toBeVisible();
    expect(
      screen.queryByTestId("pricing-config-panel"),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("tab", { name: "usage.modelStats" }),
    );
    expect(await screen.findByTestId("model-stats-table")).toBeVisible();
    await userEvent.click(screen.getByRole("tab", { name: "Cost Pricing" }));
    expect(await screen.findByTestId("pricing-config-panel")).toBeVisible();
    expect(screen.queryByTestId("model-stats-table")).not.toBeInTheDocument();
    expect(screen.queryByTestId("request-log-table")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /settings\.advanced\.pricing/ }),
    ).not.toBeInTheDocument();
  });

  it("warns for unpriced model usage with token metrics and opens Cost Pricing", async () => {
    useUnpricedModelUsageMock.mockReturnValue({
      data: [
        {
          model: "gpt-7-future",
          requestCount: 3,
          freshInputTokens: 1200,
          outputTokens: 500,
          cacheReadTokens: 800,
          cacheHitRate: 0.4,
        },
      ],
      isError: false,
      isFetching: false,
      refetch: retryPriceCheckMock,
    });

    renderDashboard();

    const warning = screen.getByRole("alert");
    expect(warning).toHaveTextContent("Unpriced model usage");
    expect(warning).toHaveTextContent("gpt-7-future");
    expect(warning).toHaveTextContent("1.2K");
    expect(warning).toHaveTextContent("500");
    expect(warning).toHaveTextContent("800");
    expect(warning).toHaveTextContent("40.0%");
    await userEvent.click(
      within(warning).getByRole("button", { name: "Review pricing" }),
    );
    expect(await screen.findByTestId("pricing-config-panel")).toBeVisible();
    expect(screen.getByRole("tab", { name: "Cost Pricing" })).toHaveAttribute(
      "data-state",
      "active",
    );
  });

  it("reports a price-check failure without claiming prices are missing", async () => {
    useUnpricedModelUsageMock.mockReturnValue({
      data: undefined,
      isError: true,
      isFetching: false,
      refetch: retryPriceCheckMock,
    });

    renderDashboard();

    const warning = screen.getByRole("alert");
    expect(warning).toHaveTextContent("Could not check prices");
    expect(warning).not.toHaveTextContent("Unpriced model usage");
    await userEvent.click(
      within(warning).getByRole("button", { name: "Retry price check" }),
    );
    expect(retryPriceCheckMock).toHaveBeenCalledOnce();
  });

  it("does not show an unpriced warning when the range contains no unpriced usage", () => {
    useUnpricedModelUsageMock.mockReturnValue({
      data: [],
      isError: false,
      isFetching: false,
      refetch: retryPriceCheckMock,
    });
    renderDashboard();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("persists refresh interval changes", async () => {
    const onRefreshIntervalChange = vi.fn().mockResolvedValue(true);
    renderDashboard({ onRefreshIntervalChange });

    fireEvent.click(
      within(screen.getByTestId("select-30000")).getByRole("button", {
        name: "choose-5000",
      }),
    );

    await waitFor(() =>
      expect(onRefreshIntervalChange).toHaveBeenCalledWith(5000),
    );
    expect(screen.getByTestId("select-5000")).toBeInTheDocument();
  });

  it("rolls back optimistic interval changes when persistence fails", async () => {
    const onRefreshIntervalChange = vi.fn().mockResolvedValue(false);
    renderDashboard({ onRefreshIntervalChange });

    fireEvent.click(
      within(screen.getByTestId("select-30000")).getByRole("button", {
        name: "choose-5000",
      }),
    );

    await waitFor(() =>
      expect(onRefreshIntervalChange).toHaveBeenCalledWith(5000),
    );
    await waitFor(() =>
      expect(screen.getByTestId("select-30000")).toBeInTheDocument(),
    );
  });
});
