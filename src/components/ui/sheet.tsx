import * as React from "react";
import * as SheetPrimitive from "@radix-ui/react-dialog";
import { XIcon } from "lucide-react";

import { cn } from "@/lib/utils";

/**
 * 侧滑抽屉（shadcn `Sheet`，基于 `@radix-ui/react-dialog`）。
 *
 * ## 为什么新增这个原语，而不是手写一个抽屉
 *
 * `AGENTS.md` 的 UI Component Policy 要求优先复用 / 新增 shadcn 原语；本仓已有
 * `@radix-ui/react-dialog`（`package.json`），而 `Sheet` 正是它的**侧滑封装**。
 * 手写一份等于把 Radix 已经处理好的焦点陷阱、`aria-modal`、Esc 关闭、滚动锁
 * 重新实现一遍，且必然与 `ui/dialog.tsx` 在细节上漂移。
 *
 * ## 为什么账号详情用抽屉而不是模态（PLAN §3.5）
 *
 * 用户是在**账号列表里**逐个查看详情，模态会盖住整张列表、打断「对照着看」的视线；
 * 抽屉从右侧推入，列表仍在原位可见。
 *
 * 风格与 `ui/dialog.tsx` 完全一致（`data-slot` / `cn()` / `tw-animate-css`），
 * 差异只有动画方向与停靠边。
 */

function Sheet(props: React.ComponentProps<typeof SheetPrimitive.Root>) {
  return <SheetPrimitive.Root data-slot="sheet" {...props} />;
}

function SheetTrigger(props: React.ComponentProps<typeof SheetPrimitive.Trigger>) {
  return <SheetPrimitive.Trigger data-slot="sheet-trigger" {...props} />;
}

function SheetClose(props: React.ComponentProps<typeof SheetPrimitive.Close>) {
  return <SheetPrimitive.Close data-slot="sheet-close" {...props} />;
}

function SheetPortal(props: React.ComponentProps<typeof SheetPrimitive.Portal>) {
  return <SheetPrimitive.Portal data-slot="sheet-portal" {...props} />;
}

function SheetOverlay({ className, ...props }: React.ComponentProps<typeof SheetPrimitive.Overlay>) {
  return (
    <SheetPrimitive.Overlay
      data-slot="sheet-overlay"
      /* ★ 退出动画刻意不加（与 `ui/dialog.tsx` 遮罩同一理由，issue #80）：
         Radix `Presence` 靠 `animationend` 决定卸载时机，该事件在部分引擎上不保证送达；
         一旦收不到，遮罩 / 内容节点永远卸不掉 ⇒ 弹窗关不掉、整窗像卡死。 */
      className={cn(
        "data-[state=open]:animate-in data-[state=open]:fade-in-0 fixed inset-0 z-50 bg-black/50",
        className,
      )}
      {...props}
    />
  );
}

/**
 * 抽屉内容。`side` 决定停靠边（本仓目前只用 `right`）。
 *
 * 默认宽度 `w-3/4` 且 `sm:max-w-sm`：账号详情的字段都是短行，
 * 再宽只会让视线在空行里来回跑。
 */
function SheetContent({
  className,
  children,
  side = "right",
  showCloseButton = true,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Content> & {
  side?: "top" | "right" | "bottom" | "left";
  showCloseButton?: boolean;
}) {
  return (
    <SheetPortal data-slot="sheet-portal">
      <SheetOverlay />
      <SheetPrimitive.Content
        data-slot="sheet-content"
        data-side={side}
        className={cn(
          /* ★ 逐值对齐原型 `.drawer`（`demo.css:481-489`）：
             底 `--card`（**不是** `--background`）、宽 `min(460px,94vw)`、
             阴影用三级 elevation 的最高档、滑入曲线 `cubic-bezier(.32,.72,0,1)`。
             ★ 退出动画（`animate-out` + `slide-out-to-*`）刻意不加 —— 与
             `ui/dialog.tsx` 同一理由（issue #80）：`Presence` 等 `animationend`
             才卸载，该事件在部分引擎上不送达 ⇒ 抽屉永久关不掉。进入动画保留。 */
          "bg-card fixed z-50 flex flex-col gap-0 shadow-overlay transition ease-in-out data-[state=open]:animate-in data-[state=open]:duration-500",
          side === "right" &&
            "data-[state=open]:slide-in-from-right inset-y-0 right-0 h-full w-3/4 border-l sm:max-w-[460px]",
          side === "left" &&
            "data-[state=open]:slide-in-from-left inset-y-0 left-0 h-full w-3/4 border-r sm:max-w-sm",
          side === "top" &&
            "data-[state=open]:slide-in-from-top inset-x-0 top-0 h-auto border-b",
          side === "bottom" &&
            "data-[state=open]:slide-in-from-bottom inset-x-0 bottom-0 h-auto border-t",
          className,
        )}
        {...props}
      >
        {children}
        {showCloseButton && (
          <SheetPrimitive.Close
            data-slot="sheet-close"
            className="ring-offset-background focus:ring-ring absolute top-4 right-4 rounded-xs opacity-70 transition-opacity hover:opacity-100 focus:ring-2 focus:ring-offset-2 focus:outline-hidden disabled:pointer-events-none"
          >
            <XIcon className="size-4" />
            <span className="sr-only">Close</span>
          </SheetPrimitive.Close>
        )}
      </SheetPrimitive.Content>
    </SheetPortal>
  );
}

function SheetHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-header"
      className={cn("flex flex-col gap-1.5 border-b border-border p-5", className)}
      {...props}
    />
  );
}

function SheetFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-footer"
      className={cn("mt-auto flex flex-col gap-2 border-t border-border p-5", className)}
      {...props}
    />
  );
}

function SheetTitle({ className, ...props }: React.ComponentProps<typeof SheetPrimitive.Title>) {
  return (
    <SheetPrimitive.Title
      data-slot="sheet-title"
      /* 原型 `.drawer-title`（`demo.css:488`）：`14px` / `600`（不是 shadcn 的 16px） */
      className={cn("text-foreground text-[14px] leading-tight font-semibold", className)}
      {...props}
    />
  );
}

function SheetDescription({
  className,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Description>) {
  return (
    <SheetPrimitive.Description
      data-slot="sheet-description"
      className={cn("text-muted-foreground text-xs", className)}
      {...props}
    />
  );
}

export {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetOverlay,
  SheetPortal,
  SheetTitle,
  SheetTrigger,
};
