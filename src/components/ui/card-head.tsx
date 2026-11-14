/**
 * 卡片头（原型 `.card-head`，`demo.css:215-220`）—— 用量页各卡片**唯一**的头部实现。
 *
 * ## 为什么收成一个原语
 *
 * 改造前用量页里「卡片头」有**两种写法**：WorkBuddy 侧走 `ChartFrame` 的内联头部，
 * Trae 侧是一堆手写的 `<div className="… border-b border-border/60 px-5 py-3">` +
 * `<span className="text-[13px] font-medium">`。同一个「用量」页里两种字号、两种内边距，
 * 肉眼就是「两套设计」（VISUAL-ALIGN §6 已把这条记为待收口项）。
 *
 * 逐值（`demo.css:215-220`）：
 * - 容器 `gap: 10px` · `padding: 13px 16px` · `border-bottom: 1px solid var(--border)`；
 * - 标题 `font-size: 13.5px` · `font-weight: 600`；
 * - 说明（`.sub`）`font-size: 11.5px` · `color: var(--muted-fg)`。
 *
 * ## 与原型的两处**有意偏离**（都已登记，勿当 bug 修）
 *
 * 1. **说明在标题下方**，原型是右侧同排（`[h3][grow][sub]`）。
 *    本仓的图注是**整句**（`lib/chart-caption.ts` 产出，且被门禁断言内容，例如
 *    「余额曲线图注必须注明『回推』与『非 0 基线』」）—— 挤到右侧会被截断成读不懂的碎片。
 *    故保留两行：标题一行、说明一行。
 * 2. **下边框用 `border-border/60`**，原型是 `var(--border)`。
 *    全仓 20+ 处卡片头都是 `/60`，只改用量页会让它**与其余页面**不一致 ——
 *    那比与原型的 40% 透明度差更显眼。全量收口留待单独一轮。
 *
 * ## 为什么不直接用 `CardHeader`
 *
 * shadcn 的 `CardHeader` 是 `px-6 py-6` 的纵向容器（`ui/card.tsx`），
 * 与原型 `.card-head` 的 `13px 16px` 横条不是同一个东西；覆盖它的样式等于没用它。
 */

import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export interface CardHeadProps {
  /** 已 i18n 的标题。 */
  title: string;
  /** 说明 / 图注（原型 `.sub`）。 */
  caption?: string;
  /** 右侧动作区（图表导出、图-表切换、计数徽标…）。 */
  actions?: ReactNode;
  className?: string;
}

export function CardHead({ title, caption, actions, className }: CardHeadProps) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-wrap items-start justify-between gap-2 border-b border-border/60 px-4 py-[13px]",
        className,
      )}
    >
      <div className="min-w-0">
        <h3 className="text-[13.5px] font-semibold leading-5">{title}</h3>
        {caption ? (
          <p className="mt-0.5 text-[11.5px] leading-4 text-muted-foreground">{caption}</p>
        ) : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-1">{actions}</div> : null}
    </div>
  );
}
