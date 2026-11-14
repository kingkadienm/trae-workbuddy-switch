import { ArrowRight } from "lucide-react";
import type { ReactNode } from "react";

import { AppSettingsGroups } from "@/components/app-settings";
import { SettingsPlainSection } from "@/components/settings-primitives";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { useT } from "@/lib/i18n";

/**
 * 顶栏 ⚙「应用设置」抽屉 —— 逐值对齐原型 `openDrawer(...)` 的齿轮分支
 *（`buddy-switch-prototype.html:4455-4461`、样式 `demo.css:476-491`）。
 *
 * ## 为什么必须是**抽屉**（而不是下拉菜单）
 *
 * 原型点 `#app-settings-btn` 是 `openDrawer(body, "应用设置")` —— **右侧抽屉**。
 * 本仓此前用的是 `DropdownMenu` + 四个 Dialog，形态与原型不同（同一个入口，两套交互）。
 * 本组件把它收回原型的形态：点齿轮 ⇒ 抽屉从右侧推入，标题「应用设置」。
 *
 * ## 为什么正文是 `.dsec` + `.srow`（不是卡片）
 *
 * 原型这一层**没有卡片**：`<div class="dsec"><h4>…</h4><div class="srow">…</div>×4</div>`，
 * 行直接贴在抽屉底色上。故走 `AppSettingsGroups variant="plain"` —— 内容与设置页
 * **同一份实现**，只是外壳换成 `.dsec`（见 `settings-primitives.tsx`）。
 *
 * ## 本仓多出来的三个入口
 *
 * 原型的抽屉里只有应用级设置，没有「客户端设置 / Skill 管理 / 捐赠」—— 那三项是本仓
 * 自己的 L0 能力（原型里不存在对应页面）。它们**不能丢**（原先挂在 ⚙ 下拉菜单里），
 * 因此按原型同一套行原语（`.srow`：标签 + 说明 + 右侧控件）另起一个 `.dsec` 区块，
 * 而不是另造一种行样式。与「添加账号」抽屉把工具栏溢出项收进「更多操作」是同一手法。
 *
 * ## 懒挂载
 *
 * Radix 的 Sheet 关闭时**卸载内容** ⇒ 抽屉不开就不取数（`UpdateCard` 会打
 * `getGithubConfig`、`StartupCard` 会打 `getLaunchAtLoginEnabled`），
 * 与原先「Dialog 懒挂载」同性能。
 */

/** `.srow`（`demo.css:382-386`）形状的**整行可点**入口：标签 + 说明 + 右侧箭头。 */
function EntryRow({
  label,
  description,
  onClick,
}: {
  label: string;
  description?: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full min-w-0 items-center gap-3.5 border-b border-border px-4 py-[13px] text-left transition-colors last:border-b-0 hover:bg-accent"
    >
      <span className="min-w-0 flex-1">
        <b className="block text-[12.5px] font-[550] leading-4">{label}</b>
        {description ? (
          <span className="mt-0.5 block text-[11.5px] leading-4 text-muted-foreground">{description}</span>
        ) : null}
      </span>
      <ArrowRight className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
    </button>
  );
}

export interface AppSettingsDrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpenClientSettings: () => void;
  onOpenSkills: () => void;
  onOpenDonate: () => void;
}

export function AppSettingsDrawer({
  open,
  onOpenChange,
  onOpenClientSettings,
  onOpenSkills,
  onOpenDonate,
}: AppSettingsDrawerProps) {
  const t = useT();

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      {/* 原型 `.drawer` = `width:min(460px,94vw)`（`demo.css:483`），与其余抽屉同宽。 */}
      <SheetContent side="right" className="w-full sm:max-w-[460px]" aria-describedby={undefined}>
        {/* 原型 `.drawer-head`：`padding:14px 16px` + `border-bottom`，标题 `14px/600`。 */}
        <SheetHeader className="border-b border-border px-4 py-3.5">
          <SheetTitle className="text-[14px] font-semibold">{t("topbar.appMenu")}</SheetTitle>
        </SheetHeader>

        {/* 原型 `.drawer-body`：`padding:16px` + `gap:16px`。 */}
        <div className="flex flex-col gap-4 overflow-y-auto p-4">
          <AppSettingsGroups variant="plain" />

          <SettingsPlainSection id="app-settings-more" title={t("appSettings.drawer.more")}>
            <EntryRow
              label={t("clientSettings.entry")}
              description={t("clientSettings.description")}
              onClick={onOpenClientSettings}
            />
            <EntryRow
              label={t("skillManager.entry")}
              description={t("skillManager.description")}
              onClick={onOpenSkills}
            />
            <EntryRow
              label={t("wbSettings.donate.trigger")}
              description={t("wbSettings.donate.desc")}
              onClick={onOpenDonate}
            />
          </SettingsPlainSection>
        </div>
      </SheetContent>
    </Sheet>
  );
}
