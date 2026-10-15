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

  // 非演示模式：原样返回 children（不注入 disabled / tabIndex / aria-disabled），
  // 否则普通构建下所有被包裹的控件都会被 cloneElement 打上 disabled: true 而点不了。
  // 两个 hook（useState、useT）都在早退之前，早退不会造成 hooks 顺序跳变。
  if (!demoModeEnabled) return <>{children}</>;

  const message = t("shared.demo.unavailable");

  const child = isValidElement(children)
    ? cloneElement(children as ReactElement<Record<string, unknown>>, {
        disabled: true,
        "aria-disabled": true,
        tabIndex: -1,
      })
    : children;

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
