import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AboutSection } from "@/components/settings/AboutSection";

const mocks = vi.hoisted(() => ({
  getVersion: vi.fn(),
  getAvailableReleaseVersion: vi.fn(),
  openExternal: vi.fn(),
  checkUpdates: vi.fn(),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: mocks.getVersion,
}));
vi.mock("@/lib/api", () => ({
  settingsApi: {
    getAvailableReleaseVersion: mocks.getAvailableReleaseVersion,
    openExternal: mocks.openExternal,
    checkUpdates: mocks.checkUpdates,
  },
}));

beforeEach(() => {
  mocks.getVersion.mockReset().mockResolvedValue("5.0.5");
  mocks.getAvailableReleaseVersion.mockReset().mockResolvedValue(null);
  mocks.openExternal.mockReset().mockResolvedValue(undefined);
  mocks.checkUpdates.mockReset().mockResolvedValue(undefined);
});

describe("AboutSection release reminder", () => {
  it("checks when opened and shows a newer release above the About card", async () => {
    mocks.getAvailableReleaseVersion.mockResolvedValue("5.0.6");
    render(<AboutSection />);

    const reminder = await screen.findByRole("status");
    expect(mocks.getAvailableReleaseVersion).toHaveBeenCalledOnce();
    expect(reminder).toHaveTextContent("New release available");
    expect(reminder).toHaveTextContent("Atlas 5.0.6");
    const about = await screen.findByRole("heading", {
      name: "Copilot Bridge Atlas 5.0.5",
    });
    expect(
      reminder.compareDocumentPosition(about) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "View release" }));
    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://github.com/Alan-s-projects/copilot-bridge-atlas/releases/tag/atlas-5.0.6",
    );
  });

  it("keeps About usable without a reminder when current or offline", async () => {
    const current = render(<AboutSection />);
    await screen.findByRole("heading", {
      name: "Copilot Bridge Atlas 5.0.5",
    });
    await waitFor(() =>
      expect(mocks.getAvailableReleaseVersion).toHaveBeenCalledOnce(),
    );
    expect(screen.queryByRole("status")).not.toBeInTheDocument();

    current.unmount();
    mocks.getAvailableReleaseVersion.mockRejectedValue(new Error("offline"));
    render(<AboutSection />);
    await screen.findByRole("heading", {
      name: "Copilot Bridge Atlas 5.0.5",
    });
    expect(mocks.getAvailableReleaseVersion).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("keeps the GitHub link without the old release and MSI buttons", async () => {
    render(<AboutSection />);
    await screen.findByRole("heading", {
      name: "Copilot Bridge Atlas 5.0.5",
    });
    expect(screen.getByRole("button", { name: "GitHub" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Release notes" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Download MSI" }),
    ).not.toBeInTheDocument();
    expect(mocks.checkUpdates).not.toHaveBeenCalled();
  });
});
