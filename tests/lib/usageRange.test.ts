import { describe, expect, it } from "vitest";
import { resolveUsageRange } from "@/lib/usageRange";
import { usageKeys } from "@/lib/query/usage";

describe("Saved rolling ranges and trend grouping keys", () => {
  it("counts calendar days rather than subtracting fixed 24-hour durations", () => {
    for (const [preset, days] of [
      ["7d", 7],
      ["14d", 14],
      ["30d", 30],
    ] as const) {
      const now = new Date(2026, 2, 9, 0, 30);
      const expected = new Date(2026, 2, 9 - days + 1);
      const range = resolveUsageRange({ preset }, now.getTime());
      expect(range.startDate).toBe(expected.getTime() / 1000);
      expect(range.endDate).toBe(now.getTime() / 1000);
    }
  });

  it("keeps model, interval and unit in the trend query key", () => {
    const key = (interval: number, unit: "day" | "hour", model = "a") =>
      usageKeys.trends("7d", undefined, undefined, { model }, false, {
        interval,
        unit,
      });
    expect(key(1, "day")).not.toEqual(key(3, "day"));
    expect(key(1, "day")).not.toEqual(key(1, "hour"));
    expect(key(1, "day")).not.toEqual(key(1, "day", "b"));
  });

  it("keeps seven local dates across the spring DST change", () => {
    const previous = process.env.TZ;
    try {
      process.env.TZ = "America/New_York";
      const now = new Date(2026, 2, 9, 0, 30);
      const range = resolveUsageRange({ preset: "7d" }, now.getTime());
      expect(new Date(range.startDate * 1000).getDate()).toBe(3);
      expect(new Date(range.startDate * 1000).getHours()).toBe(0);
    } finally {
      if (previous === undefined) delete process.env.TZ;
      else process.env.TZ = previous;
    }
  });
});
