import { CircleAlert, CircleCheck, RefreshCw } from "lucide-react";
import type { ConfigDiffLine } from "./ConfigDiff";

export function ConfigDiffStatus({
  lines,
  loading,
}: {
  lines: ConfigDiffLine[];
  loading: boolean;
}) {
  const added = lines.filter((line) => line.kind === "added").length;
  const removed = lines.filter((line) => line.kind === "removed").length;
  const count = (value: number) => `${value} ${value === 1 ? "line" : "lines"}`;
  let Icon = CircleCheck;
  let color = "text-emerald-600 dark:text-emerald-400";
  let text = "No changes needed in this file.";
  if (loading) {
    Icon = RefreshCw;
    color = "animate-spin text-muted-foreground";
    text = "Refreshing suggestion…";
  } else if (added && removed) {
    Icon = CircleAlert;
    color = "text-amber-600 dark:text-amber-400";
    text = `${count(added)} added, ${count(removed)} removed from the suggestion.`;
  } else if (removed) {
    Icon = CircleAlert;
    color = "text-amber-600 dark:text-amber-400";
    text = `${count(removed)} removed from the suggestion.`;
  } else if (added) {
    Icon = CircleAlert;
    color = "text-amber-600 dark:text-amber-400";
    text = `${count(added)} added to the suggestion.`;
  }

  return (
    <p
      role="status"
      className="flex min-w-0 items-center gap-2 px-1 text-xs leading-4"
    >
      <Icon aria-hidden className={`h-4 w-4 shrink-0 ${color}`} />
      <span>{text}</span>
    </p>
  );
}
