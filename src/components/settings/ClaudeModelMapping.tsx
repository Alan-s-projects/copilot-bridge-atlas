import { useRef, useState } from "react";
import {
  useClaudeMappings,
  claudeRoles,
  defaultClaude,
  type ClaudeRole,
} from "@/hooks/useClaudeMappings";
import { useUpdateGlobalProxyConfig } from "@/lib/query/proxy";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";

function MappingField({
  role,
  value,
  models,
}: {
  role: ClaudeRole;
  value: string;
  models: string[];
}) {
  const { mutateAsync } = useUpdateGlobalProxyConfig({
    showSuccessToast: false,
  });
  const [draft, setDraft] = useState<string | null>(null);
  const [status, setStatus] = useState<"idle" | "saving" | "error">("idle");
  const revision = useRef(0);
  const selected = draft ?? value;
  const save = async (model: string) => {
    const version = ++revision.current;
    setDraft(model);
    setStatus("saving");
    try {
      await mutateAsync((current) => ({
        ...current,
        claude: { ...(current.claude ?? defaultClaude), [role]: model },
      }));
      if (revision.current === version) {
        setDraft(null);
        setStatus("idle");
      }
    } catch {
      if (revision.current === version) setStatus("error");
    }
  };
  const unavailable = !!selected && !models.includes(selected);
  return (
    <div className="grid grid-cols-[80px_minmax(0,1fr)] items-start gap-4">
      <Label htmlFor={`claude-${role}`} className="pt-3 capitalize">
        {role}
      </Label>
      <div className="min-w-0 space-y-1.5">
        <select
          id={`claude-${role}`}
          value={selected}
          className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
          aria-describedby={`claude-${role}-status`}
          onChange={(event) => void save(event.target.value)}
        >
          <option value="">Select a model…</option>
          {unavailable && (
            <option value={selected} disabled>
              {selected} (disabled or unavailable)
            </option>
          )}
          {models.map((model) => (
            <option key={model} value={model}>
              {model}
            </option>
          ))}
        </select>
        <div
          id={`claude-${role}-status`}
          role={status === "error" ? "alert" : "status"}
          className="text-xs text-muted-foreground"
        >
          {status === "saving" ? (
            "Saving…"
          ) : status === "error" ? (
            <span className="text-destructive">
              Could not save.{" "}
              <Button
                type="button"
                variant="link"
                size="sm"
                onClick={() => void save(selected)}
              >
                Retry
              </Button>
            </span>
          ) : unavailable ? (
            "Enable this model below or choose another model."
          ) : !selected ? (
            "Not configured"
          ) : (
            "Saved"
          )}
        </div>
      </div>
    </div>
  );
}

export function ClaudeModelMapping() {
  const { mappings, models, ready, error, retry } = useClaudeMappings();
  return (
    <section
      aria-label="Claude model mapping"
      className="space-y-4 rounded-xl border bg-card p-6"
    >
      <h3 className="font-semibold">Claude model mapping</h3>
      <p className="text-sm text-muted-foreground">
        Choose an enabled Copilot model for each Claude role. Unconfigured roles
        cannot send requests.
      </p>
      {error ? (
        <div role="alert">
          Could not load mappings.{" "}
          <Button type="button" variant="outline" onClick={() => void retry()}>
            Retry
          </Button>
        </div>
      ) : !ready ? (
        <p role="status">Loading mappings…</p>
      ) : (
        <>
          {models.length === 0 && (
            <p className="text-sm text-muted-foreground">
              Select a GitHub account and enable models below to configure
              Claude.
            </p>
          )}
          {claudeRoles.map((role) => (
            <MappingField
              key={role}
              role={role}
              value={mappings[role]}
              models={models}
            />
          ))}
        </>
      )}
    </section>
  );
}
