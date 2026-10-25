import { SettingsSection } from "@/components/ui/settings-section";
import { useCallback, useRef, useState } from "react";
import {
  useClaudeMappings,
  claudeRoles,
  defaultClaude,
  type ClaudeRole,
} from "@/hooks/useClaudeMappings";
import { useUpdateGlobalProxyConfig } from "@/lib/query/proxy";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";

type SaveStatus = "idle" | "saving" | "error";

function MappingField({
  role,
  value,
  models,
  onStatusChange,
}: {
  role: ClaudeRole;
  value: string;
  models: string[];
  onStatusChange: (role: ClaudeRole, status: SaveStatus) => void;
}) {
  const { mutateAsync } = useUpdateGlobalProxyConfig({
    showSuccessToast: false,
  });
  const [draft, setDraft] = useState<string | null>(null);
  const [status, setStatus] = useState<SaveStatus>("idle");
  const revision = useRef(0);
  const selected = draft ?? value;
  const updateStatus = (next: SaveStatus) => {
    setStatus(next);
    onStatusChange(role, next);
  };
  const save = async (model: string) => {
    const version = ++revision.current;
    setDraft(model);
    updateStatus("saving");
    try {
      await mutateAsync((current) => ({
        ...current,
        claude: { ...(current.claude ?? defaultClaude), [role]: model },
      }));
      if (revision.current === version) {
        setDraft(null);
        updateStatus("idle");
      }
    } catch {
      if (revision.current === version) updateStatus("error");
    }
  };
  const unavailable = !!selected && !models.includes(selected);
  const hasFeedback = status === "error" || unavailable || !selected;
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
          aria-describedby={hasFeedback ? `claude-${role}-status` : undefined}
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
        {hasFeedback && (
          <div
            id={`claude-${role}-status`}
            role={status === "error" ? "alert" : undefined}
            className="text-xs text-muted-foreground"
          >
            {status === "error" ? (
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
            ) : (
              "Not configured"
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export function ClaudeModelMapping() {
  const { mappings, models, ready, error, retry } = useClaudeMappings();
  const [fieldStates, setFieldStates] = useState<
    Partial<Record<ClaudeRole, SaveStatus>>
  >({});
  const onStatusChange = useCallback((role: ClaudeRole, status: SaveStatus) => {
    setFieldStates((current) => ({ ...current, [role]: status }));
  }, []);
  const failed = Object.values(fieldStates).includes("error");
  const saving = Object.values(fieldStates).includes("saving");
  return (
    <SettingsSection
      title="Claude model mapping"
      actions={
        ready && !error ? (
          <span role="status" className="text-xs text-muted-foreground">
            {failed ? "Unsaved changes" : saving ? "Saving…" : "Saved"}
          </span>
        ) : undefined
      }
    >
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
              onStatusChange={onStatusChange}
            />
          ))}
        </>
      )}
    </SettingsSection>
  );
}
