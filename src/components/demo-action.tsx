import { cloneElement, isValidElement, useState, type ReactElement, type ReactNode } from "react";
import { toast } from "sonner";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { demoModeEnabled } from "@/lib/demo-mode";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

interface DemoActionProps {
  children: ReactElement;
  className?: string;
}

/**
 * Keeps operational controls visible in the public demo while preventing activation.
 * The wrapper remains hoverable and keyboard focusable because native disabled controls
 * cannot trigger a tooltip by themselves.
 */
export function DemoAction({ children, className }: DemoActionProps) {
  const [open, setOpen] = useState(false);
  const t = useT();
  const message = t("shared.demo.unavailable");

  const child = isValidElement(children)
    ? cloneElement(children as ReactElement<Record<string, unknown>>, {
        disabled: true,
        "aria-disabled": true,
        tabIndex: -1,
      })
    : children;

  if (!demoModeEnabled) return <>{child}</>;

  function explain() {
    setOpen(true);
    toast.info(message);
  }

  return (
    <Tooltip open={open} onOpenChange={setOpen}>
      <TooltipTrigger asChild>
        <span
          className={cn("inline-flex max-w-full", className)}
          role="button"
          tabIndex={0}
          aria-disabled="true"
          aria-label={message}
          onPointerDownCapture={(event) => {
            event.preventDefault();
            explain();
          }}
          onKeyDown={(event) => {
            if (event.key !== "Enter" && event.key !== " ") return;
            event.preventDefault();
            explain();
          }}
        >
          {child as ReactNode}
        </span>
      </TooltipTrigger>
      <TooltipContent>{message}</TooltipContent>
    </Tooltip>
  );
}
