import type { InputTokenUsage } from "@/types/usage";
import { fmtInt, formatTokensShort, formatReadCacheHitRate } from "./format";

export function InputUsageHeading({ inline = false }: { inline?: boolean }) {
  return (
    <>
      Input{inline ? " " : null}
      <span
        className={`${inline ? "" : "block "}text-[10px] font-normal text-muted-foreground`}
      >
        (fresh/cached/hit)
      </span>
    </>
  );
}

export function InputUsageValue({
  fresh,
  cached,
  hit,
  compact = false,
  compactDecimals,
}: {
  fresh: number;
  cached: number;
  hit: string;
  compact?: boolean;
  compactDecimals?: 1 | 2;
}) {
  const format = compact
    ? (value: number) => formatTokensShort(value, compactDecimals)
    : (value: number) => fmtInt(value, "en-US");
  return (
    <span
      className="whitespace-nowrap tabular-nums"
      title={`Fresh Input: ${fmtInt(fresh, "en-US")}; Cached Input: ${fmtInt(cached, "en-US")}; Read Cache Hit Rate: ${hit}`}
    >
      {format(fresh)}
      <span className="text-muted-foreground"> / </span>
      {format(cached)}
      <span className="text-muted-foreground"> / </span>
      <span
        className={
          hit === "--"
            ? "text-muted-foreground"
            : "text-emerald-700 dark:text-emerald-400"
        }
      >
        {hit}
      </span>
    </span>
  );
}

export function RequestInputValue({
  log,
  compact = false,
}: {
  log: InputTokenUsage;
  compact?: boolean;
}) {
  return (
    <span className="text-xs">
      <InputUsageValue
        fresh={log.freshInputTokens}
        cached={log.cacheReadTokens}
        hit={formatReadCacheHitRate(log)}
        compact={compact}
      />
    </span>
  );
}
