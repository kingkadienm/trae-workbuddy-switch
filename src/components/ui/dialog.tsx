import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { XIcon } from "lucide-react";

import { cn } from "@/lib/utils";

function Dialog({ ...props }: React.ComponentProps<typeof DialogPrimitive.Root>) {
  return <DialogPrimitive.Root data-slot="dialog" {...props} />;
}

function DialogTrigger({ ...props }: React.ComponentProps<typeof DialogPrimitive.Trigger>) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

function DialogPortal({
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Portal>) {
  return <DialogPrimitive.Portal data-slot="dialog-portal" {...props} />;
}

function DialogClose({ ...props }: React.ComponentProps<typeof DialogPrimitive.Close>) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

/**
 * 弹窗遮罩（`fixed inset-0` 的压暗层）。
 *
 * ★ **刻意不加退出动画**（`data-[state=closed]:animate-out`）：Radix 的 `Presence`
 * 靠 `animationend` 决定何时卸载节点，而这个事件在不同引擎 / 合成路径上并不保证触发
 * （WebKit 上就出现过不触发）。遮罩一旦卸载不掉，它会以 `z-50` + `pointer-events:auto`
 * **持续盖住整个窗口** ⇒ 用户看到的就是「界面卡死、任何地方都点不动」（issue #80）。
 * 没有退出动画时 `getAnimationName()` 读到 `none`，`Presence` 会**立即卸载**，
 * 「关弹窗 ⇒ 恢复可点」从此不依赖任何动画事件。
 *
 * 代价仅是一层 150ms 的淡出不播 —— 遮罩是纯装饰，肉眼不可感知；弹窗**内容**的
 * 进入 / 退出动画照旧（那才是用户能看见的部分）。
 */
function DialogOverlay({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Overlay>) {
  return (
    <DialogPrimitive.Overlay
      data-slot="dialog-overlay"
      className={cn(
        "data-[state=open]:animate-in data-[state=open]:fade-in-0 fixed inset-0 z-50 bg-black/50",
        className,
      )}
      {...props}
    />
  );
}

/**
 * 弹窗宽度档位。
 *
 * - `default` = 420px（原型 `.confirm`，二选一确认框用）；
 * - `wide` = 620px（**内容型**弹窗：升级前版本说明这类分组长文本，420 会把每行挤成三四个字）。
 *
 * 为什么做成档位而不是让调用方写 `!max-w-[620px]`：基础样式里已经有 `sm:max-w-[420px]`，
 * 同 variant 同属性叠加时**谁生效取决于 Tailwind 生成顺序**（不是 class 书写顺序），
 * 靠 `!important` 兜底等于把可预测性换成运气。
 */
type DialogWidth = "default" | "wide";

function DialogContent({
  className,
  children,
  showCloseButton = true,
  width = "default",
  /* `flush` = 内距与行距交给调用方（页头 / 可滚正文 / 固定动作条三段式）。
     这类弹窗需要「正文滚动、动作条钉底」，统一 `p-5 + gap-4` 会把三段挤成一段。 */
  flush = false,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Content> & {
  showCloseButton?: boolean;
  width?: DialogWidth;
  flush?: boolean;
}) {
  return (
    <DialogPortal data-slot="dialog-portal">
      <DialogOverlay />
      <DialogPrimitive.Content
        data-slot="dialog-content"
        className={cn(
          /* ★ 逐值对齐原型 `.confirm`（`demo.css:591-599`）：
             底 `--popover`（**不是** `--background` —— 深色下 background=0.208 比 popover=0.279
             明显发暗，浮层会「沉下去」）、内距 20px、圆角 16px、
             **靠上 6vh**（不是垂直居中）、阴影用本仓三级 elevation 的最高档。
             ⚠️ `max-w` 的任意值里 `calc()` **必须**用 `_` 写空格（Tailwind 的转义）：
             直接写 `calc(100%-2rem)` 会产出无效 CSS（`-` 两侧无空格），整条 `max-width`
             被浏览器静默丢弃 —— 窄窗口下弹窗会顶到视口边缘。
             ★ **必须限高可滚**（issue #79 同一条教训）：`fixed` + `top-[6vh]` 且不限高时，
             内容一旦高过 `94vh`（小窗口 / 大字号缩放 / 长正文）就会**向下溢出到视口之外**，
             既没有滚动条也够不到 `DialogFooter` 的按钮。而 Radix modal 在弹窗打开期间给
             `body` 挂 `pointer-events:none`，遮罩又铺满全屏 ⇒ 用户看到的就是「界面卡死、
             任何地方都点不动」（issue #80 的明文弹窗正是这种：Esc 与点外部都被拦，只剩
             底部一个按钮，按钮再够不到就彻底锁死）。留 6vh 上下边距，故上限 `100dvh - 12vh`。
             ★★ **内容也不加退出动画**（与遮罩同一理由，issue #80）：Radix `Presence` 靠
             `animationend` 决定何时卸载关闭中的节点，而该事件在部分引擎 / 合成路径上不保证
             送达（WebKit 上已实测出现过）。一旦收不到，内容节点永远挂在 `unmountSuspended`：
             弹窗处于 `data-state="closed"` 却仍在 DOM 里，而 React 的 `open` 早已置 false
             ⇒ 底部按钮与右上角 X **全都变成空操作**（再点也不会改状态），弹窗永久关不掉，
             用户只能杀进程重启 —— 这正是「创建密钥后界面卡死」的再现形态。没有退出动画时
             `getAnimationName()` 读到 `none`，`Presence` **立即卸载**，「关弹窗 ⇒ 节点消失」
             从此不依赖任何动画事件。进入动画保留（挂载不等待事件，无此风险）。 */
          "bg-popover data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 fixed left-[50%] top-[6vh] z-50 grid max-h-[calc(100dvh_-_12vh)] w-full max-w-[calc(100%_-_2rem)] -translate-x-1/2 rounded-[16px] border shadow-overlay duration-200",
          width === "wide" ? "sm:max-w-[620px]" : "sm:max-w-[420px]",
          /* `flush` 自己管滚动（三段式：页头 / 可滚正文 / 钉底动作条），别再叠 `overflow-y-auto`
             与它的 `overflow-hidden` 抢同一属性。 */
          flush ? "gap-0 overflow-hidden p-0" : "gap-4 overflow-y-auto p-5",
          className,
        )}
        {...props}
      >        {children}
        {showCloseButton && (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            className="ring-offset-background focus:ring-ring data-[state=open]:bg-accent data-[state=open]:text-muted-foreground absolute top-4 right-4 rounded-xs opacity-70 transition-opacity hover:opacity-100 focus:ring-2 focus:ring-offset-2 focus:outline-hidden disabled:pointer-events-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4"
          >
            <XIcon />
            <span className="sr-only">Close</span>
          </DialogPrimitive.Close>
        )}
      </DialogPrimitive.Content>
    </DialogPortal>
  );
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-header"
      className={cn("flex flex-col gap-2 text-center sm:text-left", className)}
      {...props}
    />
  );
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn("flex flex-col-reverse gap-2 sm:flex-row sm:justify-end", className)}
      {...props}
    />
  );
}

function DialogTitle({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Title>) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      /* 原型 `.confirm h3`（`demo.css:597`）：`15px` / `font-weight:640`（不是 shadcn 的18px/600） */
      className={cn("text-[15px] leading-tight font-[640]", className)}
      {...props}
    />
  );
}

function DialogDescription({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Description>) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      /* 原型 `.confirm p`（`demo.css:598`）：`12.5px` / `line-height:1.6` / muted */
      className={cn("text-muted-foreground text-[12.5px] leading-relaxed", className)}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
