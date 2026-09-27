import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RequestLogTable } from "@/components/usage/RequestLogTable";
import type { UsageRangeSelection } from "@/types/usage";

const useRequestLogsMock = vi.hoisted(() => vi.fn());

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (
      key: string,
      options?: {
        defaultValue?: string;
      },
    ) => options?.defaultValue ?? key,
    i18n: {
      resolvedLanguage: "en",
      language: "en",
    },
  }),
}));

vi.mock("@/lib/query/usage", () => ({
  useRequestLogs: (args: unknown) => useRequestLogsMock(args),
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: any) => (
    <button {...props}>{children}</button>
  ),
}));

vi.mock("@/components/ui/input", () => ({
  Input: (props: any) => <input {...props} />,
}));

vi.mock("@/components/ui/select", () => ({
  Select: ({ children }: any) => <div>{children}</div>,
  SelectTrigger: ({ children, ...props }: any) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  SelectValue: ({ placeholder }: any) => <span>{placeholder ?? null}</span>,
  SelectContent: () => null,
  SelectItem: () => null,
}));

vi.mock("@/components/ui/table", () => ({
  Table: ({ children }: any) => <table>{children}</table>,
  TableBody: ({ children }: any) => <tbody>{children}</tbody>,
  TableCell: ({ children, ...props }: any) => <td {...props}>{children}</td>,
  TableHead: ({ children, ...props }: any) => <th {...props}>{children}</th>,
  TableHeader: ({ children }: any) => <thead>{children}</thead>,
  TableRow: ({ children }: any) => <tr>{children}</tr>,
}));

describe("RequestLogTable", () => {
  beforeEach(() => {
    useRequestLogsMock.mockReset();
    useRequestLogsMock.mockImplementation(
      ({ page = 0, pageSize = 20 }: { page?: number; pageSize?: number }) => ({
        data: {
          data: [],
          total: 120,
          page,
          pageSize,
        },
        isLoading: false,
      }),
    );
  });

  it.each([false, true])(
    "omits Provider and Source columns and keeps cells aligned (has logs=%s)",
    (hasLogs) => {
      useRequestLogsMock.mockReturnValue({
        isLoading: false,
        data: {
          data: hasLogs
            ? [
                {
                  requestId: "request-1",
                  providerName: "GitHub Copilot",
                  model: "gpt-6-astra",
                  requestedReasoningEffort: "ultra",
                  appliedReasoningEffort: "max",
                  createdAt: 1790400000,
                  inputTokens: 100,
                  outputTokens: 20,
                  cacheReadTokens: 0,
                  cacheCreationTokens: 0,
                  latencyMs: 1000,
                  statusCode: 200,
                  totalCostUsd: "0.01",
                  costMultiplier: "1",
                  dataSource: "proxy",
                },
              ]
            : [],
          total: hasLogs ? 1 : 0,
          page: 0,
          pageSize: 20,
        },
      });
      render(
        <RequestLogTable
          range={{ preset: "today" }}
          rangeLabel="Today"
          appType="codex"
          refreshIntervalMs={0}
        />,
      );
      const headings = screen.getAllByRole("columnheader");
      expect(headings).toHaveLength(8);
      expect(headings[1]).toHaveTextContent("usage.billingModel");
      expect(headings[2]).toHaveTextContent("Reasoning(requested/applied)");
      expect(
        screen.queryByRole("columnheader", { name: "usage.provider" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("columnheader", { name: "Source" }),
      ).not.toBeInTheDocument();
      expect(screen.queryByText("GitHub Copilot")).not.toBeInTheDocument();
      expect(screen.queryByText("proxy")).not.toBeInTheDocument();
      if (hasLogs) {
        expect(screen.getAllByRole("cell")).toHaveLength(8);
        expect(screen.getAllByRole("cell")[2]).toHaveTextContent("ultra / max");
        expect(screen.getByText("gpt-6-astra")).toBeVisible();
        expect(screen.getByText("$0.0100")).toBeVisible();
      } else {
        expect(screen.getByRole("cell")).toHaveAttribute("colspan", "8");
      }
    },
  );

  it("resets pagination when the dashboard range changes", async () => {
    const initialRange: UsageRangeSelection = { preset: "today" };
    const nextRange: UsageRangeSelection = {
      preset: "custom",
      customStartDate: 1_710_000_000,
      customEndDate: 1_710_086_400,
    };

    const { rerender } = render(
      <RequestLogTable
        range={initialRange}
        rangeLabel="Today"
        appType="codex"
        refreshIntervalMs={0}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "2" }));

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 1,
          range: initialRange,
        }),
      );
    });

    rerender(
      <RequestLogTable
        range={nextRange}
        rangeLabel="Custom"
        appType="codex"
        refreshIntervalMs={0}
      />,
    );

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 0,
          range: nextRange,
        }),
      );
    });
  });

  it("resets pagination when the dashboard model filter changes", async () => {
    const range: UsageRangeSelection = { preset: "today" };
    const { rerender } = render(
      <RequestLogTable
        range={range}
        rangeLabel="Today"
        appType="codex"
        refreshIntervalMs={0}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "2" }));

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 1,
          range,
        }),
      );
    });

    rerender(
      <RequestLogTable
        range={range}
        rangeLabel="Today"
        appType="codex"
        model="gpt-6-astra"
        refreshIntervalMs={0}
      />,
    );

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 0,
          range,
        }),
      );
    });
  });
});
