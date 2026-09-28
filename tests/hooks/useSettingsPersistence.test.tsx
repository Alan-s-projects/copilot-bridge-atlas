import type { PropsWithChildren } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSettings } from "@/hooks/useSettings";
import type { Settings } from "@/types";

const api = vi.hoisted(() => ({
  get: vi.fn(),
  save: vi.fn(),
  setAutoLaunch: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  settingsApi: api,
}));
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

let stored: Settings;

beforeEach(() => {
  vi.resetAllMocks();
  stored = {
    launchOnStartup: false,
    backupRetainCount: 10,
    usageDashboardRefreshIntervalMs: 30000,
  };
  api.get.mockImplementation(async () => ({ ...stored }));
  api.save.mockImplementation(async (settings: Settings) => {
    stored = { ...settings };
    return true;
  });
  api.setAutoLaunch.mockResolvedValue(true);
});

function createWrapper() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity },
      mutations: { retry: false },
    },
  });
  client.setQueryData(["settings"], { ...stored });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("settings persistence", () => {
  it("keeps quick edits in order and retains the latest visible choices during refetch", async () => {
    const firstWrite = deferred();
    const secondWrite = deferred();
    api.save
      .mockImplementationOnce(async (settings: Settings) => {
        await firstWrite.promise;
        stored = { ...settings };
        return true;
      })
      .mockImplementationOnce(async (settings: Settings) => {
        await secondWrite.promise;
        stored = { ...settings };
        return true;
      });
    const { result } = renderHook(() => useSettings(), {
      wrapper: createWrapper(),
    });
    let first!: ReturnType<typeof result.current.autoSaveSettings>;
    let second!: ReturnType<typeof result.current.autoSaveSettings>;
    act(() => {
      first = result.current.autoSaveSettings({ backupRetainCount: 5 });
      second = result.current.autoSaveSettings({
        usageDashboardRefreshIntervalMs: 0,
      });
    });

    await waitFor(() => expect(api.save).toHaveBeenCalledTimes(1));
    expect(result.current.settings).toMatchObject({
      backupRetainCount: 5,
      usageDashboardRefreshIntervalMs: 0,
    });
    await act(async () => {
      firstWrite.resolve();
      await first;
    });
    await waitFor(() => expect(api.save).toHaveBeenCalledTimes(2));
    expect(api.save.mock.calls[1][0]).toMatchObject({
      backupRetainCount: 5,
      usageDashboardRefreshIntervalMs: 0,
    });
    expect(result.current.settings?.usageDashboardRefreshIntervalMs).toBe(0);
    expect(result.current.isSaving).toBe(true);

    await act(async () => {
      secondWrite.resolve();
      await second;
    });
    expect(stored).toMatchObject({
      backupRetainCount: 5,
      usageDashboardRefreshIntervalMs: 0,
    });
    expect(result.current.isSaving).toBe(false);
  });

  it("does not carry a failed edit into the next save", async () => {
    const failedWrite = deferred();
    api.save.mockImplementationOnce(async () => {
      await failedWrite.promise;
      return true;
    });
    const { result } = renderHook(() => useSettings(), {
      wrapper: createWrapper(),
    });
    let first!: Promise<unknown>;
    let second!: ReturnType<typeof result.current.autoSaveSettings>;
    act(() => {
      first = result.current
        .autoSaveSettings({ backupRetainCount: 5 })
        .catch((error: unknown) => error);
    });
    act(() => {
      second = result.current.autoSaveSettings({
        usageDashboardRefreshIntervalMs: 0,
      });
    });
    await waitFor(() => expect(api.save).toHaveBeenCalledTimes(1));
    await act(async () => {
      failedWrite.reject(new Error("disk full"));
      await first;
      await second;
    });
    expect(stored.backupRetainCount).toBe(10);
    expect(stored.usageDashboardRefreshIntervalMs).toBe(0);
    expect(result.current.settings).toMatchObject({
      backupRetainCount: 10,
      usageDashboardRefreshIntervalMs: 0,
    });
  });

  it("serializes saves across navigation to a new settings consumer", async () => {
    const firstWrite = deferred();
    api.save.mockImplementationOnce(async (settings: Settings) => {
      await firstWrite.promise;
      stored = { ...settings };
      return true;
    });
    const wrapper = createWrapper();
    const firstPage = renderHook(() => useSettings(), { wrapper });
    let first!: ReturnType<typeof firstPage.result.current.autoSaveSettings>;
    act(() => {
      first = firstPage.result.current.autoSaveSettings({
        backupRetainCount: 5,
      });
    });
    await waitFor(() => expect(api.save).toHaveBeenCalledTimes(1));
    firstPage.unmount();

    const secondPage = renderHook(() => useSettings(), { wrapper });
    let second!: ReturnType<typeof secondPage.result.current.autoSaveSettings>;
    act(() => {
      second = secondPage.result.current.autoSaveSettings({
        usageDashboardRefreshIntervalMs: 0,
      });
    });
    expect(api.save).toHaveBeenCalledTimes(1);
    await act(async () => {
      firstWrite.resolve();
      await first;
      await second;
    });
    expect(stored).toMatchObject({
      backupRetainCount: 5,
      usageDashboardRefreshIntervalMs: 0,
    });
  });

  it("applies both startup changes when toggled on and then off quickly", async () => {
    const firstWrite = deferred();
    api.save.mockImplementationOnce(async (settings: Settings) => {
      await firstWrite.promise;
      stored = { ...settings };
      return true;
    });
    const { result } = renderHook(() => useSettings(), {
      wrapper: createWrapper(),
    });
    let first!: ReturnType<typeof result.current.autoSaveSettings>;
    let second!: ReturnType<typeof result.current.autoSaveSettings>;
    act(() => {
      first = result.current.autoSaveSettings({ launchOnStartup: true });
      second = result.current.autoSaveSettings({ launchOnStartup: false });
    });
    await waitFor(() => expect(api.save).toHaveBeenCalledTimes(1));
    await act(async () => {
      firstWrite.resolve();
      await first;
      await second;
    });
    expect(api.setAutoLaunch.mock.calls).toEqual([[true], [false]]);
    expect(stored.launchOnStartup).toBe(false);
    expect(result.current.settings?.launchOnStartup).toBe(false);
  });

  it("keeps a rejected startup change out of the saved preferences", async () => {
    api.setAutoLaunch.mockRejectedValueOnce(new Error("permission denied"));
    const { result } = renderHook(() => useSettings(), {
      wrapper: createWrapper(),
    });
    await act(async () => {
      await expect(
        result.current.autoSaveSettings({ launchOnStartup: true }),
      ).rejects.toThrow("permission denied");
    });
    expect(api.save).not.toHaveBeenCalled();
    expect(stored.launchOnStartup).toBe(false);
    expect(result.current.settings?.launchOnStartup).toBe(false);
  });

  it("restores the system startup preference when saving its setting fails", async () => {
    api.save.mockRejectedValueOnce(new Error("disk full"));
    const { result } = renderHook(() => useSettings(), {
      wrapper: createWrapper(),
    });
    await act(async () => {
      await expect(
        result.current.autoSaveSettings({ launchOnStartup: true }),
      ).rejects.toThrow("disk full");
    });
    expect(api.setAutoLaunch.mock.calls).toEqual([[true], [false]]);
    expect(stored.launchOnStartup).toBe(false);
    expect(result.current.settings?.launchOnStartup).toBe(false);
  });
});
