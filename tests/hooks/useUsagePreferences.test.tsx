import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useUsageDateRange,
  normalizeUsageDateRange,
} from "@/hooks/useUsageDateRange";
import { useUsageTrendGrouping } from "@/hooks/useUsageTrendGrouping";
import { normalizeTrendGrouping } from "@/lib/trendGrouping";
import { createTestQueryClient } from "../utils/testQueryClient";

const mocks = vi.hoisted(() => ({
  getRange: vi.fn(),
  setRange: vi.fn(),
  getGrouping: vi.fn(),
  setGrouping: vi.fn(),
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: {
    getUsageDateRange: mocks.getRange,
    setUsageDateRange: mocks.setRange,
    getUsageTrendGrouping: mocks.getGrouping,
    setUsageTrendGrouping: mocks.setGrouping,
  },
}));

beforeEach(() => {
  mocks.getRange.mockReset().mockResolvedValue({ preset: "7d" });
  mocks.getGrouping
    .mockReset()
    .mockResolvedValue({ interval: 3, unit: "hour" });
  mocks.setRange.mockReset().mockImplementation(async (value) => {
    mocks.getRange.mockResolvedValue(value);
    return value;
  });
  mocks.setGrouping.mockReset().mockImplementation(async (value) => {
    mocks.getGrouping.mockResolvedValue(value);
    return value;
  });
});

function setup() {
  const client = createTestQueryClient();
  return renderHook(
    () => ({ range: useUsageDateRange(), grouping: useUsageTrendGrouping() }),
    {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    },
  );
}

describe("Usage preferences", () => {
  it("orders rapid date changes so the latest selection stays saved", async () => {
    const { result } = setup();
    await waitFor(() =>
      expect(result.current.range.range).toEqual({ preset: "7d" }),
    );
    act(() => {
      result.current.range.change({ preset: "14d" });
      result.current.range.change({ preset: "30d" });
    });
    await waitFor(() => expect(mocks.setRange).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(result.current.range.range).toEqual({ preset: "30d" }),
    );
    expect(mocks.setRange.mock.calls.map(([range]) => range.preset)).toEqual([
      "14d",
      "30d",
    ]);
  });

  it("loads and auto-saves range and grouping independently", async () => {
    const { result } = setup();
    await waitFor(() =>
      expect(result.current.range.range).toEqual({ preset: "7d" }),
    );
    expect(result.current.grouping.grouping).toEqual({
      interval: 3,
      unit: "hour",
    });
    act(() => {
      result.current.range.change({ preset: "14d" });
      result.current.grouping.change({ interval: 5, unit: "minute" });
    });
    await waitFor(() =>
      expect(mocks.setRange).toHaveBeenCalledWith(
        { preset: "14d" },
        expect.anything(),
      ),
    );
    await waitFor(() =>
      expect(result.current.grouping.grouping).toEqual({
        interval: 5,
        unit: "minute",
      }),
    );
    expect(result.current.range.range).toEqual({ preset: "14d" });
  });

  it("falls back to code defaults on missing or invalid saved settings", () => {
    expect(normalizeTrendGrouping({ interval: 2, unit: "hour" })).toEqual({
      interval: 1,
      unit: "day",
    });
    expect(normalizeTrendGrouping(undefined)).toEqual({
      interval: 1,
      unit: "day",
    });
    expect(normalizeUsageDateRange({ preset: "unknown" })).toEqual({
      preset: "today",
    });
    expect(normalizeUsageDateRange({ preset: "custom" })).toEqual({
      preset: "today",
    });
    expect(
      normalizeUsageDateRange({
        preset: "custom",
        customStartDate: 10,
        customEndDate: 20,
      }),
    ).toMatchObject({
      preset: "custom",
      customStartDate: 10,
      customEndDate: 20,
    });
  });
});
