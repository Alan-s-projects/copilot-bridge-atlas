import { createContext, useContext, type ReactNode } from "react";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsPage } from "@/components/settings/SettingsPage";

const mocks = vi.hoisted(() => ({
  autoSaveSettings: vi.fn(),
  t: vi.fn(
    (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
  ),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: mocks.t }),
}));
vi.mock("@/hooks/useSettings", () => ({
  useSettings: () => ({
    settings: { launchOnStartup: false },
    isLoading: false,
    isSaving: false,
    autoSaveSettings: mocks.autoSaveSettings,
  }),
}));
vi.mock("@/components/settings/ThemeSettings", () => ({
  ThemeSettings: () => <div>Theme settings</div>,
}));
vi.mock("@/components/settings/WindowSettings", () => ({
  WindowSettings: ({
    onChange,
  }: {
    onChange: (updates: { launchOnStartup: boolean }) => Promise<boolean>;
  }) => (
    <button onClick={() => onChange({ launchOnStartup: true })}>
      Toggle startup
    </button>
  ),
}));
vi.mock("@/components/settings/ProxyTabContent", () => ({
  ProxyTabContent: () => <div>Proxy settings</div>,
}));
vi.mock("@/components/settings/BackupListSection", () => ({
  BackupListSection: ({
    onSettingsChange,
  }: {
    onSettingsChange: (updates: { backupRetainCount: number }) => void;
  }) => (
    <div>
      <button onClick={() => onSettingsChange({ backupRetainCount: 7 })}>
        Change backup retention
      </button>
    </div>
  ),
}));
vi.mock("@/components/settings/AuthCenterPanel", () => ({
  AuthCenterPanel: () => <div>Auth settings</div>,
}));
vi.mock("@/components/settings/CopilotSettingsPanel", () => ({
  CopilotSettingsPanel: () => <div>Copilot settings</div>,
}));
vi.mock("@/components/settings/AboutSection", () => ({
  AboutSection: () => <div>About</div>,
}));
const TabsContext = createContext<{
  value: string;
  onValueChange?: (value: string) => void;
}>({ value: "general" });

vi.mock("@/components/ui/tabs", () => ({
  Tabs: ({
    value,
    onValueChange,
    children,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    children: ReactNode;
  }) => (
    <TabsContext.Provider value={{ value, onValueChange }}>
      {children}
    </TabsContext.Provider>
  ),
  TabsList: ({ children }: { children: ReactNode }) => <nav>{children}</nav>,
  TabsTrigger: ({
    value,
    children,
  }: {
    value: string;
    children: ReactNode;
  }) => {
    const tabs = useContext(TabsContext);
    return (
      <button type="button" onClick={() => tabs.onValueChange?.(value)}>
        {children}
      </button>
    );
  },
  TabsContent: ({
    value,
    children,
  }: {
    value: string;
    children: ReactNode;
  }) => {
    const tabs = useContext(TabsContext);
    return tabs.value === value ? <div>{children}</div> : null;
  },
}));

beforeEach(() => {
  mocks.autoSaveSettings.mockReset().mockResolvedValue(true);
  mocks.t.mockImplementation(
    (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
  );
});

describe("SettingsPage", () => {
  it("scrolls only content and resets its position when the section changes", () => {
    render(<SettingsPage />);
    const content = screen.getByRole("region", { name: "Settings content" });
    const sidebar = screen.getByRole("navigation");
    expect(content).toHaveClass(
      "relative",
      "overflow-y-auto",
      "overscroll-contain",
      "flex-1",
    );
    expect(content).not.toContainElement(sidebar);
    expect(content).not.toContainElement(
      screen.getByRole("heading", { name: "Settings" }),
    );
    expect(content).toHaveAttribute("tabindex", "0");
    content.scrollTop = 300;
    fireEvent.click(screen.getByRole("button", { name: "Accounts" }));
    expect(content.scrollTop).toBe(0);
    expect(content).toHaveTextContent("Auth settings");
    expect(
      screen.queryByRole("button", { name: "Auth" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Models" }));
    expect(content).toHaveTextContent("Copilot settings");
  });

  it("autosaves general preferences when they change", async () => {
    render(<SettingsPage />);

    fireEvent.click(screen.getByRole("button", { name: "Toggle startup" }));

    await waitFor(() =>
      expect(mocks.autoSaveSettings).toHaveBeenCalledWith({
        launchOnStartup: true,
      }),
    );
  });

  it("places backup and diagnostic logs below About without an Advanced tab", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sidebar = screen.getByRole("navigation");
    expect(
      within(sidebar)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual([
      "settings.tabGeneral",
      "settings.tabProxy",
      "Accounts",
      "Models",
      "About",
    ]);
    expect(
      screen.queryByRole("button", { name: "settings.tabAdvanced" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "About" }));

    const about = within(
      screen.getByRole("region", { name: "Settings content" }),
    ).getByText("About");
    const backupTrigger = screen.getByRole("button", {
      name: /Backup & Restore/,
    });
    expect(
      about.compareDocumentPosition(backupTrigger) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    await user.click(backupTrigger);
    expect(
      await screen.findByRole("heading", { name: "Backup & Restore" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: /Application Diagnostic Logs/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "common.save" }),
    ).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Change backup retention" }),
    );
    expect(mocks.autoSaveSettings).toHaveBeenCalledWith({
      backupRetainCount: 7,
    });
  });
});
