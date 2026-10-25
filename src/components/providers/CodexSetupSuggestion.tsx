import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Copy, FolderSearch, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { copyText } from "@/lib/clipboard";
import { ConfigDiff, type ConfigDiffLine } from "./ConfigDiff";
import { ConfigDiffStatus } from "./ConfigDiffStatus";

const PREVIEW_PATH_KEY = "copilot-bridge-atlas-codex-preview-path";
const MODEL_LIST_KEY = "copilot-bridge-atlas-codex-model-list";
type ModelList = "atlas" | "codex";
const normalizePath = (path: string) => path.trim().replace(/^"(.*)"$/, "$1");
function readSavedModelList(): ModelList {
  try {
    return window.localStorage.getItem(MODEL_LIST_KEY) === "codex"
      ? "codex"
      : "atlas";
  } catch {
    return "atlas";
  }
}

function readSavedPath(): string | null {
  try {
    return (
      normalizePath(window.localStorage.getItem(PREVIEW_PATH_KEY) ?? "") || null
    );
  } catch {
    return null;
  }
}

interface SetupSuggestion {
  configPath: string;
  configExists: boolean;
  configured: boolean;
  copilotConfig: string;
  copilotLines: ConfigDiffLine[];
  openaiConfig: string;
  openaiLines: ConfigDiffLine[];
}

export function CodexSetupSuggestion({
  clientNavigation,
}: { clientNavigation?: import("react").ReactNode } = {}) {
  const [target, setTarget] = useState<"copilot" | "openai">("copilot");
  const [modelList, setModelList] = useState<ModelList>(readSavedModelList);
  const [selectedPath, setSelectedPath] = useState<string | null>(
    readSavedPath,
  );
  const [draftPath, setDraftPath] = useState<string | null>(null);
  const [loadedSource, setLoadedSource] = useState<{
    selectedPath: string | null;
    configPath: string;
  } | null>(null);
  const { data, error, isFetching, refetch } = useQuery({
    queryKey: ["codex-setup-suggestion", selectedPath, modelList],
    queryFn: () =>
      invoke<SetupSuggestion>("get_codex_setup_suggestion", {
        configPath: selectedPath,
        useCustomModelCatalog: modelList === "atlas",
      }),
    staleTime: 0,
    retry: false,
  });
  const previousSource =
    loadedSource?.selectedPath === selectedPath ? loadedSource : null;
  const source = data ?? previousSource;
  const loadedPath = source?.configPath ?? selectedPath ?? "";
  const pathText = draftPath ?? loadedPath;
  const isPathEdited = normalizePath(pathText) !== normalizePath(loadedPath);

  useEffect(() => {
    if (!data || error || isFetching) return;
    setLoadedSource({
      selectedPath,
      configPath: data.configPath,
    });
    if (!selectedPath || !data.configExists) return;
    try {
      window.localStorage.setItem(PREVIEW_PATH_KEY, data.configPath);
    } catch {
      // The selected file can still be previewed when local storage is unavailable.
    }
  }, [data, error, isFetching, selectedPath]);

  const choosePath = (path: string | null) => {
    const nextPath = path ? normalizePath(path) || null : null;
    setDraftPath(null);
    if (nextPath === null) {
      try {
        window.localStorage.removeItem(PREVIEW_PATH_KEY);
      } catch {
        // Auto-detection remains available without persisted preferences.
      }
    }
    if (nextPath === selectedPath) {
      if (!isFetching) void refetch();
    } else {
      setSelectedPath(nextPath);
    }
  };
  const refresh = () => {
    if (isPathEdited) {
      if (normalizePath(pathText)) choosePath(pathText);
    } else if (!isFetching) {
      setDraftPath(null);
      void refetch();
    }
  };
  const browse = async () => {
    try {
      const path = await open({
        directory: false,
        multiple: false,
        filters: [{ name: "TOML", extensions: ["toml"] }],
        defaultPath: normalizePath(pathText) || undefined,
      });
      if (typeof path === "string") choosePath(path);
    } catch (error) {
      toast.error(String(error));
    }
  };
  const lines = target === "copilot" ? data?.copilotLines : data?.openaiLines;
  const proposed =
    target === "copilot" ? data?.copilotConfig : data?.openaiConfig;
  const copy = async (text: string) => {
    try {
      await copyText(text);
      toast.success("Copied");
    } catch (error) {
      toast.error(String(error));
    }
  };
  return (
    <section className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden px-6 pb-4 pt-4">
      <div className="flex shrink-0 items-start justify-between gap-6">
        <div className="min-w-0 space-y-1">
          <h1 className="text-2xl font-bold tracking-tight">Connect</h1>
          <p className="text-sm text-muted-foreground">
            Preview only. Apply TOML file changes yourself manually.
          </p>
        </div>
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 gap-3 sm:gap-5">
        {clientNavigation}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3">
          <div className="grid shrink-0 grid-cols-[6rem_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-2.5">
            <form
              aria-label="Codex configuration file"
              className="col-span-3 grid grid-cols-subgrid items-center gap-3"
              onSubmit={(event) => {
                event.preventDefault();
                refresh();
              }}
            >
              <Label htmlFor="codex-toml-path" className="shrink-0 text-xs">
                Codex TOML file
              </Label>
              <Input
                id="codex-toml-path"
                className="min-w-0 font-mono text-sm"
                value={pathText}
                placeholder="Automatically detect config.toml"
                onChange={(event) => setDraftPath(event.target.value)}
              />
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  className="shrink-0"
                  title="Browse for TOML file"
                  aria-label="Browse for TOML file"
                  onClick={() => void browse()}
                >
                  <FolderSearch aria-hidden className="h-4 w-4" />
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  className="shrink-0"
                  onClick={() => choosePath(null)}
                >
                  Auto-detect
                </Button>
                <Button
                  type="submit"
                  variant="outline"
                  className="shrink-0"
                  disabled={
                    (isFetching && !isPathEdited) ||
                    (isPathEdited && !normalizePath(pathText))
                  }
                >
                  <RefreshCw
                    aria-hidden
                    className={`h-4 w-4 ${isFetching ? "animate-spin" : ""}`}
                  />
                  Refresh
                </Button>
              </div>
            </form>
            {isPathEdited && (
              <p
                role="status"
                className="col-span-3 text-xs text-muted-foreground"
              >
                Refresh to preview the file at this location.
              </p>
            )}
            {error && (
              <p
                role="alert"
                className="col-span-3 break-words text-sm text-destructive"
              >
                {String(error)}
              </p>
            )}
            {data && !error && !isPathEdited && !data.configExists && (
              <p
                role="status"
                className="col-span-3 text-xs text-muted-foreground"
              >
                No config.toml was found at this location. Browse for your file
                or review the proposed new file below. Atlas will not create it.
              </p>
            )}
            <div className="col-span-3 grid grid-cols-subgrid items-center gap-3">
              <Label htmlFor="codex-connection-target" className="text-xs">
                Connection
              </Label>
              <Select
                value={target}
                onValueChange={(value) => {
                  if (value === "copilot" || value === "openai")
                    setTarget(value);
                }}
              >
                <SelectTrigger id="codex-connection-target">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="copilot">Copilot Bridge</SelectItem>
                  <SelectItem value="openai">
                    Official OpenAI sign-in
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            {target === "copilot" && (
              <div className="col-span-3 grid grid-cols-subgrid items-center gap-3">
                <Label htmlFor="codex-model-list" className="text-xs">
                  Model list
                </Label>
                <Select
                  value={modelList}
                  onValueChange={(value) => {
                    if (value !== "atlas" && value !== "codex") return;
                    setModelList(value);
                    try {
                      window.localStorage.setItem(MODEL_LIST_KEY, value);
                    } catch {
                      // The current preview still works without persisted preferences.
                    }
                  }}
                >
                  <SelectTrigger id="codex-model-list">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="atlas">
                      Atlas customized list (Settings → Models)
                    </SelectItem>
                    <SelectItem value="codex">Codex default list</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            )}
          </div>
          {!data && isFetching && !isPathEdited && (
            <p role="status" className="text-xs text-muted-foreground">
              Loading suggestion…
            </p>
          )}
          {data && !error && !isPathEdited && (
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border">
              <div className="flex shrink-0 items-center justify-between gap-4 border-b bg-muted/40 p-2">
                <ConfigDiffStatus lines={lines ?? []} loading={isFetching} />
                <Button
                  size="sm"
                  className="shrink-0"
                  disabled={isFetching}
                  onClick={() => void copy(proposed ?? "")}
                >
                  <Copy aria-hidden className="mr-2 h-4 w-4" />
                  Copy proposed TOML
                </Button>
              </div>
              <ConfigDiff lines={lines ?? []} />
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
