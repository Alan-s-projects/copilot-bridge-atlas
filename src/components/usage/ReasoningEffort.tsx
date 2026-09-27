import type { RequestLog } from "@/types/usage";

export function ReasoningEffortHeading() {
  return (
    <>
      Reasoning
      <span className="block text-[10px] font-normal text-muted-foreground">
        (requested/applied)
      </span>
    </>
  );
}

export function ReasoningEffortValue({
  log,
}: {
  log: Pick<RequestLog, "requestedReasoningEffort" | "appliedReasoningEffort">;
}) {
  const requested = log.requestedReasoningEffort || "\u2014";
  const applied = log.appliedReasoningEffort || "\u2014";
  const changed =
    log.requestedReasoningEffort &&
    log.appliedReasoningEffort &&
    log.requestedReasoningEffort !== log.appliedReasoningEffort;
  return (
    <span
      className="inline-block max-w-44 truncate align-middle whitespace-nowrap font-mono text-xs"
      title={`Requested: ${log.requestedReasoningEffort || "not specified or recorded"}; Applied: ${log.appliedReasoningEffort || "not sent or recorded"}`}
    >
      {requested}
      <span className="text-muted-foreground"> / </span>
      <span
        className={
          changed ? "font-medium text-amber-700 dark:text-amber-400" : undefined
        }
      >
        {applied}
      </span>
    </span>
  );
}
