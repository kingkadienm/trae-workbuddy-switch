"use client";

import { Check, Gauge, KeyRound, Loader2, RefreshCw, Sparkles, type LucideIcon } from "lucide-react";

import { DropdownMenuItem } from "@/components/ui/dropdown-menu";
import { ACCOUNT_REFRESH_KINDS, type AccountRefreshKind } from "@/lib/account-refresh";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * 卡片「更多」菜单里三平台共用的积木：**头部 / 分组小标题 / 四项刷新 / 不可用状态行 / 结尾提示**。
 *
 * ## 为什么要抽出来
 *
 * WorkBuddy / Trae / Qoder 三张卡片的菜单结构一样、图标与文案也一样。各写一份的下场
 * 在本仓已经出现过多次（`gateway-snippets`、抽屉壳）：三处会立刻在文案、禁用逻辑、
 * 进行中态上漂移。所以「四项刷新」只在这里定义一次，卡片只负责把它摆进菜单。
 *
 * ## 菜单为什么**不自动关闭**（与原型 `acctMenuInner` 同一决定）
 *
 * 刷完一项还想刷下一项很常见（先看凭据，再决定要不要刷积分）。Radix 默认选中即关：
 * 一次刷新要重新点开菜单，而且「刷新中 / 已更新」这两个状态只在菜单里看得见。
 * 所以刷新项 `preventDefault()` 保持菜单打开；其余动作（签到 / 导出 / 删除）仍按默认关闭。
 *
 * ## 为什么是四项，而不是一个「刷新」
 *
 * 见 `lib/account-refresh.ts` 的文件头：四类数据来自不同层（上游 / 本机日志 / 本机签到 /
 * 本机凭据），快慢与失败原因都不同；合成一个动作时慢的拖住快的，且**失败原因分不清**。
 */

/** 菜单头部：账号名（原型 `.am-head`）。点它没有动作，只说明「这些操作作用于谁」。 */
export function MenuHead({ children }: { children: React.ReactNode }) {
  return <div className="px-2 pt-1 pb-1.5 text-[11.5px] font-medium text-muted-foreground">{children}</div>;
}

/** 分组小标题（原型 `.am-grp`）。 */
export function MenuGroupLabel({ children, className }: { children: React.ReactNode; className?: string }) {
  /* 不用 `DropdownMenuLabel`：本仓的 `ui/dropdown-menu` 没有导出它（只有 Menu / Item /
     Separator / Trigger / Content），为了一个 `<div>` 去改原语不划算。 */
  return (
    <div
      role="presentation"
      className={cn("px-2 pt-1 pb-0.5 text-[10.5px] font-normal tracking-wide text-muted-foreground", className)}
    >
      {children}
    </div>
  );
}

/** 菜单结尾提示（原型 `.am-note`）：告诉用户备注与登录态在哪，省得再找。 */
export function MenuNote({ children }: { children: React.ReactNode }) {
  return <div className="px-2 pt-1.5 text-[10.5px] leading-snug text-muted-foreground/80">{children}</div>;
}

/** 不可用的状态行（原型里「今日已签到」「需先登录」两态共用这个形状）。 */
export function MenuStateItem({
  icon,
  children,
  hint,
  state,
}: {
  icon: React.ReactNode;
  children: React.ReactNode;
  /** 悬停解释（禁用项的 `title`，Radix 不会吞掉）。 */
  hint?: string;
  /** 右对齐的状态词（如「需先登录」）。 */
  state?: string;
}) {
  return (
    <div
      role="menuitem"
      aria-disabled
      title={hint}
      className="flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-muted-foreground"
    >
      <span className="grid size-4 shrink-0 place-items-center">{icon}</span>
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {state ? <span className="ml-2 shrink-0 text-[10.5px] opacity-80">{state}</span> : null}
    </div>
  );
}

const REFRESH_ITEMS = [
  { kind: "credits", icon: RefreshCw, labelKey: "shared.cardMenu.refresh.credits", hintKey: "shared.cardMenu.refresh.creditsHint" },
  { kind: "tokens", icon: Gauge, labelKey: "shared.cardMenu.refresh.tokens", hintKey: "shared.cardMenu.refresh.tokensHint" },
  { kind: "checkin", icon: Sparkles, labelKey: "shared.cardMenu.refresh.checkin", hintKey: "shared.cardMenu.refresh.checkinHint" },
  { kind: "credential", icon: KeyRound, labelKey: "shared.cardMenu.refresh.credential", hintKey: "shared.cardMenu.refresh.credentialHint" },
] as const satisfies ReadonlyArray<{
  kind: AccountRefreshKind;
  icon: LucideIcon;
  labelKey: string;
  hintKey: string;
}>;

export function RefreshMenuGroup({
  refreshingKinds,
  doneKinds,
  kinds = ACCOUNT_REFRESH_KINDS,
  onPick,
}: {
  /** 本账号正在进行中的刷新项。 */
  refreshingKinds?: readonly AccountRefreshKind[];
  /** 刚刚刷新成功（会自行淡出）的项。 */
  doneKinds?: readonly AccountRefreshKind[];
  /**
   * 本平台**真的能刷**的项（默认四项全给）。
   *
   * ⚠️ 这是「不许假装」的闸门：没有该项的只读重取接口时，宁可少给一项，
   * 也不要摆一个点了只会重读缓存的菜单项。
   */
  kinds?: readonly AccountRefreshKind[];
  onPick: (kind: AccountRefreshKind) => void;
}) {
  const t = useT();
  const items = REFRESH_ITEMS.filter((item) => kinds.includes(item.kind));
  if (items.length === 0) return null;
  return (
    <>
      <MenuGroupLabel>{t("shared.cardMenu.refreshGroup")}</MenuGroupLabel>
      {items.map(({ kind, icon: Icon, labelKey, hintKey }) => {
        const busy = refreshingKinds?.includes(kind) ?? false;
        const done = doneKinds?.includes(kind) ?? false;
        return (
          <DropdownMenuItem
            key={kind}
            disabled={busy}
            title={t(hintKey)}
            /* ⚠️ 保菜单开着：刷完一项常常接着刷下一项，且「刷新中 / 已更新」只有菜单里看得见。 */
            onSelect={(event) => {
              event.preventDefault();
              onPick(kind);
            }}
          >
            <span className="grid size-4 shrink-0 place-items-center">
              {busy ? <Loader2 className="size-4 animate-spin" /> : done ? <Check className="size-4 text-primary" /> : <Icon />}
            </span>
            <span className="min-w-0 flex-1 truncate">{t(labelKey)}</span>
            {busy ? (
              <span className="ml-2 shrink-0 text-[10.5px] text-muted-foreground">{t("shared.cardMenu.busy")}</span>
            ) : done ? (
              <span className="ml-2 shrink-0 text-[10.5px] text-primary">{t("shared.cardMenu.done")}</span>
            ) : null}
          </DropdownMenuItem>
        );
      })}
    </>
  );
}

/** 顺序对外暴露一份，供调用方做类型收窄（不必再 import 数组常量）。 */
export { ACCOUNT_REFRESH_KINDS };
