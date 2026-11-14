/**
 * 浮层 · 弹层（Radix Popover）。
 *
 * ## 为什么新增这个原语
 *
 * 用量页参数条的「自定义时间」要用户在**不离开页面**的前提下填一对日期，
 * 关掉后接着看同一张图。三种已有浮层都不合适：
 *
 * - `DropdownMenu`：语义是「选一项、选中即关」。里面放 `<input>` 会被它的
 *   键盘处理吃掉（字符键跳到下一个 item、Esc 关菜单），用户根本敲不进日期；
 * - `Dialog` / `Sheet`：**模态**，遮住整页 —— 而这里用户要**边看边选**；
 * - `Tooltip`：不可聚焦、不可交互。
 *
 * ⇒ 按 `AGENTS.md`「缺原语就补 shadcn/Radix 并收进 `src/components/ui/`」补这一个。
 * 几何与 `DropdownMenuContent` **同源**（`rounded-lg border bg-popover shadow-overlay`
 * + 同一批 `data-[state]` 动效），保证同一页里两个浮层看起来是一套。
 *
 * ## 与 `DropdownMenu` 的一处刻意差异
 *
 * `sideOffset` 默认 **6px**（与 `DropdownMenuContent` 一致），但内容**不设 `min-w-36`**：
 * 弹层的宽度由调用点按内容给（这里是一对日期输入），默认最小宽度只会让它看起来偏窄。
 */

import * as React from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";

import { cn } from "@/lib/utils";

function Popover(props: React.ComponentProps<typeof PopoverPrimitive.Root>) {
  return <PopoverPrimitive.Root data-slot="popover" {...props} />;
}

function PopoverTrigger(props: React.ComponentProps<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger data-slot="popover-trigger" {...props} />;
}

/**
 * 定位锚点。**不给 `Trigger` 时用它**：调用点自己控制 `open`
 * （参数条的「自定义时间」是 `.seg` 里的一枚按钮，它不是一个「触发器按钮」，
 * 而整条 `.seg` 才是定位参照）。
 */
function PopoverAnchor(props: React.ComponentProps<typeof PopoverPrimitive.Anchor>) {
  return <PopoverPrimitive.Anchor data-slot="popover-anchor" {...props} />;
}

function PopoverContent({
  className,
  align = "center",
  sideOffset = 6,
  ...props
}: React.ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        data-slot="popover-content"
        align={align}
        sideOffset={sideOffset}
        className={cn(
          "z-50 rounded-lg border bg-popover p-1.5 text-popover-foreground shadow-overlay outline-none",
          /* ★ 浮层必须限高可滚（本仓硬约束，issue #79/#80）：不限高 + 内容长
             ⇒ 超出窗口的部分被切掉且**没有滚动条**，用户以为内容就这么点。
             用 Radix 给的可用高度变量，窗口再小也不会溢出。 */
          "max-h-[var(--radix-popover-content-available-height)] overflow-y-auto",
          /* 退出动画刻意不加（issue #80）：`Presence` 等 `animationend` 才卸载，
             该事件在部分引擎上不保证送达 ⇒ 浮层关不掉、幽灵节点留在 DOM 里。
             进入动画保留（挂载不等待事件）。 */
          "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          "data-[side=bottom]:slide-in-from-top-1 data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1 data-[side=top]:slide-in-from-bottom-1",
          className,
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverAnchor, PopoverContent, PopoverTrigger };
