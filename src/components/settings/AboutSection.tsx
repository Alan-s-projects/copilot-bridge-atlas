import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { ArrowUpRight, CircleArrowUp, Github } from "lucide-react";
import { settingsApi } from "@/lib/api";
import { Button } from "@/components/ui/button";

export function AboutSection({
  availableReleaseVersion,
}: {
  availableReleaseVersion?: string | null;
}) {
  const [version, setVersion] = useState("");

  useEffect(() => {
    let mounted = true;
    void getVersion()
      .then((installed) => {
        if (mounted) setVersion(installed);
      })
      .catch(() => {});
    return () => {
      mounted = false;
    };
  }, []);

  return (
    <div className="space-y-6">
      {availableReleaseVersion && (
        <div
          role="status"
          className="flex flex-wrap items-center gap-4 rounded-xl border border-emerald-500/40 bg-emerald-500/10 p-5 text-emerald-950 dark:text-emerald-100"
        >
          <CircleArrowUp
            aria-hidden
            className="h-6 w-6 shrink-0 text-emerald-600 dark:text-emerald-400"
          />
          <div className="min-w-0 flex-1">
            <h2 className="font-semibold">New release available</h2>
            <p className="text-sm text-emerald-900/80 dark:text-emerald-100/80">
              Atlas {availableReleaseVersion} is available. View the release and
              download the Windows MSI.
            </p>
          </div>
          <Button
            variant="outline"
            className="border-emerald-600/30 bg-background/70 text-emerald-900 hover:bg-emerald-500/10 dark:text-emerald-100"
            onClick={() =>
              void settingsApi.openExternal(
                `https://github.com/Alan-s-projects/copilot-bridge-atlas/releases/tag/atlas-${availableReleaseVersion}`,
              )
            }
          >
            View release
            <ArrowUpRight aria-hidden className="ml-2 h-4 w-4" />
          </Button>
        </div>
      )}
      <section className="space-y-5 rounded-xl border bg-card p-6">
        <h2 className="text-xl font-semibold">
          Copilot Bridge Atlas {version}
        </h2>
        <p className="text-sm text-muted-foreground">
          A local OpenAI-compatible server connecting Codex to GitHub Copilot,
          with usage statistics and read-only configuration suggestions.
        </p>
        <div className="flex flex-wrap gap-3">
          <Button
            variant="outline"
            onClick={() =>
              void settingsApi.openExternal(
                "https://github.com/Alan-s-projects/copilot-bridge-atlas",
              )
            }
          >
            <Github className="mr-2 h-4 w-4" />
            GitHub
          </Button>
        </div>
      </section>
    </div>
  );
}
