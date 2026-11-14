/**
 * 「添加账号」抽屉 —— 逐值对齐原型 `drawerAdd()`
 *（`buddy-switch-prototype.html:3708-3734`、样式 `demo.css:509-519 / 481-495 / 538-547`）。
 *
 * ## 为什么必须有这一层（而不是让「添加账号」直接开 OAuth 对话框）
 *
 * 原型点工具栏「+ 添加账号」是 `case "add-account": stop(); drawerAdd();`（`:4235`）——
 * **先开一个抽屉**，各种方式（OAuth 登录 / 从本机导入 / 导入备份文件）各占一张
 * `.opt` 可点卡。本仓此前**跳过**了这一层，`+ 添加账号` 直接 `setOauthOpen(true)`。
 *
 * ## ★ 本抽屉同时是工具栏「⋯」溢出菜单的**唯一落点**（2026-10-12 用户要求）
 *
 * 原先工具栏右侧挂着一枚「⋯」，把「平台专属控件 + 次级操作」共 8 个入口塞进下拉里。
 * 用户要求「**合并到添加账号的右侧弹出栏内**，外面那枚菜单不用存在了」⇒ 现在：
 *
 * - 工具栏回到**只有** 搜索 / 排序 / 分组 / 视图 / 全选 / 主操作（与原型逐控件一致，「⋯」消失）；
 * - 抽屉内 = ① 添加方式卡（基础三张 + 平台专属 `extraOptions`）
 *   ② 「更多操作」分区（`moreTools`：平台开关 + 不属「添加」的动作）。
 * - **已经存在的功能不重复**：原先溢出菜单里的「从本机导入 / 导入备份」本就是本抽屉的前两张卡
 *   ⇒ 直接不搬，只搬抽屉里没有的（导出备份 / 各平台开关 / 回收站 / 刷新）。
 *
 * ⇒ 两平台的「添加账号」入口**同一个组件、同一套结构**，差异只有数据
 *（`extraOptions` / `moreTools` 由页面注入），与 `account-toolbar.tsx` 同一条纪律。
 *
 * ## 几何（逐值对齐）
 *
 * -抽屉：复用 `ui/sheet.tsx`（本轮已对齐为 `bg-card` / `w-3/4 sm:max-w-[460px]`），
 *   即原型 `.drawer` 的 `width:min(460px,94vw)`（`demo.css:483`）。
 * - 作用域副标题：原型是 `.tiny` = **11px**（`demo.css:550`），作用域名用 `<b>` 加粗。
 * - `.opt`（`demo.css:509-519`）：`flex items-center gap:11px`、`padding:12px 13px`、
 *   `rounded-[11px]`、`border` + `bg-card-2`；hover 换 `--accent` 底 + `--border-strong` 描边。
 *   图标块 `32×32 rounded-[9px] bg-muted`；标题 `12.5px/550`；说明 `11px muted`；
 *   右侧 `ArrowRight`（继承全局 `svg 16px`，原型未给单独尺寸）。
 * - 「更多操作」行与 `.opt` **同几何**（图标块 + 文字 + 右侧控件），只是右侧控件换成
 *   开关 / 图标钮 —— 视觉上仍是同一列卡，不引入第二套行样式。
 * - 未装客户端提示：原型是 `warning` 描边卡（`demo.css:3730-3732`），这里同样式。
 */

import type { LucideIcon } from "lucide-react";
import { ArrowRight, Download, LogIn, TriangleAlert, Upload } from "lucide-react";
import type { ReactNode } from "react";

import { DemoAction } from "@/components/demo-action";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { cn } from "@/lib/utils";

export type AddAccountWay = "oauth" | "local" | "backup";

/** 一张「添加方式」卡（平台专属的那些经 `extraOptions` 注入）。 */
export interface DrawerOption {
  key: string;
  icon: LucideIcon;
  /** 已 i18n。 */
  title: string;
  /** 已 i18n。 */
  desc: string;
  disabled?: boolean;
  onClick: () => void;
}

/**
 * 「更多操作」里的一行（原工具栏「⋯」菜单的内容）。两种形态，**二选一**：
 *
 * - **开关行**（`control`）：左侧图标块 + 文字（`<label htmlFor>`）+ 右侧控件；
 * - **动作行**（`onClick`）：整行可点，与上面的 `.opt` 卡**同形**（右侧一枚 `ArrowRight`）——
 *   这样「更多操作」里的动作与「添加方式」在视觉上是同一列卡，不引入第二套行样式。
 */
export interface DrawerTool {
  key: string;
  icon: LucideIcon;
  /** 已 i18n。 */
  label: string;
  /** 已 i18n；可选说明，显示在 `label` 下方。 */
  hint?: string;
  /** 开关行：文字 `<label htmlFor>` 与控件关联（**必须**给，否则点文字不生效）。 */
  htmlFor?: string;
  /** 开关行：右侧控件（`Switch` 及其保存中指示）。 */
  control?: ReactNode;
  /** 动作行：整行可点。与 `control` 二选一。 */
  onClick?: () => void;
  /** 动作行是否禁用（如「没有账号可导出」）。 */
  disabled?: boolean;
}

export interface AddAccountDrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 作用域显示名（如「WorkBuddy 国际版」），用于副标题。 */
  scopeLabel: string;
  /** 当前作用域是否检测到客户端 —— 未检测到时禁用「从本机导入」并给提示。 */
  installed: boolean;
  onPick: (way: AddAccountWay) => void;
  /**
   * ★ 实际渲染的**基础方式**子集（缺省 = 原型三张全出）。
   *
   * 平台能力子集：某方式尚未落地的平台**不渲染对应卡** —— 摆一张点下去必然失败的卡，
   * 与「禁用按钮不给解释」一样糟（与 `account-batch-bar`「缺哪个就不渲染哪个」
   * 同一条纪律）。⚠️ 收窄前先确认该方式在**该平台的后端命令链**里真的不存在：
   * Qoder 侧「OAuth 设备码 / 导入备份」都尚未落地 ⇒ 只留 `["local"]`，
   * 「粘贴 PAT」走 `extraOptions`（那是已落地的真实命令）。
   */
  ways?: AddAccountWay[];
  /** 平台专属的**额外**添加方式（如 Trae 的「粘贴 JWT」），接在基础三张卡之后。 */
  extraOptions?: DrawerOption[];
  /** 「更多操作」分区内容（平台开关 + 不属「添加」的动作）。 */
  moreTools?: DrawerTool[];
  labels: {
    /** 副标题前半句，作用域名之后紧接（原型：「当前作用域：」）。 */
    scopeHintPrefix: string;
    /** 副标题后半句（原型：「。选择一种方式添加账号。」）。 */
    scopeHintSuffix: string;
    oauthTitle: string;
    oauthDesc: string;
    localTitle: string;
    localDesc: string;
    backupTitle: string;
    backupDesc: string;
    notInstalled: string;
    /** 「更多操作」分区标题（有 `moreTools` 时必传）。 */
    moreTools: string;
    title: string;
  };
}

/** 单个 `.opt` 可点卡（原型 `demo.css:509-519`）。 */
function Option({
  icon: Icon,
  title,
  desc,
  disabled,
  onClick,
}: {
  icon: LucideIcon;
  title: string;
  desc: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-[11px] border border-border bg-card-2 px-3 py-3 text-left transition-all",
        "hover:border-border-strong hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50",
      )}
    >
      {/* 32×32 / r9 / 底 --muted（原型 `.opt .ico`，`demo.css:515`） */}
      <span className="grid size-8 shrink-0 place-items-center rounded-[9px] bg-muted text-muted-foreground">
        <Icon className="size-4" aria-hidden="true" />
      </span>
      {/* `.t b` 12.5px/550 · `.t span` 11px muted（`demo.css:518-519`） */}
      <span className="min-w-0 flex-1">
        <b className="block text-[12.5px] font-[550] leading-tight">{title}</b>
        <span className="mt-0.5 block text-[11px] leading-snug text-muted-foreground">{desc}</span>
      </span>
      <ArrowRight className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
    </button>
  );
}

/** 「更多操作」里的一行：开关行与 `.opt` 同几何，动作行**就是** `.opt` 的形状。 */
function ToolRow({ icon: Icon, label, hint, htmlFor, control, onClick, disabled }: Omit<DrawerTool, "key">) {
  /* 图标块 + 两行文字：两种形态共用（`12.5px/550` + `11px muted`，与 `.opt` 逐值一致）。 */
  const body = (
    <>
      <span className="grid size-8 shrink-0 place-items-center rounded-[9px] bg-muted text-muted-foreground">
        <Icon className="size-4" aria-hidden="true" />
      </span>
      {/* ⚠️ `<label>` 只包文字、**不包控件** —— 包住的话点开关会触发两次（label 转发 + 控件自身）。 */}
      <span className="min-w-0 flex-1">
        <b className="block text-[12.5px] font-[550] leading-tight">{label}</b>
        {hint ? (
          <span className="mt-0.5 block text-[11px] leading-snug text-muted-foreground">{hint}</span>
        ) : null}
      </span>
    </>
  );

  if (onClick) {
    /* 动作行：与 `.opt` 同形（整行可点 + 右侧 `ArrowRight`）。
       演示构建下由 `DemoAction` 统一「看得见但点不动 + 说明原因」，与旧工具栏行为一致。 */
    return (
      <DemoAction>
        <button
          type="button"
          disabled={disabled}
          onClick={onClick}
          className={cn(
            "flex w-full items-center gap-2.5 rounded-[11px] border border-border bg-card-2 px-3 py-2.5 text-left transition-all",
            "hover:border-border-strong hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50",
          )}
        >
          {body}
          <ArrowRight className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        </button>
      </DemoAction>
    );
  }

  return (
    <div className="flex items-center gap-2.5 rounded-[11px] border border-border bg-card-2 px-3 py-2.5">
      <label htmlFor={htmlFor} className={cn("flex min-w-0 flex-1 items-center gap-2.5", htmlFor && "cursor-pointer")}>
        {body}
      </label>
      {control}
    </div>
  );
}

export function AddAccountDrawer({
  open,
  onOpenChange,
  scopeLabel,
  installed,
  onPick,
  ways = ["oauth", "local", "backup"],
  extraOptions,
  moreTools,
  labels,
}: AddAccountDrawerProps) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" showCloseButton={false} className="w-full sm:max-w-[460px]">
        <SheetHeader className="border-b border-border px-4 py-3.5">
          {/* 原型抽屉标题走 `openDrawer(body, "添加账号")`，标题即「添加账号」 */}
          <SheetTitle className="text-[14px] font-semibold">{labels.title}</SheetTitle>
          {/* 原型 `drawerAdd()`（`:3712`）：「当前作用域：`<b>`作用域名</b>`。选择一种方式添加账号。」
              `.tiny` = 11px（`demo.css:550`），作用域名用 `<b>` 加粗。
              ⇒ 文案拆成前后两半拼，作用域名单独渲染（不能整体丢给 t() 替换）。 */}
          <SheetDescription className="text-[11px] leading-relaxed">
            {labels.scopeHintPrefix}
            <b className="text-foreground">{scopeLabel}</b>
            {labels.scopeHintSuffix}
          </SheetDescription>
        </SheetHeader>

        {/* `.dsec` = `flex flex-col gap:9px`（`demo.css:490`） */}
        <div className="flex flex-col gap-2.5 overflow-y-auto p-4">
          <div className="flex flex-col gap-2.5">
            {/* 基础三张卡按 `ways` 收窄（缺省全出）；平台尚未落地的方式**不渲染**（见 Props）。 */}
            {ways.includes("oauth") && (
              <Option
                icon={LogIn}
                title={labels.oauthTitle}
                desc={labels.oauthDesc}
                onClick={() => {
                  onOpenChange(false);
                  onPick("oauth");
                }}
              />
            )}
            {ways.includes("local") && (
              /* ⚠️ **不置灰**（issue #62）：禁用既不给原因也不给替代，用户只会看到
                 「点不动」。没装客户端时它仍然可点，由页面回一句「改用哪两条路」。 */
              <Option
                icon={Upload}
                title={labels.localTitle}
                desc={installed ? labels.localDesc : labels.notInstalled}
                onClick={() => {
                  onOpenChange(false);
                  onPick("local");
                }}
              />
            )}
            {ways.includes("backup") && (
              <Option
                icon={Download}
                title={labels.backupTitle}
                desc={labels.backupDesc}
                onClick={() => {
                  onOpenChange(false);
                  onPick("backup");
                }}
              />
            )}
            {/* 平台专属的额外方式（Trae：粘贴 JWT 兜底）—— 由页面注入，壳内不含平台判断。 */}
            {extraOptions?.map((option) => (
              <Option
                key={option.key}
                icon={option.icon}
                title={option.title}
                desc={option.desc}
                disabled={option.disabled}
                onClick={() => {
                  onOpenChange(false);
                  option.onClick();
                }}
              />
            ))}
          </div>

          {/* 未装客户端提示（原型 `:3730-3732`）：描边 warning 卡 */}
          {!installed ? (
            <div className="mt-1 flex items-center gap-2 rounded-[10px] border border-warning/40 bg-warning/10 px-3 py-2">
              <TriangleAlert className="size-[15px] shrink-0 text-warning" aria-hidden="true" />
              <span className="text-[11px] leading-relaxed text-warning">{labels.notInstalled}</span>
            </div>
          ) : null}

          {/* ★「更多操作」：原工具栏「⋯」菜单的落点（平台开关 + 不属「添加」的动作）。
              与上面三张卡**同一列同几何**，只把右侧换成控件 —— 不引入第二套行样式。 */}
          {moreTools?.length ? (
            <section className="mt-1 flex flex-col gap-2" aria-label={labels.moreTools}>
              <div className="px-1 text-[11px] font-medium text-muted-foreground">{labels.moreTools}</div>
              {moreTools.map(({ key, ...tool }) => (
                <ToolRow key={key} {...tool} />
              ))}
            </section>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}