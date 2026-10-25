import { useId, type ReactNode } from "react";
import { cn } from "@/lib/utils";

export const settingsSidebarClassName =
  "flex min-h-0 w-full shrink-0 flex-row items-stretch justify-start gap-1 self-stretch overflow-auto rounded-xl border border-border bg-background p-2.5 sm:w-[204px] sm:flex-col";

export const settingsSidebarItemClassName =
  "min-w-0 shrink-0 justify-start gap-2.5 rounded-md px-3 py-2.5 text-left font-normal text-foreground transition-none data-[state=inactive]:opacity-100 data-[state=active]:bg-blue-600 data-[state=active]:text-white data-[state=active]:shadow-none dark:data-[state=active]:bg-blue-600 aria-[current=page]:bg-blue-600 aria-[current=page]:text-white aria-[current=page]:hover:bg-blue-600";

export function SettingsSection({
  title,
  actions,
  children,
  className,
  bodyClassName,
}: {
  title: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className={cn("space-y-3", className)}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 id={headingId} className="text-sm font-semibold">
          {title}
        </h2>
        {actions}
      </div>
      <div
        className={cn(
          "min-w-0 space-y-4 rounded-xl border border-border bg-background p-4 sm:px-6 sm:py-5",
          bodyClassName,
        )}
      >
        {children}
      </div>
    </section>
  );
}
