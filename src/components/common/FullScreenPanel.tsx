import React from "react";
import { createPortal } from "react-dom";
import { motion, AnimatePresence, useReducedMotion } from "framer-motion";
import { ArrowLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { isTextEditableTarget } from "@/utils/domUtils";

interface FullScreenPanelProps {
  isOpen: boolean;
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  footer?: React.ReactNode;
  /** Entry/exit motion. Nested navigation panels can opt into a horizontal transition. */
  motionPreset?: "fade" | "slide-from-right";
}

const HEADER_HEIGHT = 64; // px - match App.tsx

let bodyScrollLockCount = 0;
let bodyOverflowBeforeFirstLock: string | null = null;

const lockBodyScroll = () => {
  if (bodyScrollLockCount === 0) {
    bodyOverflowBeforeFirstLock = document.body.style.overflow;
    document.body.style.overflow = "hidden";
  }
  bodyScrollLockCount += 1;
};

const unlockBodyScroll = () => {
  bodyScrollLockCount = Math.max(0, bodyScrollLockCount - 1);
  if (bodyScrollLockCount === 0) {
    document.body.style.overflow = bodyOverflowBeforeFirstLock ?? "";
    bodyOverflowBeforeFirstLock = null;
  }
};

/**
 * Reusable full-screen panel component
 * Handles portal rendering, header with back button, and footer
 * Uses solid theme colors without transparency
 */
export const FullScreenPanel: React.FC<FullScreenPanelProps> = ({
  isOpen,
  title,
  onClose,
  children,
  footer,
  motionPreset = "fade",
}) => {
  const { t } = useTranslation();
  const prefersReducedMotion = useReducedMotion();
  const shouldSlideFromRight =
    motionPreset === "slide-from-right" && !prefersReducedMotion;

  React.useEffect(() => {
    if (!isOpen) return;

    lockBodyScroll();
    return unlockBodyScroll;
  }, [isOpen]);

  // ESC key to close the panel
  const onCloseRef = React.useRef(onClose);

  React.useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  React.useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        // If subcomponents (such as Radix's Select/Dialog/Dropdown) have already consumed ESC, do not close the entire panel.
        if (event.defaultPrevented) {
          return;
        }

        if (isTextEditableTarget(event.target)) {
          return; // Let the input box handle ESC by itself (such as clearing, losing focus, etc.)
        }

        event.stopPropagation(); // Prevent events from bubbling up to the window to avoid triggering App.tsx's global listener
        onCloseRef.current();
      }
    };

    // Use bubbling phase listening to let sub-components (such as Radix UI) prioritize ESC
    window.addEventListener("keydown", handleKeyDown, false);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, false);
    };
  }, [isOpen]);

  return createPortal(
    <AnimatePresence>
      {isOpen && (
        <motion.div
          initial={
            prefersReducedMotion
              ? false
              : shouldSlideFromRight
                ? { x: "100%" }
                : { opacity: 0 }
          }
          animate={shouldSlideFromRight ? { x: 0 } : { opacity: 1 }}
          exit={shouldSlideFromRight ? { x: "100%" } : { opacity: 0 }}
          transition={
            shouldSlideFromRight
              ? { duration: 0.26, ease: [0.22, 1, 0.36, 1] }
              : { duration: prefersReducedMotion ? 0 : 0.2 }
          }
          className="fixed inset-0 z-[60] flex flex-col"
          style={{ backgroundColor: "hsl(var(--background))" }}
        >
          {/* Header - match App.tsx */}
          <div
            className="flex-shrink-0 flex items-center"
            style={
              {
                backgroundColor: "hsl(var(--background))",
                height: HEADER_HEIGHT,
              } as React.CSSProperties
            }
          >
            <div className="px-6 w-full flex items-center gap-4">
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={onClose}
                aria-label={t("common.back")}
                className="rounded-lg select-none"
                style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
              >
                <ArrowLeft className="h-4 w-4" />
              </Button>
              <h2 className="text-lg font-semibold text-foreground select-none">
                {title}
              </h2>
            </div>
          </div>

          {/* Content */}
          <div className="flex-1 overflow-y-auto scroll-overlay">
            <div className="px-6 py-6 space-y-6 w-full">{children}</div>
          </div>

          {/* Footer */}
          {footer && (
            <div
              className="flex-shrink-0 py-4 border-t border-border-default"
              style={{ backgroundColor: "hsl(var(--background))" }}
            >
              <div className="px-6 flex items-center justify-end gap-3">
                {footer}
              </div>
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
};
