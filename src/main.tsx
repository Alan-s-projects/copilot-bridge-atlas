import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
// Import internationalization configuration
import "./i18n";
import { QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "@/components/theme-provider";
import { queryClient } from "@/lib/query";
import { Toaster } from "@/components/ui/sonner";
import { FrontendErrorBoundary } from "./components/FrontendErrorBoundary";
import { installGlobalErrorHandlers } from "./lib/frontendLogger";
import { initializeWindowActivity } from "@/lib/windowActivity";

installGlobalErrorHandlers();

function bootstrap() {
  initializeWindowActivity();

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <FrontendErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <ThemeProvider
            defaultTheme="system"
            storageKey="copilot-bridge-atlas-theme"
          >
            <App />
            <Toaster />
          </ThemeProvider>
        </QueryClientProvider>
      </FrontendErrorBoundary>
    </React.StrictMode>,
  );
}

void bootstrap();
