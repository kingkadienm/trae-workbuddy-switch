import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/**
 * ★ 徽章几何**逐值对齐原型** `.badge`（`demo.css:244-253`）。
 *
 * 原型是**固定高 20px**、圆角 6px、字号 10.5px、内边距 `0 7px`、图标 11px；
 * shadcn 默认是 `py-0.5` 自适应高、圆角 10px、字号 12px、内边距 `0 8px`、图标 12px
 * ⇒ 此前每一项都偏大。徽章全站 30 个文件在用，改一处即全站生效。
 *
 * 变体底色也按原型百分比对齐：`ok`=success **16%**、`warn`=warning **18%**
 * （原型是 `color-mix(... 16% / 18%, transparent)`），此前一律写 15%。
 *
 * ⚠️ 固定 `h-5` + `max-w-full` 是原型的形，但**必须同时处理溢出**：徽章是 `w-fit` 的
 * inline-flex + `whitespace-nowrap`，父容器不够宽时 `overflow-hidden` 会把右侧字符
 * **物理裁掉且没有任何提示**。真实受害点：`client-limits-card.tsx` 传后端模型名
 * （`client_limits.rs` 的 `model: Option<String>`，运行时数据、无长度上限）、
 * `TraeCreditsPage` 传用户自定义分组名（限 40 字）。
 * ⇒ 外层给 `max-w-full`（允许收缩），内层 span 给 `truncate`（真正出省略号，见下方说明）。
 */
const badgeVariants = cva(
  /* ⚠️★ 图标选择器必须是**后代**（`[&_svg]`）而不是直接子级（`[&>svg]`）。
     下面为了「长文本出省略号」把 children 包进了内层 `truncate` span ⇒ 图标**不再**是
     `Badge` 的直接子级，`[&>svg]:size-[11px]` 直接失配、图标回到 lucide 默认 16px。
     更糟的是 Tailwind preflight 给 `svg` 设了 `display:block`，而内层 span 是 block：
     图标与文字于是**上下堆叠成两行**（实测内层 33px），塞进 `h-5`（内容区 18px）的徽章里
     被 `overflow-hidden` **硬裁 13px** —— 用户看到的就是「徽章里的图标/文字被切掉」。
     ⇒ ① 后代选择器让图标回到 11px；② `[&_svg]:inline` 抵消 preflight 的 `display:block`，
     让图标与文字回到**同一行**（`white-space:nowrap` 下仍是一行，`truncate` 的省略号照常生效）。
     变异：把 `[&_svg]` 改回 `[&>svg]` ⇒ 图标变 16px 且堆叠，徽章内容被裁（实测 scrollHeight 25 > 18）。 */
  "inline-flex h-5 max-w-full items-center justify-center gap-1 rounded-[6px] border px-[7px] text-[10.5px] font-medium w-fit whitespace-nowrap shrink-0 [&_svg]:size-[11px] [&_svg]:shrink-0 [&_svg]:pointer-events-none [&_svg]:inline focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] aria-invalid:ring-destructive/20 aria-invalid:border-destructive transition-[color,box-shadow] overflow-hidden",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground [a&]:hover:bg-primary/90",
        secondary:
          "border-transparent bg-secondary text-secondary-foreground [a&]:hover:bg-secondary/90",
        destructive:
          "border-transparent bg-destructive text-white [a&]:hover:bg-destructive/90 focus-visible:ring-destructive/20",
        outline: "text-foreground [a&]:hover:bg-accent [a&]:hover:text-accent-foreground",
        /* 原型 `.badge.ok`：success 16%（此前 15%） */
        success: "border-transparent bg-success/16 text-success",
        /* 原型 `.badge.warn`：warning 18%（此前 15%） */
        warning: "border-transparent bg-warning/18 text-warning",
        /* 原型 `.badge.info`：primary **16%** 底 + primary 字（`demo.css:272`）。
           卡片上的「当前登录」「使用中 N 个工具」「优先」都用它 —— 此前各写一遍
           `bg-primary/15`，三处百分比还会各自漂移。 */
        info: "border-transparent bg-primary/16 text-primary",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

function Badge({
  className,
  variant,
  asChild = false,
  children,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : "span";
  return (
    <Comp data-slot="badge" className={cn(badgeVariants({ variant }), className)} {...props}>
      {/* ⚠️ 省略号必须落在**内层 span** 上，不能只给外层加 `truncate`：
          外层是 `inline-flex`，其文本节点会成为**匿名 flex 项**，
          而 `text-overflow:ellipsis` 只对**块容器**生效 ⇒ 只加在外层等于没加。
          内层 span 是块容器，`min-w-0` 让它能在 flex 里收缩，溢出才变省略号。
          ⚠️ `asChild` 时**不能**再包一层（会多出非法嵌套），故按原样透传 children。 */}
      {asChild ? (
        children
      ) : (
        <span className="min-w-0 truncate" title={typeof children === "string" ? children : undefined}>
          {children}
        </span>
      )}
    </Comp>
  );
}

export { Badge, badgeVariants };
