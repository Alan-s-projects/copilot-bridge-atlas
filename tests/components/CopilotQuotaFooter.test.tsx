import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CopilotQuotaFooter from "@/components/CopilotQuotaFooter";

const quota = vi.hoisted(() => vi.fn());
vi.mock("@/lib/query/copilot", () => ({ useCopilotQuota: quota }));

beforeEach(() =>
  quota.mockReturnValue({
    data: { utilization: 6, plan: "enterprise", resetDate: "2026-10-01" },
    error: null,
    isFetching: false,
  }),
);

describe("Copilot quota ring", () => {
  it("shows the percentage ring with the plan and reset date", () => {
    render(<CopilotQuotaFooter />);
    expect(
      screen.getByRole("img", { name: "Copilot premium requests: 6.0% used" }),
    ).toBeVisible();
    expect(screen.getByText("enterprise")).toBeVisible();
    expect(screen.getByText("Resets 10/1/2026")).toBeVisible();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it.each([
    [-5, "0.0"],
    [105, "100.0"],
  ])("bounds out-of-range utilization %s", (value, expected) => {
    quota.mockReturnValue({
      data: { utilization: value, plan: "Pro" },
      error: null,
      isFetching: false,
    });
    render(<CopilotQuotaFooter />);
    expect(
      screen.getByRole("img", {
        name: `Copilot premium requests: ${expected}% used`,
      }),
    ).toBeVisible();
  });

  it("does not show a misleading zero-percent ring when quota cannot load", () => {
    quota.mockReturnValue({
      data: undefined,
      error: new Error("Quota unavailable"),
      isFetching: false,
    });
    render(<CopilotQuotaFooter />);
    expect(screen.getByRole("alert")).toHaveTextContent("Quota unavailable");
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });
});
