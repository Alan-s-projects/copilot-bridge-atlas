import { Github } from "lucide-react";
import { CopilotAuthSection } from "@/components/providers/forms/CopilotAuthSection";
export function AuthCenterPanel() {
  return (
    <section className="rounded-xl border border-border/60 bg-card/60 p-6">
      <div className="mb-4 flex items-center gap-3">
        <Github className="h-5 w-5" />
        <div>
          <h4 className="font-medium">GitHub Copilot</h4>
        </div>
      </div>
      <CopilotAuthSection />
    </section>
  );
}
