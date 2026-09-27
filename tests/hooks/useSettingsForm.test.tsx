import { renderHook, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useSettingsForm } from "@/hooks/useSettingsForm";

const useSettingsQueryMock = vi.fn();
vi.mock("@/lib/query", () => ({
  useSettingsQuery: (...args: unknown[]) => useSettingsQueryMock(...args),
}));

beforeEach(() => useSettingsQueryMock.mockReset());

describe("application settings form", () => {
  it("loads the app preferences and preserves unrelated values when editing", () => {
    useSettingsQueryMock.mockReturnValue({
      data: { backupRetainCount: 8 },
      isLoading: false,
    });
    const { result } = renderHook(() => useSettingsForm());
    act(() => result.current.updateSettings({ launchOnStartup: true }));
    expect(result.current.settings).toEqual({
      backupRetainCount: 8,
      launchOnStartup: true,
    });
  });
});
