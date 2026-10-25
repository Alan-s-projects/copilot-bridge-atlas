import { createContext, useContext } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, within } from "@testing-library/react";
import type { ComponentProps } from "react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageDashboard } from "@/components/usage/UsageDashboard";

const useModelStatsMock = vi.hoisted(() => vi.fn());
const useUnpricedModelUsageMock = vi.hoisted(() => vi.fn());
const useUsageClientsMock = vi.hoisted(() => vi.fn());
const usageTrendMock = vi.hoisted(() => vi.fn());
const requestLogMock = vi.hoisted(() => vi.fn());
const modelTableMock = vi.hoisted(() => vi.fn());
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
    useUsageClients: () => useUsageClientsMock(),
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
  UsageTrendChart: (props: unknown) => {
    usageTrendMock(props);
    return <div data-testid="usage-trend" />;
  },
}));

vi.mock("@/components/usage/RequestLogTable", () => ({
  RequestLogTable: (props: unknown) => {
    requestLogMock(props);
    return <div data-testid="request-log-table" />;
  },
}));

vi.mock("@/components/usage/ModelStatsTable", () => ({
  ModelStatsTable: (props: unknown) => {
    modelTableMock(props);
    return <div data-testid="model-stats-table" />;
  },
}));

vi.mock("@/components/usage/PricingConfigPanel", () => ({
  PricingConfigPanel: () => <div data-testid="pricing-config-panel" />,
}));

vi.mock("@/components/usage/UsageDateRangePicker", () => ({
  UsageDateRangePicker: () => <button type="button">date-range</button>,
}));

const SelectContext = createContext<(value: string) => void>(() => {});
vi.mock("@/components/ui/select", () => ({
  Select: ({ value, onValueChange, children }: any) => (
    <SelectContext.Provider value={onValueChange}>
      <div data-testid={`select-${value}`}>{children}</div>
    </SelectContext.Provider>
  ),
  SelectTrigger: ({ children, ...props }: any) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  SelectValue: ({ children }: any) => children ?? null,
  SelectContent: ({ children }: any) => <div>{children}</div>,
  SelectItem: ({ children, value }: any) => {
    const change = useContext(SelectContext);
    return (
      <button type="button" role="option" onClick={() => change(value)}>
        {children}
      </button>
    );
  },
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
    usageTrendMock.mockReset();
    requestLogMock.mockReset();
    modelTableMock.mockReset();
    useUsageClientsMock.mockReturnValue({
      data: ["Codex", "Claude Desktop", "all"],
      isError: false,
    });
    useModelStatsMock.mockReturnValue({ data: [] });
    useUnpricedModelUsageMock.mockReturnValue({
      data: [],
      isError: false,
      isFetching: false,
      refetch: retryPriceCheckMock,
    });
  });

  it("uses a fixed 30-second interval without a refresh selector or visible page title", () => {
    renderDashboard();
    expect(usageHeroMock).toHaveBeenLastCalledWith(
      expect.objectContaining({ refreshIntervalMs: 30000 }),
    );
    expect(useModelStatsMock).toHaveBeenLastCalledWith(
      expect.anything(),
      expect.anything(),
      { refetchInterval: 30000 },
    );
    expect(
      screen.queryByLabelText("usage.refreshInterval"),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "usage.title" })).toHaveClass(
      "sr-only",
    );
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

  it("combines client and model filters for every Usage view and clears them independently", async () => {
    useModelStatsMock.mockReturnValue({
      data: [{ model: "gpt-6-astra" }, { model: "all" }],
    });
    renderDashboard();
    fireEvent.click(screen.getByRole("option", { name: "Claude Desktop" }));
    fireEvent.click(screen.getByRole("option", { name: "gpt-6-astra" }));
    const filters = expect.objectContaining({
      clientName: "Claude Desktop",
      model: "gpt-6-astra",
      refreshIntervalMs: 30000,
    });
    expect(usageHeroMock).toHaveBeenLastCalledWith(filters);
    expect(usageTrendMock).toHaveBeenLastCalledWith(filters);
    expect(requestLogMock).toHaveBeenLastCalledWith(filters);
    expect(useUnpricedModelUsageMock).toHaveBeenLastCalledWith(
      expect.anything(),
      expect.objectContaining({
        clientName: "Claude Desktop",
        model: "gpt-6-astra",
      }),
      expect.anything(),
    );
    await userEvent.click(
      screen.getByRole("tab", { name: "usage.modelStats" }),
    );
    expect(modelTableMock).toHaveBeenLastCalledWith(filters);
    fireEvent.click(screen.getByRole("option", { name: "All clients" }));
    expect(usageHeroMock).toHaveBeenLastCalledWith(
      expect.objectContaining({ clientName: undefined, model: "gpt-6-astra" }),
    );
    fireEvent.click(screen.getByRole("option", { name: "usage.allModels" }));
    expect(usageHeroMock).toHaveBeenLastCalledWith(
      expect.objectContaining({ clientName: undefined, model: undefined }),
    );
  });

  it("keeps a client named all distinct from the all-clients option", () => {
    renderDashboard();
    fireEvent.click(screen.getByRole("option", { name: "all" }));
    expect(usageHeroMock).toHaveBeenLastCalledWith(
      expect.objectContaining({ clientName: "all" }),
    );
  });
});
