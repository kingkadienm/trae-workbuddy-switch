"use client";

import { Check, CircleCheck, Download, Ellipsis, RefreshCw, Sparkles, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MenuGroupLabel, MenuHead, MenuNote, MenuStateItem, RefreshMenuGroup } from "@/components/account-menu-parts";
import { DemoAction } from "@/components/demo-action";
import type { AccountRefreshKind } from "@/lib/account-refresh";
import { useT } from "@/lib/i18n";

/**
 * 账号的「更多」菜单（**卡片与表格共用同一份**）。
 *
 * ## 为什么抽成组件而不是两处各写一遍
 *
 * 表格视图与卡片视图列的是同一批账号、同一批动作。此前只有卡片有菜单，于是用户在表格里
 * 只能「点行进详情」再找动作 —— 这是「同一能力两种形态」的经典分叉。共用一份组件后：
 * 菜单结构、文案、禁用逻辑、进行中态只有一处定义，加一项动作不会漏掉另一个视图。
 *
 * ## 结构逐项对齐原型 `acctMenuInner`
 *
 * 头部（账号名）· 刷新组（四项）· 账号组（签到三态 / 导出账号）· 删除 · 结尾提示。
 * **两边不一致时以原型为准** —— 原型是设计评审通过的那一版。
 *
 * ⚠️ 菜单里**没有**「查看详情」：原型把「打开详情」放在卡片/行的整体点击上，
 * 再摆一枚同义按钮就是两个入口做同一件事。
 */
export interface AccountMoreMenuProps {
  /** 账号展示名（已过 `displayText`）。 */
  accountName: string;
  /** 单项刷新；不传 ⇒ 退回旧的单条「刷新 Token」项（向后兼容）。 */
  onRefreshPart?: (kind: AccountRefreshKind) => void;
  refreshingKinds?: readonly AccountRefreshKind[];
  doneKinds?: readonly AccountRefreshKind[];
  refreshKinds?: readonly AccountRefreshKind[];
  /** 旧的单条刷新（仅在没有 `onRefreshPart` 时用）。 */
  onRefresh?: () => void;
  /** 今日签到状态：`true` 已签 / `false` 未签 / `undefined` 未知（未知 ⇒ 不显示签到行）。 */
  todayCheckedIn?: boolean;
  /** 凭据已过期（与 `needsRelogin` 一起决定「需先登录」态）。 */
  expired?: boolean;
  needsRelogin?: boolean;
  onCheckin?: () => void;
  onExport?: () => void;
  onGrowthTasks?: () => void;
  onToggleActive?: () => void;
  deactivated?: boolean;
  onDelete: () => void;
  /** 演示模式下把菜单整体禁用（与卡片一致）。 */
  featuresDisabled?: boolean;
  /** 触发按钮的尺寸（表格里用更紧凑的）。 */
  compact?: boolean;
}

export function AccountMoreMenu({
  accountName,
  onRefreshPart,
  refreshingKinds,
  doneKinds,
  refreshKinds,
  onRefresh,
  todayCheckedIn,
  expired,
  needsRelogin,
  onCheckin,
  onExport,
  onGrowthTasks,
  onToggleActive,
  deactivated,
  onDelete,
  featuresDisabled = false,
  compact = false,
}: AccountMoreMenuProps) {
  const t = useT();
  const credentialDead = Boolean(expired) || Boolean(needsRelogin);

  const trigger = (
    <Button
      variant="ghost"
      size="sm"
      className={compact ? "!size-7 !px-0" : "!px-[10px]"}
      aria-label={t("wbAccounts.card.manageAria", { name: accountName })}
      title={t("wbAccounts.card.moreActions")}
    >
      <Ellipsis className="size-4" />
    </Button>
  );

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <MenuHead>{accountName}</MenuHead>
        {onRefreshPart ? (
          <RefreshMenuGroup
            refreshingKinds={refreshingKinds}
            doneKinds={doneKinds}
            kinds={refreshKinds}
            onPick={onRefreshPart}
          />
        ) : (
          onRefresh && (
            <DropdownMenuItem disabled={featuresDisabled} onSelect={onRefresh}>
              <RefreshCw />
              {t("wbAccounts.card.refreshToken")}
            </DropdownMenuItem>
          )
        )}
        <DropdownMenuSeparator />
        <MenuGroupLabel>{t("shared.cardMenu.accountGroup")}</MenuGroupLabel>
        {/* ★ 签到是**三态**：已签到 / 需先登录 / 可点。
            缺了前两态就是「卡片给入口、批量却算结构性失败」的自相矛盾（F5）。 */}
        {todayCheckedIn === true ? (
          <MenuStateItem
            icon={<Check className="size-4 text-primary" />}
            hint={t("shared.cardMenu.checkedTodayHint")}
          >
            {t("shared.cardMenu.checkedToday")}
          </MenuStateItem>
        ) : credentialDead ? (
          <MenuStateItem
            icon={<Sparkles />}
            hint={t("shared.cardMenu.needLoginHint")}
            state={t("shared.cardMenu.needLogin")}
          >
            {t("wbAccounts.card.manualCheckin")}
          </MenuStateItem>
        ) : todayCheckedIn === false ? (
          <DropdownMenuItem disabled={featuresDisabled || !onCheckin} onSelect={onCheckin}>
            <CircleCheck />
            {t("wbAccounts.card.manualCheckin")}
          </DropdownMenuItem>
        ) : null}
        {onGrowthTasks && <DropdownMenuItem disabled={featuresDisabled} onSelect={onGrowthTasks}><Sparkles />{t("wbAccounts.dialog.task")}</DropdownMenuItem>}
        {onToggleActive && <DropdownMenuItem disabled={featuresDisabled} onSelect={onToggleActive}><CircleCheck />{t(deactivated ? "wbAccounts.card.reactivateAccount" : "wbAccounts.card.deactivateAccount")}</DropdownMenuItem>}
        {onExport && (
          <DropdownMenuItem onSelect={onExport}>
            <Download />
            {t("shared.cardMenu.export")}
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="text-destructive focus:bg-destructive/5 focus:text-destructive"
          onSelect={onDelete}
        >
          <Trash2 />
          {t("wbAccounts.card.deleteAccount")}
        </DropdownMenuItem>
        <MenuNote>{t("shared.cardMenu.note")}</MenuNote>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** 演示模式下的占位（与卡片一致：菜单整体不可用）。 */
export function AccountMoreMenuDemo() {
  return (
    <DemoAction>
      <Button variant="ghost" size="sm" className="!px-[10px]">
        <Ellipsis className="size-4" />
      </Button>
    </DemoAction>
  );
}
