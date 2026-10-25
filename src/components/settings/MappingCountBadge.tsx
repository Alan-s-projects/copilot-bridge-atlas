import { cn } from "@/lib/utils";

export function MappingCountBadge({
  count,
  selected = false,
  className,
}: {
  count?: number;
  selected?: boolean;
  className?: string;
}) {
  if (!count) return null;
  return (
    <span
      aria-label={`${count} Claude ${count === 1 ? "mapping needs" : "mappings need"} attention`}
      className={cn(
        "inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full px-1 text-[11px] font-semibold leading-none tabular-nums",
        selected
          ? "bg-white text-amber-800"
          : "bg-amber-500/10 text-amber-800 dark:text-amber-200",
        className,
      )}
    >
      {count}
    </span>
  );
}
