import { invoke } from "@tauri-apps/api/core";
import { exit } from "@tauri-apps/plugin-process";
import { AlertTriangle, ExternalLink, FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";

interface DatabaseUpgradeProps {
  payload: {
    path?: string;
    error?: string;
    kind?: string;
    db_version?: number;
    supported_version?: number;
  };
}

export function DatabaseUpgrade({ payload }: DatabaseUpgradeProps) {
  const newer =
    typeof payload.db_version === "number" &&
    typeof payload.supported_version === "number" &&
    payload.db_version > payload.supported_version;
  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
      <div className="w-full max-w-lg space-y-5 rounded-2xl border bg-card p-7 shadow-xl">
        <AlertTriangle className="h-8 w-8 text-amber-500" />
        <h1 className="text-xl font-semibold">
          {newer
            ? "A newer Atlas version is required"
            : "Atlas 6 needs a clean data folder"}
        </h1>
        <p className="text-sm text-muted-foreground">
          This database uses schema {payload.db_version}; this app requires
          schema {payload.supported_version}. Your data has not been changed.{" "}
          {newer
            ? "Install a newer Atlas release to open it."
            : "Quit Atlas, move the folder containing this database aside, then launch Atlas 6 again to start fresh."}
        </p>
        {payload.path && (
          <pre className="overflow-x-auto text-xs">{payload.path}</pre>
        )}
        <div className="flex flex-wrap gap-2">
          {newer && (
            <Button onClick={() => void invoke("check_for_updates")}>
              <ExternalLink className="mr-2 h-4 w-4" />
              Atlas releases
            </Button>
          )}
          <Button
            variant="outline"
            onClick={() => void invoke("open_app_config_folder")}
          >
            <FolderOpen className="mr-2 h-4 w-4" />
            Open data folder
          </Button>
          <Button variant="ghost" onClick={() => void exit(0)}>
            Quit
          </Button>
        </div>
      </div>
    </div>
  );
}

export default DatabaseUpgrade;
