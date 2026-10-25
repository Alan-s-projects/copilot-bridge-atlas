import { describe, expect, it } from "vitest";
import { getUsageRangePresetLabel, resolveUsageRange } from "@/lib/usageRange";

describe("calendar Usage presets", () => {
  it("starts the week on local Monday, including across a year boundary", () => {
    const sunday = new Date(2026, 9, 25, 18, 30);
    expect(
      resolveUsageRange({ preset: "this-week" }, sunday.getTime()),
    ).toEqual({
      startDate: new Date(2026, 9, 19).getTime() / 1000,
      endDate: sunday.getTime() / 1000,
    });
    expect(
      resolveUsageRange(
        { preset: "this-week" },
        new Date(2027, 0, 1, 12).getTime(),
      ).startDate,
    ).toBe(new Date(2026, 11, 28).getTime() / 1000);
    expect(
      resolveUsageRange(
        { preset: "this-week" },
        new Date(2026, 9, 26, 12).getTime(),
      ).startDate,
    ).toBe(new Date(2026, 9, 26).getTime() / 1000);
  });

  it("re-resolves calendar month boundaries for each refresh", () => {
    const selection = { preset: "this-month" as const };
    expect(
      resolveUsageRange(selection, new Date(2026, 9, 31, 23, 59).getTime())
        .startDate,
    ).toBe(new Date(2026, 9, 1).getTime() / 1000);
    expect(
      resolveUsageRange(selection, new Date(2026, 10, 1, 0, 1).getTime())
        .startDate,
    ).toBe(new Date(2026, 10, 1).getTime() / 1000);
  });

  it("uses local midnight across the spring daylight-saving transition", () => {
    const previous = process.env.TZ;
    try {
      process.env.TZ = "America/New_York";
      const now = new Date(2026, 2, 8, 20);
      const range = resolveUsageRange({ preset: "this-week" }, now.getTime());
      expect(range.startDate).toBe(new Date(2026, 2, 2).getTime() / 1000);
      expect(new Date(range.startDate * 1000).getHours()).toBe(0);
      expect(range.endDate).toBe(now.getTime() / 1000);
    } finally {
      if (previous === undefined) delete process.env.TZ;
      else process.env.TZ = previous;
    }
  });

  it("labels calendar presets distinctly from rolling-day ranges", () => {
    const t = (_key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? _key;
    expect(getUsageRangePresetLabel("this-week", t)).toBe("This week");
    expect(getUsageRangePresetLabel("this-month", t)).toBe("This month");
    expect(getUsageRangePresetLabel("7d", t)).toBe("7d");
  });
});
