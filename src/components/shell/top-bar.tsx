import { Loader2, RefreshCw, Search } from "lucide-react";

import { AppMenu, ThemeMenu, iconButtonClass } from "@/components/shell/app-menu";
import { RunPill } from "@/components/shell/run-pill";
import { ScopeSelector } from "@/components/shell/scope-selector";
import { useT } from "@/lib/i18n";
import { isMacOverlayTitleBar } from "@/lib/platform";
import type { Scope } from "@/lib/scope";
import { useRefreshAction } from "@/lib/use-refresh-action";
import type { ScopeStatusMap } from "@/lib/use-scope-status";
import { cn } from "@/lib/utils";

/**
 * **顶部应用栏** —— L1 作用域 + 全局动作（§2.1）。
 *
 * ## 三段式的第一段
 *
 * 骨架是「顶栏 + 侧栏 + 内容区」（原型 `index.html:11-79`）：
 * 顶栏承载**跨页恒定的全局上下文与动作** —— 当前作用域、运行态、搜索、刷新、主题、
 * 应用设置。侧栏只承载**目的地**（L2），内容区承载视图参数（L3）。
 * 把作用域留在侧栏（旧 `ScopeSwitch`）会让它随产品分区换血，正是本次要修掉的错位。
 *
 * ## macOS 红绿灯避让
 *
 * 窗口用 `titleBarStyle: "Overlay"` + `trafficLightPosition{x:16,y:30}` ⇒ 顶栏是
 * **标题栏本身**，左侧要为红绿灯留出 ≥76px。这是**真机才有**的约束：浏览器能看到
 * 覆盖率不足，但看不到红绿灯压住控件 —— 必须真机截图确认。
 *
 * ## 可拖拽
 *
 * `data-tauri-drag-region` 挂在本容器上（子按钮**不带**该属性 ⇒ 照常接收点击）。
 * 因此不再需要旧实现里那条覆盖顶部的固定 32px 拖拽条。
 */
export function TopBar({
  scope,
  statuses,
  shortcutBindings,
  onSelectScope,
  searchLabel,
  onOpenSearch,
  onRefresh,
  aggregateMode = false,
  onSelectAggregate,
}: {
  scope: Scope;
  statuses: ScopeStatusMap;
  shortcutBindings: ReadonlyMap<Scope, number>;
  onSelectScope: (next: Scope) => void;
  /** 搜索按钮文案（传 `t("palette.entry")`——沿用既有词条，不新增同义键）。 */
  searchLabel: string;
  onOpenSearch: () => void;
  /**
   * 刷新当前作用域；**返回 Promise**——进行中态归本组件（`useRefreshAction` 包装），
   * 结果反馈（toast）由调用方在 Promise 内自行给出。
   */
  onRefresh: () => Promise<void>;
  /**
   * 是否处于聚合视图（issue #50）。为 true 时作用域按钮显示「聚合 · 全部账号」，
   * 菜单里高亮聚合项、6 个作用域项都不高亮。
   *
   * ⚠️ 顶栏是聚合页**唯一保留的公共外壳**：聚合入口就在这枚菜单里，
   * 聚合页若没有顶栏，用户进去就出不来。
   */
  aggregateMode?: boolean;
  /** 进入聚合视图。不传 ⇒ 菜单里不渲染「聚合」项。 */
  onSelectAggregate?: () => void;
}) {
  const t = useT();
  const { busy: refreshing, run: runRefresh } = useRefreshAction();

  return (
    <header
      data-tauri-drag-region
      className={cn(
        "flex h-14 shrink-0 items-center gap-2.5 border-b border-border bg-card px-3.5",
        isMacOverlayTitleBar() && "pl-[76px]",
      )}
    >
      <ScopeSelector
        scope={scope}
        statuses={statuses}
        shortcutBindings={shortcutBindings}
        onSelect={onSelectScope}
        aggregate={aggregateMode}
        onSelectAggregate={onSelectAggregate}
      />
      <RunPill status={statuses[scope]} />

      <div className="flex-1" />

      <button
        type="button"
        onClick={onOpenSearch}
        title={t("topbar.search")}
        /* ⚠️ 底色必须是 `--card-2`（原型 `.search-btn` 的 `background: var(--card-2)`，`:136`）：
           顶栏本身是 `bg-card`，而深色下 `--muted` 与 `--card` **同值** ⇒ 用 `bg-muted/40`
           等于「在 card 上再铺一层 card」= 控件看不出边界。

           ⚠️ `min-w` 用**响应式**值：原型写死 `min-width:250px`，但那是给浏览器窗口用的；
           桌面端最小窗口只有 720px（`tauri.conf.json` 的 `minWidth`），而顶栏不可压缩部分
           （作用域 300 + 运行态~65 + 三个图标钮 102 + 间距/内边距 ~78）已占 ~545px
           ⇒ 搜索钮再锁死 250px 就会把右侧的设置按钮挤出视口。
           故宽屏给 250px（与原型一致），窄屏退到 120px，内部的 `truncate` 负责省略。 */
        className="flex h-[34px] w-[250px] min-w-[120px] items-center gap-2 rounded-[10px] border border-border bg-card-2 pl-2.5 pr-2 text-xs text-muted-foreground outline-none transition-colors hover:border-border-strong focus-visible:ring-2 focus-visible:ring-ring/40 lg:w-auto"
      >
        <Search className="size-4 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate text-left">{searchLabel}</span>
        {/* 原型 `.search-btn kbd`（`buddy-switch-prototype.html:139`）：等宽 10.5px、内边距 2/5、
            圆角 5px、`--muted` 底 —— 这些已由 `index.css` 的全局 `kbd` 规则提供，此处不再重复。 */}
        <kbd className="shrink-0 tabular-nums">⌘K</kbd>
      </button>

      <button
        type="button"
        disabled={refreshing}
        aria-busy={refreshing}
        onClick={() => void runRefresh(onRefresh)}
        aria-label={t("topbar.refresh")}
        title={t("topbar.refresh")}
        className={iconButtonClass}
      >
        {/* 进行中 ⇒ Loader2 旋转（仓内「按钮内 Loader2 = 操作进行中」的既定口径）；
            图标同为 16px（原 `size-4`），切换无布局跳动。 */}
        {refreshing ? (
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        ) : (
          <RefreshCw className="size-4" aria-hidden="true" />
        )}
      </button>

      <ThemeMenu />
      <AppMenu />
    </header>
  );
}