/**
 * 首次引导条 —— P6 / `P6-ARCH.md` §1.5。
 *
 * ## 为什么内容要「由真实绑定派生」
 *
 * 原型引导条（`index.html:128-132`）**写死**了几条快捷键。真实应用里那样会制造
 * 「引导里列了、按了却没反应」的**假承诺**（§3.4 规则 1）。因此这里：
 * - `⌘K` **恒列**（命令面板总在）；
 * - `⌘1..N` **仅在 `planScopeShortcuts` 的 `bindings` 非空时**才出现；
 * - `/`（列表搜索，`AccountsPage` 已实现）可列。
 *
 * 降级（>9 个作用域 / 用户关闭）⇒ 那一行**根本不渲染**，与快捷键的绑定状态同源。
 *
 * ## 形态与退场
 *
 * **底部居中浮层**（原型 `.hint`，`demo.css:538-547`：`fixed` + `bottom:20px` +
 * `left:50%` + `translateX(-50%)`、`rounded-[12px]`、底 `--popover`、`shadow-overlay`、
 * 字号 12px、`max-width: min(760px,94vw)`）。
 *
 * ⚠️ 原先是**顶部通栏**（`border-b` + 通栏底色）：它会把内容整体下推一截，且与页头
 * 争夺注意力。原型做成浮层正是为了「不挤压内容、不抢视觉重心」。
 *
 * ⚠️ 浮层会盖住底部内容 ⇒ 外层 `pointer-events-none` + 本体 `pointer-events-auto`，
 * 使浮层**空隙处**的点击能穿透到下面的卡片，不挡住操作。
 *
 * 关闭即**持久化**（`lib/first-run.ts`），下次冷启动不再出现。
 * 刻意不做「自动退场」—— 那会让每次冷启动都重来一遍。
 */

import { useMemo, useState } from "react";
import { X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { dismissFirstRunHint, shouldShowFirstRunHint } from "@/lib/first-run";
import { useT } from "@/lib/i18n";
import { planScopeShortcuts } from "@/lib/scope-shortcuts";
import { useShortcutsEnabled } from "@/lib/shortcut-preference";
import { useVisibleScopes } from "@/lib/scope-visibility";

/** 取可用的 `localStorage`（不可用返回 `undefined`）。 */
function safeStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

export function FirstRunHint() {
  const t = useT();
  const [shortcutsEnabled] = useShortcutsEnabled();
  const visibleScopes = useVisibleScopes((s) => s.visibleScopes);
  const [dismissed, setDismissed] = useState(() => !shouldShowFirstRunHint(safeStorage()));

  const plan = useMemo(
    () => planScopeShortcuts({ scopes: visibleScopes, enabled: shortcutsEnabled }),
    [visibleScopes, shortcutsEnabled],
  );

  if (dismissed) return null;

  // ⌘1..N 只在**绑定存在**时列出（降级 ⇒ 不写进引导，避免假承诺）。
  const scopeHint = plan.bindings.size > 0 ? `⌘1–⌘${plan.bindings.size}` : null;

  function dismiss() {
    dismissFirstRunHint(safeStorage());
    setDismissed(true);
  }

  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-5 z-50 flex justify-center px-4">
      <div className="pointer-events-auto flex w-full max-w-[760px] items-start gap-3 rounded-[12px] border border-border bg-popover px-3.5 py-2.5 text-xs shadow-overlay">
        <span className="shrink-0 font-medium text-primary">{t("firstRun.title")}</span>
      <ul className="min-w-0 flex-1 space-y-0.5 text-muted-foreground">
        <li>{t("firstRun.openPalette", { shortcut: "⌘K" })}</li>
        {scopeHint ? <li>{t("firstRun.scopeShortcut", { shortcut: scopeHint })}</li> : null}
        <li>{t("firstRun.listSearch")}</li>
      </ul>
      <Button
        variant="ghost"
        size="sm"
        className="h-6 shrink-0 px-2 text-xs"
        onClick={dismiss}
      >
        <X className="size-3.5" aria-hidden="true" />
        {t("firstRun.dismiss")}
      </Button>
      </div>
    </div>
  );
}
