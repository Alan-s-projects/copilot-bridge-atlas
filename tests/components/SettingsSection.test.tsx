import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SettingsSection } from "@/components/ui/settings-section";
import { ProxyTabContent } from "@/components/settings/ProxyTabContent";

vi.mock("@/components/proxy/ProxyPanel", () => ({
  ProxyPanel: () => <div>API listeners</div>,
}));
vi.mock("@/components/settings/GlobalProxySettings", () => ({
  GlobalProxySettings: () => <input aria-label="Outbound proxy URL" />,
}));

describe("Settings section layout", () => {
  it("places the heading and live feedback outside the white bordered group", () => {
    render(
      <SettingsSection title="Theme" actions={<span role="status">Saved</span>}>
        <button>Light</button>
      </SettingsSection>,
    );
    const region = screen.getByRole("region", { name: "Theme" });
    const heading = within(region).getByRole("heading", { name: "Theme" });
    const body = screen.getByRole("button", { name: "Light" }).parentElement!;
    expect(body).toHaveClass("border", "bg-background", "rounded-xl");
    expect(body).not.toContainElement(heading);
    expect(
      within(heading.parentElement!).getByRole("status"),
    ).toHaveTextContent("Saved");
    expect(
      heading.compareDocumentPosition(body) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("shows outbound networking without an accordion or an extra click", () => {
    render(<ProxyTabContent />);
    expect(
      screen.getByRole("heading", { name: "Outbound network" }),
    ).toBeVisible();
    expect(
      screen.getByRole("textbox", { name: "Outbound proxy URL" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Outbound network" }),
    ).not.toBeInTheDocument();
  });
});
