/**
 * 页内**左侧子导航**（L2.5）—— 网关 / 设置两个工作台共用同一份版式。
 *
 * ## 为什么抽出来
 *
 * 设置页（9 项）与网关页（**7 项**；Trae 侧去掉「请求代理」为 6 项）的子导航**外观与行为完全相同**：
 * 窄屏横向换行、宽屏纵向列表、`aria-current` 标当前位置、点击只切视图。
 * 两份实现必然漂移（本仓已发生过多次），故只留一份。
 *
 * ## 为什么不做成「可折叠 + 记住位置」
 *
 * 子导航回答的是「这一类里有哪些项」，不是「我在哪一步」；位置真值在 URL
 * （网关用 `?sub=`、设置用 `?section=`）。本组件只负责**切换视图**，
 * 选中态交给调用方（`active` / `onSelect`），**不自持状态** —— 否则
 * 「刷新后回到第一项」这类问题要在两个地方各修一遍。
 *
 * ## 逐值对齐原型（`demo.css:370-379`）
 *
 * | 原型 | 本仓 |
 * |:---|:---|
 * | `.subnav { width:186px; gap:2px; sticky top:26px }` | `[data-side-subnav]`（`index.css`）+ `gap-0.5` |
 * | `button { height:34px; padding:0 10px; border-radius:9px; font-size:12.5px; gap:8px }` | `h-[34px] px-2.5 rounded-[9px] text-[12.5px] gap-2` |
 * | `:hover / .on { background: var(--accent) }` | `hover:bg-accent` / `bg-accent` |
 * | `.on { font-weight:550 }` + `::before` 3×14 竖条（`ml:-10px mr:4px`） | `font-[550]` + `[data-subnav-marker]`（`-ml-2.5 mr-1`） |
 * | 900px 以下转横向换行 + 隐藏竖条 | `index.css` 的 `@media (max-width:900px)` |
 *
 * ⚠️ **原型子导航没有图标**（`demo-app.js` 里 subnav 按钮只渲染 label）。
 * 本仓此前多了一枚 16px 图标 ⇒ 属于「demo 里不存在的自定义设计」，已删除。
 * 按钮因此只剩一个文本节点，`truncate` 直接挂在它上面。
 */

import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import type { TranslationKey } from "@/locales/zh";

export interface SideSubnavItem<Id extends string = string> {
  id: Id;
  /** 标签键（应用级导航，落在 `shell` 域）。 */
  labelKey: TranslationKey;
}

interface Props<Id extends string> {
  active: Id;
  items: readonly SideSubnavItem<Id>[];
  onSelect: (id: Id) => void;
  /** 无障碍名称：各页自己的说法（如「设置分区」「网关分区」）。 */
  ariaLabel: string;
  className?: string;
}

/**
 * 左侧子导航。
 *
 * 方向 / 宽度 / 定位由 `index.css` 的 `[data-side-subnav]` 负责（含 900px 断点），
 * 这里只写**按钮内部**的样式 —— 理由见 `index.css` 该规则上的注释。
 */
export function SideSubnav<Id extends string>({
  active,
  items,
  onSelect,
  ariaLabel,
  className,
}: Props<Id>) {
  const t = useT();
  return (
    <nav aria-label={ariaLabel} data-side-subnav className={cn("gap-0.5", className)}>
      {items.map((item) => {
        const on = item.id === active;
        return (
          <button
            key={item.id}
            type="button"
            aria-current={on ? "page" : undefined}
            onClick={() => onSelect(item.id)}
            className={cn(
              "flex h-[34px] shrink-0 cursor-pointer items-center gap-2 rounded-[9px] px-2.5 text-left text-[12.5px] transition-colors",
              on
                ? "bg-accent font-[550] text-foreground"
                : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            {/* 选中态竖条（原型 `.subnav button.on::before`）：`-ml-2.5` 抵消按钮左内边距，
                让它贴齐按钮左边缘；`mr-1` 与父级 `gap-2` 合计 12px = 原型的 8+4。 */}
            {on && (
              <span
                aria-hidden="true"
                data-subnav-marker
                className="-ml-2.5 mr-1 h-[14px] w-[3px] shrink-0 rounded-[2px] bg-primary"
              />
            )}
            <span className="truncate">{t(item.labelKey)}</span>
          </button>
        );
      })}
    </nav>
  );
}
