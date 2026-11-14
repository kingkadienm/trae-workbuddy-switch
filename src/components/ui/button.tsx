import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/**
 * ★ 按钮几何**逐值对齐原型** `.btn`（`buddy-switch-prototype.html:191-207` / `demo.css:191-207`）。
 *
 * ## 变体映射（原型只有 4 个，shadcn 有 6 个 —— 一一对上，不新增自定义设计）
 *
 * | 原型类 | 视觉 | 本组件 variant |
 * |:--|:--|:--|
 * | `.btn` | `card-2` 底 + 边框（**最常见的那个**） | `outline` |
 * | `.btn.primary` | `primary` 实底 | `default` |
 * | `.btn.ghost` | 透明 + 弱化字色 | `ghost` |
 * | `.btn.danger` | **描边式**（`destructive` 字色 + 40% 描边） | `destructive` |
 *
 * ⚠️ **`destructive` 是描边式而非实心** —— 原型的不可逆操作确认框
 * （`buddy-switch-prototype.html:1027`）确认按钮就是 `btn danger`，
 * 全站仅此一处 `danger` 用法，故按描边式对齐。改前请先看这条。
 *
 * ⚠️ 原型 `.btn` 的底是 `--card-2`（比card 深一层的凹陷面），**不是** `--background`；
 * 直接沿用 shadcn 的 `bg-background` 会让按钮比原型亮一档。
 *
 * 尺寸阶梯：`.btn` 34px / `.sm` 28px / `.xs` 24px，字号 12.5 / 12 / 11
 * —— 原型三档都比 shadcn 默认（36/32）矮，**全套下移 2px**。
 */
const buttonVariants = cva(
  "inline-flex cursor-pointer items-center justify-center gap-1.5 whitespace-nowrap rounded-[10px] text-[12.5px] font-medium transition-all disabled:pointer-events-none disabled:opacity-45 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-[14px] shrink-0 [&_svg]:shrink-0 outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] aria-invalid:ring-destructive/20 aria-invalid:border-destructive",
  {
    variants: {
      variant: {
        /* 原型 `.btn.primary`：实底primary，hover 用 `brightness(1.08)` 而非变淡 —— 淡出会让
           实底按钮的对比度掉下去，原型刻意选了提亮。
           ⚠️ 必须写任意值 `brightness-[1.08]`：Tailwind 主题里**没有** `--brightness-*` 键，
           写 `brightness-108` 会被静默丢弃（不报错、样式不生效）。 */
        default: "bg-primary text-primary-foreground hover:brightness-[1.08]",
        /* 原型 `.btn.danger`：描边式（字色 + 40% 描边，hover 才给 12% 底）。 */
        destructive:
          "border border-destructive/40 bg-transparent text-destructive hover:bg-destructive/12 focus-visible:ring-destructive/20",
        /* 原型 `.btn`：`--card-2` 底 + 边框；hover 换 `--accent` 底 + `--border-strong` 描边。 */
        outline:
          "border border-border bg-card-2 hover:border-border-strong hover:bg-accent hover:text-accent-foreground",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/80",
        /* 原型 `.btn.ghost`：**静止态就是弱化字色**，不是前景色（此前漏了这条，
           表现为侧栏里 ghost 按钮比原型更「重」）。 */
        ghost: "bg-transparent text-muted-foreground hover:bg-accent hover:text-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        /* 原型 `.btn`：高 34px、内边距 0 13px（`:193`） */
        default: "h-[34px] px-[13px] has-[>svg]:px-[11px]",
        /* 原型 `.btn.sm`：高 28px、内边距 0 10px、字号 12px、圆角 8px（`:205`） */
        sm: "h-[28px] rounded-[8px] px-[10px] text-[12px] has-[>svg]:px-[8px]",
        /* 原型 `.btn.xs`：高 24px、内边距 0 8px、字号 11px、圆角 7px（`:206`） */
        xs: "h-[24px] rounded-[7px] px-[8px] text-[11px] has-[>svg]:px-[6px]",
        lg: "h-[38px] rounded-[10px] px-[16px] has-[>svg]:px-[13px]",
        /* 图标钮与 `.btn` 同高（原型 `.icon-btn` 也是 34px，`demo.css:125`） */
        icon: "size-[34px]",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

function Button({
  className,
  variant,
  size,
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
  }) {
  const Comp = asChild ? Slot : "button";
  return (
    <Comp
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };
