import { cn } from "@/lib/utils";

export function ReleaseUpdateBadge({
  selected = false,
  className,
}: {
  selected?: boolean;
  className?: string;
}) {
  return (
    <span
      aria-label="1 update available"
      className={cn(
        "inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full px-1 text-[11px] font-semibold leading-none tabular-nums",
        selected
          ? "bg-white text-blue-600"
          : "bg-emerald-500/10 text-emerald-800 dark:text-emerald-200",
        className,
      )}
    >
      1
    </span>
  );
}
