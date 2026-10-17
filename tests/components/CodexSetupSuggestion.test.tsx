import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClientProvider } from "@tanstack/react-query";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { CodexSetupSuggestion } from "@/components/providers/CodexSetupSuggestion";
import type { ConfigDiffLine } from "@/components/providers/ConfigDiff";
import { createTestQueryClient } from "../utils/testQueryClient";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  copy: vi.fn(),
  open: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("@/lib/clipboard", () => ({ copyText: mocks.copy }));

const previewPathKey = "copilot-bridge-atlas-codex-preview-path";
const modelListKey = "copilot-bridge-atlas-codex-model-list";
const profilePath = "D:/Codex profiles/work config.toml";
const oldUrl = 'base_url = "http://127.0.0.1:4142/v1"';
const newUrl = 'base_url = "http://127.0.0.1:15722/v1"';
const currentLines = [
  'model_provider = "copilot-bridge"',
  "model_auto_compact_token_limit = 900000",
  "",
  "[model_providers.copilot-bridge]",
  'name = "Copilot Bridge"',
  oldUrl,
  'env_key = "COPILOT_BRIDGE_TOKEN"',
  'wire_api = "responses"',
];
const currentConfig = currentLines.join("\n") + "\n";
const context = (
  oldLineNumber: number,
  newLineNumber = oldLineNumber,
): ConfigDiffLine => ({
  kind: "context",
  oldLineNumber,
  newLineNumber,
  text: currentLines[oldLineNumber - 1] + "\n",
});
const removed = (oldLineNumber: number): ConfigDiffLine => ({
  kind: "removed",
  oldLineNumber,
  newLineNumber: null,
  text: currentLines[oldLineNumber - 1] + "\n",
});
const added = (newLineNumber: number, text: string): ConfigDiffLine => ({
  kind: "added",
  oldLineNumber: null,
  newLineNumber,
  text: text + "\n",
});
const copilotLines = [
  context(1),
  context(2),
  context(3),
  context(4),
  removed(5),
  removed(6),
  removed(7),
  added(5, 'name = "GitHub Copilot"'),
  added(6, newUrl),
  context(8, 7),
  added(8, "requires_openai_auth = false"),
  added(9, 'experimental_bearer_token = "PROXY_MANAGED"'),
  added(10, "supports_websockets = false"),
];
const openaiLines = [
  removed(1),
  added(1, 'model_provider = "openai"'),
  context(2),
  added(3, 'forced_login_method = "chatgpt"'),
  ...[3, 4, 5, 6, 7, 8].map((number) => context(number, number + 1)),
];
const proposedText = (lines: ConfigDiffLine[]) =>
  lines
    .filter((line) => line.kind !== "removed")
    .map((line) => line.text)
    .join("");
const preview = {
  configPath: "C:/Users/test/.codex/config.toml",
  configExists: true,
  configured: false,
  copilotConfig: proposedText(copilotLines),
  copilotLines,
  openaiConfig: proposedText(openaiLines),
  openaiLines,
};
const catalogLine =
  'model_catalog_json = "/Users/test/.copilot-bridge-atlas/copilot-model-catalog.json"\n';
const catalogCurrentLines = [catalogLine.trim(), ...currentLines];
const atlasListPreview = {
  ...preview,
  copilotConfig: catalogLine + currentConfig,
  copilotLines: catalogCurrentLines.map((text, index) => ({
    kind: "context" as const,
    oldLineNumber: index + 1,
    newLineNumber: index + 1,
    text: text + "\n",
  })),
};
const codexListPreview = {
  ...atlasListPreview,
  copilotConfig: currentConfig.replace(
    "model_auto_compact_token_limit = 900000",
    "model_context_window = 872000\nmodel_auto_compact_token_limit = 780000",
  ),
  copilotLines: [
    {
      kind: "removed" as const,
      oldLineNumber: 1,
      newLineNumber: null,
      text: catalogLine,
    },
    {
      kind: "context" as const,
      oldLineNumber: 2,
      newLineNumber: 1,
      text: currentLines[0] + "\n",
    },
    { ...removed(2), oldLineNumber: 3 },
    added(2, "model_context_window = 872000"),
    added(3, "model_auto_compact_token_limit = 780000"),
    ...currentLines.slice(2).map((text, index) => ({
      kind: "context" as const,
      oldLineNumber: index + 4,
      newLineNumber: index + 4,
      text: text + "\n",
    })),
  ],
};
const profilePreview = {
  ...preview,
  configPath: profilePath,
  copilotConfig: preview.copilotConfig.replace("900000", "150000"),
  copilotLines: copilotLines.map((line) => ({
    ...line,
    text: line.text.replace("900000", "150000"),
  })),
  openaiConfig: preview.openaiConfig.replace("900000", "150000"),
  openaiLines: openaiLines.map((line) => ({
    ...line,
    text: line.text.replace("900000", "150000"),
  })),
};
const renderPanel = () =>
  render(
    <QueryClientProvider client={createTestQueryClient()}>
      <CodexSetupSuggestion />
    </QueryClientProvider>,
  );
const comparison = () =>
  screen.getByRole("region", { name: "Configuration diff" });
const pathInput = () =>
  screen.getByRole("textbox", { name: "Codex TOML file" });
const refresh = () => screen.getByRole("button", { name: "Refresh" });
const copyProposal = () =>
  screen.getByRole("button", { name: "Copy proposed TOML" });
const connectionControl = () =>
  screen.getByRole("combobox", { name: "Connection" });
const modelListControl = () =>
  screen.getByRole("combobox", { name: "Model list" });
async function selectOption(control: HTMLElement, option: string) {
  fireEvent.keyDown(control, { key: "ArrowDown" });
  fireEvent.click(await screen.findByRole("option", { name: option }));
}
const expectNoPreview = () => {
  expect(
    screen.queryByRole("region", { name: "Configuration diff" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Copy proposed TOML" }),
  ).not.toBeInTheDocument();
};
const rowFor = (text: string) => {
  const row = within(comparison()).getByText(text).closest("tr");
  expect(row).not.toBeNull();
  return within(row!);
};

describe("read-only Codex connection suggestions", () => {
  const scrollIntoViewDescriptor = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "scrollIntoView",
  );
  beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
  });
  afterAll(() => {
    if (scrollIntoViewDescriptor) {
      Object.defineProperty(
        HTMLElement.prototype,
        "scrollIntoView",
        scrollIntoViewDescriptor,
      );
    } else {
      Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    }
  });
  beforeEach(() => {
    localStorage.removeItem(previewPathKey);
    localStorage.removeItem(modelListKey);
    mocks.invoke.mockReset().mockResolvedValue(preview);
    mocks.copy.mockReset().mockResolvedValue(undefined);
    mocks.open.mockReset().mockResolvedValue(null);
  });

  afterEach(() => {
    localStorage.removeItem(previewPathKey);
    localStorage.removeItem(modelListKey);
  });

  it("places the file controls before the separate connection destination row", async () => {
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    const fileControls = screen.getByRole("form", {
      name: "Codex configuration file",
    });
    const destination = connectionControl();
    expect(within(fileControls).getByRole("textbox")).toBe(pathInput());
    expect(fileControls).not.toContainElement(destination);
    expect(
      fileControls.compareDocumentPosition(destination) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("lists the default choice first in each dropdown", async () => {
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    fireEvent.keyDown(connectionControl(), { key: "ArrowDown" });
    await screen.findByRole("option", { name: "Copilot Bridge" });
    expect(
      screen.getAllByRole("option").map((option) => option.textContent),
    ).toEqual(["Copilot Bridge", "Official OpenAI sign-in"]);
  });

  it("defaults to the Atlas list and remembers the preview-only choice", async () => {
    mocks.invoke.mockImplementation((_command, { useCustomModelCatalog }) =>
      Promise.resolve(
        useCustomModelCatalog ? atlasListPreview : codexListPreview,
      ),
    );
    const first = renderPanel();
    await screen.findByText("No changes needed in this file.");
    expect(modelListControl()).toHaveTextContent(
      "Atlas customized list (Settings → Models)",
    );
    fireEvent.keyDown(modelListControl(), { key: "ArrowDown" });
    await screen.findByRole("option", { name: "Codex default list" });
    expect(
      screen.getAllByRole("option").map((option) => option.textContent),
    ).toEqual([
      "Atlas customized list (Settings → Models)",
      "Codex default list",
    ]);
    fireEvent.click(screen.getByRole("option", { name: "Codex default list" }));
    await screen.findByText(
      "2 lines added, 2 lines removed from the suggestion.",
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_codex_setup_suggestion",
      { configPath: null, useCustomModelCatalog: false },
    );
    const cells = rowFor(catalogLine.trim()).getAllByRole("cell");
    expect(cells[0]).toHaveTextContent(catalogLine.trim());
    expect(cells[1]).toHaveTextContent(/^$/);
    for (const line of [
      "model_context_window = 872000",
      "model_auto_compact_token_limit = 780000",
    ]) {
      const cells = rowFor(line).getAllByRole("cell");
      expect(cells[1]).toHaveTextContent(line);
    }
    expect(copyProposal().parentElement).toContainElement(
      screen.getByRole("status"),
    );
    fireEvent.click(copyProposal());
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenLastCalledWith(
        codexListPreview.copilotConfig,
      ),
    );
    expect(localStorage.getItem(modelListKey)).toBe("codex");
    first.unmount();
    renderPanel();
    await screen.findByText(
      "2 lines added, 2 lines removed from the suggestion.",
    );
    expect(modelListControl()).toHaveTextContent("Codex default list");
    await selectOption(connectionControl(), "Official OpenAI sign-in");
    expect(
      screen.queryByRole("combobox", { name: "Model list" }),
    ).not.toBeInTheDocument();
    fireEvent.click(copyProposal());
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenLastCalledWith(preview.openaiConfig),
    );
    expect(
      mocks.invoke.mock.calls.every(
        ([command]) => command === "get_codex_setup_suggestion",
      ),
    ).toBe(true);
  });

  it("uses the Atlas default for an unrecognized saved preference", async () => {
    localStorage.setItem(modelListKey, "invalid");
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    expect(modelListControl()).toHaveTextContent(
      "Atlas customized list (Settings → Models)",
    );
    expect(mocks.invoke).toHaveBeenCalledWith("get_codex_setup_suggestion", {
      configPath: null,
      useCustomModelCatalog: true,
    });
  });

  it("does not show or copy a late result from the previously selected model list", async () => {
    let resolve!: (value: typeof codexListPreview) => void;
    mocks.invoke.mockImplementation((_command, { useCustomModelCatalog }) =>
      useCustomModelCatalog
        ? Promise.resolve(atlasListPreview)
        : new Promise<typeof codexListPreview>((done) => {
            resolve = done;
          }),
    );
    renderPanel();
    await screen.findByText("No changes needed in this file.");
    await selectOption(modelListControl(), "Codex default list");
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenLastCalledWith(
        "get_codex_setup_suggestion",
        { configPath: null, useCustomModelCatalog: false },
      ),
    );
    expectNoPreview();
    expect(screen.getByRole("status")).toHaveTextContent("Loading suggestion");
    await selectOption(
      modelListControl(),
      "Atlas customized list (Settings → Models)",
    );
    await waitFor(() => expect(copyProposal()).toBeEnabled());
    await act(async () => resolve(codexListPreview));
    expect(modelListControl()).toHaveTextContent("Atlas customized list");
    expect(screen.getByRole("status")).toHaveTextContent(
      "No changes needed in this file.",
    );
    fireEvent.click(copyProposal());
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenLastCalledWith(
        atlasListPreview.copilotConfig,
      ),
    );
  });

  it("shows current TOML on the left and proposed TOML on the right in a keyboard-scrollable pane", async () => {
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    comparison().focus();
    expect(comparison()).toHaveFocus();
    expect(connectionControl()).toHaveTextContent("Copilot Bridge");
    const headings = within(comparison()).getAllByRole("columnheader");
    expect(headings[0]).toHaveTextContent("Current TOML");
    expect(headings[1]).toHaveTextContent("Proposed TOML");
    const changedCells = rowFor(oldUrl).getAllByRole("cell");
    expect(changedCells[0]).toHaveTextContent(oldUrl);
    expect(changedCells[1]).toHaveTextContent(newUrl);
    const preferences = within(comparison()).getAllByText(currentLines[1]);
    expect(preferences).toHaveLength(2);
    expect(preferences[0].closest("tr")).toBe(preferences[1].closest("tr"));
    expect(
      within(preferences[0].closest("tr")!).getAllByText("2"),
    ).toHaveLength(2);
  });

  it("aligns unequal change blocks and preserves line numbers after deletions", async () => {
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    const deletedCells = rowFor(currentLines[6]).getAllByRole("cell");
    expect(deletedCells[0]).toHaveTextContent(currentLines[6]);
    expect(deletedCells[1]).toHaveTextContent(/^$/);
    const addedCells = rowFor("requires_openai_auth = false").getAllByRole(
      "cell",
    );
    expect(addedCells[0]).toHaveTextContent(/^$/);
    expect(addedCells[1]).toHaveTextContent("requires_openai_auth = false");
    const unchangedWire = within(comparison()).getAllByText(currentLines[7]);
    expect(unchangedWire).toHaveLength(2);
    const wireCells = within(unchangedWire[0].closest("tr")!).getAllByRole(
      "cell",
    );
    expect(within(wireCells[0]).getByText("8")).toBeVisible();
    expect(within(wireCells[1]).getByText("7")).toBeVisible();
  });

  it("shows the complete current file even when no changes are needed", async () => {
    mocks.invoke.mockResolvedValue({
      ...preview,
      copilotConfig: currentConfig,
      copilotLines: currentLines.map((_, index) => context(index + 1)),
    });
    renderPanel();
    expect(
      await screen.findByText("No changes needed in this file."),
    ).toBeVisible();
    expect(screen.getAllByText("No changes needed in this file.")).toHaveLength(
      1,
    );
    expect(screen.getByRole("status")).toContainElement(
      screen.getByText("No changes needed in this file."),
    );
    expect(screen.getByRole("status").querySelector("svg")).toHaveAttribute(
      "aria-hidden",
      "true",
    );
    expect(comparison()).not.toContainElement(screen.getByRole("status"));
    for (const line of currentLines.filter(Boolean)) {
      expect(within(comparison()).getAllByText(line)).toHaveLength(2);
    }
    expect(
      screen.getByRole("button", { name: "Copy proposed TOML" }),
    ).toBeEnabled();
  });

  it("explains a missing auto-detected file and previews additions without creating it", async () => {
    const text = 'model_provider = "copilot-bridge-atlas"';
    mocks.invoke.mockResolvedValue({
      ...preview,
      configExists: false,
      copilotConfig: text,
      copilotLines: [{ ...added(1, text), text }],
    });
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    expect(
      screen.getByText(/No config.toml was found at this location/),
    ).toBeVisible();
    expect(pathInput()).toHaveValue(preview.configPath);
    const cells = rowFor(text).getAllByRole("cell");
    expect(cells[0]).toHaveTextContent(/^$/);
    expect(cells[1]).toHaveTextContent(text);
    expect(
      within(cells[1]).getByText(/No newline at end of file/),
    ).toBeVisible();
    expect(mocks.invoke.mock.calls).toEqual([
      [
        "get_codex_setup_suggestion",
        { configPath: null, useCustomModelCatalog: true },
      ],
    ]);
    expect(localStorage.getItem(previewPathKey)).toBeNull();
  });

  it("copies either proposal and refreshes using read-only commands", async () => {
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    expect(pathInput()).toHaveValue(preview.configPath);
    expect(
      screen.getByText(
        "Preview only. Apply TOML file changes yourself manually.",
      ),
    ).toBeVisible();
    expect(copyProposal().parentElement).toContainElement(
      screen.getByText("5 lines added, 3 lines removed from the suggestion."),
    );
    expect(screen.getAllByRole("textbox")).toEqual([pathInput()]);
    expect(
      screen.queryByRole("button", { name: /^(Apply|Save|Repair)/ }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Copy proposed TOML" }));
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenLastCalledWith(preview.copilotConfig),
    );
    await selectOption(connectionControl(), "Official OpenAI sign-in");
    const cells = rowFor('model_provider = "openai"').getAllByRole("cell");
    expect(cells[0]).toHaveTextContent(currentLines[0]);
    expect(cells[1]).toHaveTextContent('model_provider = "openai"');
    expect(comparison()).toHaveTextContent('forced_login_method = "chatgpt"');
    expect(comparison()).not.toHaveTextContent(newUrl);
    fireEvent.click(screen.getByRole("button", { name: "Copy proposed TOML" }));
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenLastCalledWith(preview.openaiConfig),
    );
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(2));
    expect(
      mocks.invoke.mock.calls.every(
        ([command]) => command === "get_codex_setup_suggestion",
      ),
    ).toBe(true);
    expect(mocks.invoke).toHaveBeenCalledWith("get_codex_setup_suggestion", {
      configPath: null,
      useCustomModelCatalog: true,
    });
  });

  it("reports invalid configuration without offering a write or repair action", async () => {
    mocks.invoke.mockRejectedValue(new Error("Codex config.toml is invalid"));
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent("invalid");
    expect(
      screen.queryByRole("button", { name: "Copy proposed TOML" }),
    ).not.toBeInTheDocument();
    expect(mocks.copy).not.toHaveBeenCalled();
    expect(mocks.invoke).toHaveBeenCalledWith("get_codex_setup_suggestion", {
      configPath: null,
      useCustomModelCatalog: true,
    });
    expect(pathInput()).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "Browse for TOML file" }),
    ).toBeEnabled();
  });

  it.each(["Refresh", "Enter"] as const)(
    "loads a typed path with %s without copying the previous file or changing the comparison target",
    async (action) => {
      let resolve!: (value: typeof profilePreview) => void;
      mocks.invoke.mockResolvedValueOnce(preview).mockImplementationOnce(
        () =>
          new Promise<typeof profilePreview>((done) => {
            resolve = done;
          }),
      );
      renderPanel();
      await screen.findByRole("region", { name: "Configuration diff" });
      await selectOption(connectionControl(), "Official OpenAI sign-in");
      fireEvent.change(pathInput(), {
        target: { value: ` "${profilePath}" ` },
      });
      expectNoPreview();
      expect(mocks.invoke).toHaveBeenCalledTimes(1);
      expect(localStorage.getItem(previewPathKey)).toBeNull();

      if (action === "Enter") {
        await userEvent.type(pathInput(), "{enter}");
      } else {
        fireEvent.click(refresh());
      }
      await waitFor(() =>
        expect(mocks.invoke).toHaveBeenLastCalledWith(
          "get_codex_setup_suggestion",
          { configPath: profilePath, useCustomModelCatalog: true },
        ),
      );
      expectNoPreview();
      expect(localStorage.getItem(previewPathKey)).toBeNull();
      await act(async () => resolve(profilePreview));

      await screen.findByRole("region", { name: "Configuration diff" });
      expect(pathInput()).toHaveValue(profilePath);
      expect(comparison()).toHaveTextContent(
        "model_auto_compact_token_limit = 150000",
      );
      expect(comparison()).not.toHaveTextContent("900000");
      expect(connectionControl()).toHaveTextContent("Official OpenAI sign-in");
      fireEvent.click(copyProposal());
      await waitFor(() =>
        expect(mocks.copy).toHaveBeenLastCalledWith(
          profilePreview.openaiConfig,
        ),
      );
      expect(mocks.invoke.mock.calls).toEqual([
        [
          "get_codex_setup_suggestion",
          { configPath: null, useCustomModelCatalog: true },
        ],
        [
          "get_codex_setup_suggestion",
          { configPath: profilePath, useCustomModelCatalog: true },
        ],
      ]);
      await waitFor(() =>
        expect(localStorage.getItem(previewPathKey)).toBe(profilePath),
      );
    },
  );

  it("keeps the current file when browsing is cancelled and loads a chosen TOML immediately", async () => {
    mocks.open.mockResolvedValueOnce(null).mockResolvedValueOnce(profilePath);
    mocks.invoke
      .mockResolvedValueOnce(preview)
      .mockResolvedValueOnce(profilePreview);
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    const browse = screen.getByRole("button", {
      name: "Browse for TOML file",
    });
    await userEvent.click(browse);
    expect(mocks.open).toHaveBeenCalledWith({
      directory: false,
      multiple: false,
      filters: [{ name: "TOML", extensions: ["toml"] }],
      defaultPath: preview.configPath,
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(pathInput()).toHaveValue(preview.configPath);
    expect(copyProposal()).toBeEnabled();

    await userEvent.click(browse);
    await waitFor(() => expect(pathInput()).toHaveValue(profilePath));
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_codex_setup_suggestion",
      { configPath: profilePath, useCustomModelCatalog: true },
    );
    await waitFor(() => expect(copyProposal()).toBeEnabled());
    fireEvent.click(copyProposal());
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenCalledWith(profilePreview.copilotConfig),
    );
  });

  it("reopens the remembered file and clears that choice with Auto-detect", async () => {
    localStorage.setItem(previewPathKey, profilePath);
    mocks.invoke.mockImplementation((_command, { configPath }) =>
      Promise.resolve(configPath === profilePath ? profilePreview : preview),
    );
    const first = renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    expect(pathInput()).toHaveValue(profilePath);
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_codex_setup_suggestion",
      { configPath: profilePath, useCustomModelCatalog: true },
    );
    first.unmount();

    const reopened = renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    expect(pathInput()).toHaveValue(profilePath);
    fireEvent.click(screen.getByRole("button", { name: "Auto-detect" }));
    await waitFor(() => expect(pathInput()).toHaveValue(preview.configPath));
    expect(localStorage.getItem(previewPathKey)).toBeNull();
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_codex_setup_suggestion",
      { configPath: null, useCustomModelCatalog: true },
    );
    reopened.unmount();

    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    expect(pathInput()).toHaveValue(preview.configPath);
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_codex_setup_suggestion",
      { configPath: null, useCustomModelCatalog: true },
    );
  });

  it.each(["The selected file does not exist", "The selected TOML is invalid"])(
    "keeps the last successful path and allows recovery when %s",
    async (message) => {
      localStorage.setItem(previewPathKey, profilePath);
      mocks.invoke
        .mockResolvedValueOnce(profilePreview)
        .mockRejectedValueOnce(new Error(message))
        .mockResolvedValue(preview);
      renderPanel();
      await screen.findByRole("region", { name: "Configuration diff" });
      const badPath = "D:/Codex profiles/bad.toml";
      fireEvent.change(pathInput(), { target: { value: badPath } });
      fireEvent.click(refresh());
      expect(await screen.findByRole("alert")).toHaveTextContent(message);
      expect(pathInput()).toHaveValue(badPath);
      expectNoPreview();
      expect(localStorage.getItem(previewPathKey)).toBe(profilePath);
      expect(refresh()).toBeEnabled();
      expect(
        screen.getByRole("button", { name: "Browse for TOML file" }),
      ).toBeEnabled();
      fireEvent.click(screen.getByRole("button", { name: "Auto-detect" }));
      await screen.findByRole("region", { name: "Configuration diff" });
      expect(pathInput()).toHaveValue(preview.configPath);
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(localStorage.getItem(previewPathKey)).toBeNull();
      expect(mocks.copy).not.toHaveBeenCalled();
    },
  );

  it("disables copying while refreshing and hides cached content after a failed refresh", async () => {
    localStorage.setItem(previewPathKey, profilePath);
    let reject!: (error: Error) => void;
    mocks.invoke
      .mockResolvedValueOnce(profilePreview)
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, fail) => {
            reject = fail;
          }),
      )
      .mockResolvedValue(profilePreview);
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    fireEvent.click(refresh());
    await waitFor(() => expect(copyProposal()).toBeDisabled());
    expect(screen.getByRole("status")).toHaveTextContent(
      "Refreshing suggestion",
    );
    await act(async () => reject(new Error("Cannot read this TOML file")));
    expect(await screen.findByRole("alert")).toHaveTextContent("Cannot read");
    expectNoPreview();
    expect(localStorage.getItem(previewPathKey)).toBe(profilePath);

    fireEvent.click(refresh());
    await screen.findByRole("region", { name: "Configuration diff" });
    expect(copyProposal()).toBeEnabled();
    expect(mocks.invoke.mock.calls).toEqual(
      Array.from({ length: 3 }, () => [
        "get_codex_setup_suggestion",
        { configPath: profilePath, useCustomModelCatalog: true },
      ]),
    );
  });

  it("does not replace the auto-detected preview with a late result for another file", async () => {
    let resolve!: (value: typeof profilePreview) => void;
    mocks.invoke.mockImplementation((_command, { configPath }) =>
      configPath === profilePath
        ? new Promise<typeof profilePreview>((done) => {
            resolve = done;
          })
        : Promise.resolve(preview),
    );
    mocks.open.mockResolvedValue(profilePath);
    renderPanel();
    await screen.findByRole("region", { name: "Configuration diff" });
    await userEvent.click(
      screen.getByRole("button", { name: "Browse for TOML file" }),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenLastCalledWith(
        "get_codex_setup_suggestion",
        { configPath: profilePath, useCustomModelCatalog: true },
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Auto-detect" }));
    await waitFor(() => expect(pathInput()).toHaveValue(preview.configPath));
    await act(async () => resolve(profilePreview));
    expect(pathInput()).toHaveValue(preview.configPath);
    expect(comparison()).toHaveTextContent(
      "model_auto_compact_token_limit = 900000",
    );
    expect(localStorage.getItem(previewPathKey)).toBeNull();
    await waitFor(() => expect(copyProposal()).toBeEnabled());
    fireEvent.click(copyProposal());
    await waitFor(() =>
      expect(mocks.copy).toHaveBeenCalledWith(preview.copilotConfig),
    );
  });
});
