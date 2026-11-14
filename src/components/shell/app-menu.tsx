import { useState } from "react";
import { Monitor, Moon, MoonStar, Settings, Sun } from "lucide-react";

import { ClientSettingsDialog } from "@/components/client-settings";
import { DonateDialog } from "@/components/donate-dialog";
import { AppSettingsDrawer } from "@/components/shell/app-settings-drawer";
import { SkillManagerDialog } from "@/components/skill-manager";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useT } from "@/lib/i18n";
import {
  getThemePreference,
  setThemePreference,
  THEME_LABEL_KEYS,
  THEME_PREFERENCES,
  type ThemePreference,
} from "@/lib/theme";
import { cn } from "@/lib/utils";

/**
 * 顶栏应用级入口（L0）—— ⚙ **应用设置抽屉**。
 *
 * ## 形态由原型决定：抽屉，不是下拉菜单
 *
 * 原型点 `#app-settings-btn` 走的是 `openDrawer(body, "应用设置")`
 *（`buddy-switch-prototype.html:4455-4461`）—— 右侧抽屉。本组件此前是
 * `DropdownMenu` + 四个 Dialog，同一个入口两套交互；现在收回原型的形态：
 * 齿轮 = 抽屉触发器（唯一真值在 `AppSettingsDrawer`）。
 *
 * ## 为什么这里只做「驱动」，不重写业务
 *
 * 抽屉与三个弹层各是一整域实现（`AppSettingsDrawer` / `ClientSettingsDialog` /
 * `SkillManagerDialog` / `DonateDialog`），本组件只持有它们的 `open` 状态，
 * 并在「从抽屉进弹层」时**先关抽屉**（两层同时开着会让 Esc 只关最上层，
 * 用户以为没反应）。内容均**懒挂载**（Radix 关闭即卸载）⇒ 不开就不取数。
 *
 * ## 为什么主题不在抽屉里
 *
 * 顶栏本就有独立的主题钮（原型 `#theme-btn`），抽屉里再放一份就是同一设置的第二个入口。
 */
export function AppMenu() {
  const t = useT();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [clientOpen, setClientOpen] = useState(false);
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [donateOpen, setDonateOpen] = useState(false);

  /** 抽屉 → 弹层：先关抽屉，再开目标弹层（避免两层叠着）。 */
  const leaveDrawer = (open: (value: boolean) => void) => () => {
    setDrawerOpen(false);
    open(true);
  };

  return (
    <>
      <button
        type="button"
        aria-label={t("topbar.appMenu")}
        title={t("topbar.appMenu")}
        className={iconButtonClass}
        onClick={() => setDrawerOpen(true)}
      >
        <Settings className="size-[17px]" aria-hidden="true" />
      </button>

      <AppSettingsDrawer
        open={drawerOpen}
        onOpenChange={setDrawerOpen}
        onOpenClientSettings={leaveDrawer(setClientOpen)}
        onOpenSkills={leaveDrawer(setSkillsOpen)}
        onOpenDonate={leaveDrawer(setDonateOpen)}
      />

      {/* 三个受控弹层：内容懒挂载，关闭即卸载。 */}
      <ClientSettingsDialog open={clientOpen} onOpenChange={setClientOpen} />
      <SkillManagerDialog open={skillsOpen} onOpenChange={setSkillsOpen} />
      <DonateDialog open={donateOpen} onOpenChange={setDonateOpen} />
    </>
  );
}

/**
 * 四态选项 → 图标（`system` 跟随系统用显示器；`black` 用「月+星」区别于普通 `dark` 的月亮，
 * 两者若同图标，菜单里会出现两个一模一样的月亮、只能靠文字分辨）。
 * 文案键统一走 `theme.ts::THEME_LABEL_KEYS`，与设置页下拉共用同一份。
 */
const THEME_ICONS: Record<ThemePreference, typeof Sun> = {
  system: Monitor,
  light: Sun,
  dark: Moon,
  black: MoonStar,
};

/**
 * 顶栏主题菜单 —— **四态选择**（系统 / 浅色 / 深色 / 纯黑）。
 *
 * ⚠️ **不做二态假开关**：`theme.ts` 的偏好本就多态（`system` 是独立选项，
 * 不是「跟随系统时把按钮按当前系统值显示」）。压成明/暗二态会让「跟随系统」
 * 这一态在界面上无处可表达，用户也关不掉它。
 *
 * 触发器图标跟随当前偏好（而不是固定写死晴天）—— 否则选了纯黑之后按钮仍显示太阳，
 * 看起来像「没生效」。
 */
export function ThemeMenu() {
  const t = useT();
  const [preference, setPreference] = useState<ThemePreference>(getThemePreference);
  const TriggerIcon = THEME_ICONS[preference];

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={t("topbar.theme")}
          title={t("topbar.theme")}
          className={iconButtonClass}
        >
          <TriggerIcon className="size-[17px]" aria-hidden="true" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        {THEME_PREFERENCES.map((key) => {
          const Icon = THEME_ICONS[key];
          return (
            <DropdownMenuItem
              key={key}
              onSelect={() => {
                setThemePreference(key);
                setPreference(key);
              }}
              className={cn(key === preference && "bg-accent")}
            >
              <Icon aria-hidden="true" />
              {t(THEME_LABEL_KEYS[key])}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** 顶栏图标按钮样式（与原型 `.icon-btn` 对齐：34px 方角圆角 + hover 底色）。 */
export const iconButtonClass = cn(
  /* 尺寸对齐原型 `.icon-btn`（`buddy-switch-prototype.html:143-147`）：34×34 / 圆角 10px。 */
  "grid size-[34px] shrink-0 place-items-center rounded-[10px] text-muted-foreground outline-none",
  /* 过渡在原型 `background .14s, color .14s` 之上**多一项 transform**（压反弹所需的缩放）。 */
  "transition-[color,background-color,transform]",
  "hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40",
  /* ★ 按压反馈：按下时轻微缩小。原型静态 demo 表达不了「按下」这一帧，但真机上手
     缺了它，点击**瞬间**零视觉变化 —— 这正是本次要修的「点击没有任何反馈」。 */
  "active:scale-95",
  /* 进行中（如刷新）/ 不可用的禁用态：对齐 `Button` 的 `disabled:opacity-45`。 */
  "disabled:pointer-events-none disabled:opacity-45",
);
