import type { ReactNode } from "react";
import { act, cleanup, renderHook } from "@testing-library/react";
import {
  focusManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useBridgeOverview } from "@/hooks/useBridgeOverview";
import type { PaginatedLogs, UsageSummary } from "@/types/usage";

const mocks = vi.hoisted(() => ({
  summary: vi.fn(),
  logs: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("@/lib/api/usage", () => ({
  usageApi: {
    getUsageSummary: mocks.summary,
    getRequestLogs: mocks.logs,
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

const summary: UsageSummary = {
  totalRequests: 3,
  totalCost: "0.04",
  totalInputTokens: 100,
  totalOutputTokens: 50,
  totalCacheCreationTokens: 0,
  totalCacheReadTokens: 100,
  successRate: 100,
  avgLatencyMs: 1200,
  realTotalTokens: 250,
  cacheHitRate: 0.5,
};
const recent: PaginatedLogs = { data: [], total: 0, page: 0, pageSize: 5 };
const listeners = new Set<() => void>();
const activityListeners = new Set<(event: { payload: string }) => void>();
const unlisteners: Array<ReturnType<typeof vi.fn>> = [];
const clients: QueryClient[] = [];

function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  clients.push(client);
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { ...renderHook(() => useBridgeOverview(), { wrapper }), client };
}

async function advance(milliseconds: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
  });
}

function emitUsage(count = 1) {
  act(() => {
    for (let index = 0; index < count; index++) {
      for (const listener of listeners) listener();
    }
  });
}

async function setFocused(focused: boolean) {
  act(() => focusManager.setFocused(focused));
  await advance(1);
}

describe("low-CPU bridge overview", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 8, 26, 12));
    focusManager.setFocused(true);
    listeners.clear();
    activityListeners.clear();
    unlisteners.length = 0;
    mocks.summary.mockReset().mockResolvedValue(summary);
    mocks.logs.mockReset().mockResolvedValue(recent);
    mocks.listen
      .mockReset()
      .mockImplementation((event: string, callback: () => void) => {
        const target =
          event === "proxy-request-activity" ? activityListeners : listeners;
        target.add(callback);
        const off = vi.fn(() => {
          target.delete(callback);
        });
        unlisteners.push(off);
        return Promise.resolve(off);
      });
  });

  afterEach(() => {
    cleanup();
    for (const client of clients.splice(0)) client.clear();
    focusManager.setFocused(undefined);
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it("reads today's summary and the latest five once, without idle SQL polling", async () => {
    const now = Date.now();
    const { result } = mount();
    await advance(1);
    expect(result.current.data).toEqual({ summary, recent });
    expect(mocks.summary).toHaveBeenCalledWith(
      Math.floor(new Date(2026, 8, 26).getTime() / 1000),
      Math.floor(now / 1000),
      "codex",
    );
    expect(mocks.logs).toHaveBeenCalledWith({ appType: "codex" }, 0, 5);
    expect(mocks.listen).toHaveBeenCalledWith(
      "usage-log-recorded",
      expect.any(Function),
    );

    await advance(2 * 60 * 60 * 1000);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.logs).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("coalesces a burst into one refresh after five seconds", async () => {
    mount();
    await advance(1);
    emitUsage(20);
    await advance(4999);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    emitUsage(20);
    await advance(1);
    expect(mocks.summary).toHaveBeenCalledTimes(2);
    expect(mocks.logs).toHaveBeenCalledTimes(2);

    await advance(30_000);
    expect(mocks.summary).toHaveBeenCalledTimes(2);
    emitUsage();
    await advance(5000);
    expect(mocks.summary).toHaveBeenCalledTimes(3);
    expect(mocks.logs).toHaveBeenCalledTimes(3);
  });

  it("does no work while inactive and resumes with a fresh snapshot", async () => {
    focusManager.setFocused(false);
    const { result } = mount();
    await advance(60_000);
    expect(mocks.summary).not.toHaveBeenCalled();
    expect(mocks.logs).not.toHaveBeenCalled();
    expect(mocks.listen).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);

    await setFocused(true);
    expect(result.current.data?.summary.totalRequests).toBe(3);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    emitUsage();
    await advance(1000);
    await setFocused(false);
    expect(unlisteners[0]).toHaveBeenCalledTimes(1);
    expect(listeners.size).toBe(0);
    await advance(60_000);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.logs).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);

    mocks.summary.mockResolvedValue({ ...summary, totalRequests: 4 });
    await setFocused(true);
    expect(result.current.data?.summary.totalRequests).toBe(4);
    expect(mocks.summary).toHaveBeenCalledTimes(2);
    expect(mocks.logs).toHaveBeenCalledTimes(2);
    expect(mocks.listen).toHaveBeenCalledTimes(4);
    await advance(30_000);
    expect(mocks.summary).toHaveBeenCalledTimes(2);
  });

  it("refreshes active metadata without SQL polling and coalesces completion events", async () => {
    const { client } = mount();
    await advance(1);
    const invalidations = vi.spyOn(client, "invalidateQueries");
    act(() => {
      for (const listener of activityListeners)
        listener({ payload: "started" });
    });
    await advance(1);
    expect(invalidations).toHaveBeenCalledWith(
      { queryKey: ["proxyStatus"] },
      { cancelRefetch: false },
    );
    expect(mocks.logs).toHaveBeenCalledTimes(1);
    act(() => {
      for (let index = 0; index < 20; index++) {
        for (const listener of activityListeners)
          listener({ payload: "finished" });
      }
    });
    await advance(99);
    expect(mocks.logs).toHaveBeenCalledTimes(1);
    await advance(1);
    expect(mocks.logs).toHaveBeenCalledTimes(2);
    await advance(30_000);
    expect(mocks.logs).toHaveBeenCalledTimes(2);
  });

  it("unsubscribes and cancels both the pending refresh and midnight timer on unmount", async () => {
    const { unmount } = mount();
    await advance(1);
    emitUsage();
    expect(vi.getTimerCount()).toBe(2);
    unmount();
    await advance(1);
    expect(unlisteners[0]).toHaveBeenCalledTimes(1);
    expect(listeners.size).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    await advance(26 * 60 * 60 * 1000);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.logs).toHaveBeenCalledTimes(1);
  });

  it("ignores late callbacks and disposes a listener whose registration completes after unmount", async () => {
    let callback!: () => void;
    let register!: (off: () => void) => void;
    const off = vi.fn();
    mocks.listen.mockImplementationOnce(
      (_event: string, listener: () => void) => {
        callback = listener;
        return new Promise<() => void>((resolve) => {
          register = resolve;
        });
      },
    );
    const { unmount } = mount();
    await advance(1);
    unmount();
    await act(async () => {
      callback();
      register(off);
    });
    await advance(1);
    expect(off).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
    await advance(5000);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.logs).toHaveBeenCalledTimes(1);
  });

  it("rolls today's range over once at midnight without starting an idle polling loop", async () => {
    vi.setSystemTime(new Date(2026, 8, 26, 23, 59, 59, 500));
    const nextMidnight = new Date(2026, 8, 27).getTime();
    mount();
    await advance(1);
    await advance(nextMidnight + 50 - Date.now());
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    await advance(5000);
    expect(mocks.summary).toHaveBeenCalledTimes(2);
    expect(mocks.summary).toHaveBeenLastCalledWith(
      Math.floor(nextMidnight / 1000),
      Math.floor(Date.now() / 1000),
      "codex",
    );
    expect(mocks.logs).toHaveBeenCalledTimes(2);
    await advance(60 * 60 * 1000);
    expect(mocks.summary).toHaveBeenCalledTimes(2);
    expect(mocks.logs).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(1);
  });
});
