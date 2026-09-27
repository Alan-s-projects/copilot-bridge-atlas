import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useUsageTableColumns } from "@/hooks/useUsageTableColumns";
import { createTestQueryClient } from "../utils/testQueryClient";

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  set: vi.fn(),
  error: vi.fn(),
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: {
    getUsageTableColumns: mocks.get,
    setUsageTableColumns: mocks.set,
  },
}));
vi.mock("sonner", () => ({ toast: { error: mocks.error } }));

function setup() {
  const client = createTestQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(() => useUsageTableColumns(), { wrapper });
}

beforeEach(() => {
  mocks.get.mockReset().mockResolvedValue({
    requestLogs: ["time", "cost"],
    modelStats: ["model", "cost"],
  });
  mocks.set.mockReset();
  mocks.error.mockReset();
});

describe("Saved usage columns", () => {
  it("falls back silently to code defaults when loading fails", async () => {
    mocks.get.mockRejectedValue(new Error("Unavailable"));
    const { result } = setup();
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(result.current.data).toEqual({});
    expect(mocks.get).toHaveBeenCalledTimes(1);
    expect(mocks.error).not.toHaveBeenCalled();
  });

  it("auto-saves one table while preserving the other and restores a failed edit", async () => {
    const { result } = setup();
    await waitFor(() =>
      expect(result.current.data.requestLogs).toEqual(["time", "cost"]),
    );
    mocks.set.mockResolvedValue({
      requestLogs: ["model"],
      modelStats: ["model", "cost"],
    });
    await act(() => result.current.change("requestLogs", ["model"]));
    expect(mocks.set).toHaveBeenCalledWith("requestLogs", ["model"]);
    expect(result.current.data.modelStats).toEqual(["model", "cost"]);
    mocks.set.mockRejectedValue(new Error("Disk full"));
    await act(async () => {
      await expect(
        result.current.change("modelStats", ["requests"]),
      ).rejects.toThrow("Disk full");
    });
    expect(result.current.data.modelStats).toEqual(["model", "cost"]);
    expect(result.current.data.requestLogs).toEqual(["model"]);
    expect(mocks.error).toHaveBeenCalledWith("Could not save column settings.");
  });

  it("also uses defaults when reloading a previously saved selection fails", async () => {
    const { result } = setup();
    await waitFor(() =>
      expect(result.current.data.requestLogs).toEqual(["time", "cost"]),
    );
    mocks.get.mockRejectedValue(new Error("Unavailable after restore"));
    await act(async () => {
      await result.current.refetch();
    });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toEqual({});
    expect(mocks.error).not.toHaveBeenCalled();
  });
});
