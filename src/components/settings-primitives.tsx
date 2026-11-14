import type { ReactElement, ReactNode } from "react";

import { DemoAction } from "@/components/demo-action";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

/**
 * 设置页的三个行原语（分组 / 普通行 / 字段行）。
 *
 * ## 为什么要抽出来
 *
 * 这两个产品的设置页要求「视觉上无法区分」，而本模块原先在
 * `SettingsPage.tsx` 与 `TraeSettingsPage.tsx` 里各有一份**逐字重复**的实现。
 * 重复的代价不是多敲几行，而是**漂移**：Trae 那份当时已经漏掉了 `operational`
 * 参数，于是同一个「需要演示模式遮罩的开关行」在两个页面表现不同。
 * 抽成共享模块后，两边只可能是同一套间距与断点。
 *
 * ## 边界
 *
 * 这里只放**纯版式**原语，不含任何数据获取与业务判断 —— 两个设置页的内容
 * 差异极大（Trae 有客户端探测/设备标识/平台能力，WorkBuddy 有外观/权限/更新），
 * 强行合并内容只会得到一堆 `if (product === …)`。骨架统一、内容各自实现。
 *
 * ## 逐值对齐原型（`demo.css:210-220 / 382-386`）
 *
 * | 原型 | 本仓 | 说明 |
 * |:---|:---|:---|
 * | `.card { background:var(--card); border:1px solid var(--border); border-radius:var(--radius) }` | `Card` 默认 | `rounded-xl` 即 `--radius`=12px |
 * | `.card-head { padding:13px 16px; border-bottom:1px solid var(--border) }` | `px-4 py-[13px] border-b border-border` | **标题在卡片内部** |
 * | `.card-head h3 { font-size:13.5px; font-weight:600 }` | `text-[13.5px] font-semibold` | |
 * | `.srow { gap:14px; padding:13px 16px; border-bottom }` | `gap-3.5 px-4 py-[13px] border-b border-border` | |
 * | `.srow:last-child { border-bottom:0 }` | `last:border-b-0` | 不再由调用点手写 `border-b-0` |
 * | `.srow .lab b { font-size:12.5px; font-weight:550 }` | `text-[12.5px] font-[550]` | |
 * | `.srow .lab span { font-size:11.5px; color:var(--muted-fg); margin-top:2px }` | `mt-0.5 text-[11.5px] text-muted-foreground` | 不降不透明度 |
 *
 * ⚠️ **曾经的两处偏离**（本轮修掉）：
 * 1. **分组标题写在卡片外面**（`section` 里再放一个 `h2`）—— 原型是卡片内的
 *    `.card-head`。卡外标题会额外占掉「标题行高 + 间距」× 分区数，
 *    这正是设置页内容高度 `803 vs 503` 的主要来源。
 * 2. **行边框靠 `mx-4` 缩进**（`px-0` + 左右 margin）—— 原型的 `border-bottom`
 *    铺满整行，缩进来自 `padding`。用 margin 模拟会让分隔线比原型短 32px。
 */

interface SettingsGroupProps {
  id: string;
  title: string;
  children: ReactNode;
  /**
   * 版式变体。
   * - `card`（默认）：`<section class="card">` + `.card-head` —— 设置页用。
   * - `plain`：`.dsec` + `h4` —— 顶栏 ⚙「应用设置」抽屉用。
   *
   * ★ 用参数而不是各写一份组件：这两个出口的**内容完全相同**，只是外壳不同；
   * 复制一份必然漂移（本模块存在的理由）。
   */
  variant?: "card" | "plain";
}

/**
 * **抽屉形态**的分区（原型 `.dsec`，`demo.css:490-491`）：`flex-col` + `gap:9px`，标题 `h4` = 11.5px/550/muted。
 *
 * 与 `SettingsGroup` 的区别是**没有卡片外壳**：抽屉里不摆卡 —— 原型点顶栏齿轮
 * （`buddy-switch-prototype.html:4455-4461`）打开的就是「一个 `.dsec` + 若干 `.srow`」，
 * 行直接贴在抽屉底色上。
 *
 * ⚠️ 行必须是本容器的**直接子级**，`gap:9px` 才是行距（原型如此）。中间夹一层
 * `display:block` 的包装（如 `CardContent`）会让行距塌成 0、只剩分隔线。
 */
export function SettingsPlainSection({ id, title, children }: SettingsGroupProps) {
  return (
    <section className="flex min-w-0 flex-col gap-[9px]" aria-labelledby={id}>
      <h4 id={id} className="text-[11.5px] font-[550] leading-4 text-muted-foreground">
        {title}
      </h4>
      {children}
    </section>
  );
}

/** 设置分组：卡片内一条 `.card-head` 标题 + 内容（原型 `viewSettings` 的 `<section class="card">`）。 */
export function SettingsGroup({ id, title, children, variant = "card" }: SettingsGroupProps) {
  if (variant === "plain") {
    return (
      <SettingsPlainSection id={id} title={title}>
        {children}
      </SettingsPlainSection>
    );
  }
  return (
    <section className="min-w-0" aria-labelledby={id}>
      <Card className="min-w-0 gap-0 overflow-hidden rounded-xl py-0 shadow-none">
        <div className="flex items-center gap-2.5 border-b border-border px-4 py-[13px]">
          <h2 id={id} className="text-[13.5px] font-semibold leading-5">
            {title}
          </h2>
        </div>
        {children}
      </Card>
    </section>
  );
}

/** 设置行：左右两栏的通用容器（无标签结构时用）。 */
export function SettingsRow({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "flex min-w-0 items-center justify-between gap-3.5 border-b border-border px-4 py-[13px] last:border-b-0",
        className,
      )}
    >
      {children}
    </div>
  );
}

interface SettingsFieldRowProps {
  label: ReactNode;
  description?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
  className?: string;
  /**
   * 该行的控件是「有副作用的操作」（需要演示模式遮罩）时置 true。
   *
   * 用参数而不是让调用方自己包 `<DemoAction>`：遮罩必须与行的右栏宽度绑定
   * （`w-full sm:w-auto`），调用方自己包极容易漏掉宽度而让按钮在窄屏溢出。
   */
  operational?: boolean;
  /**
   * 强制**纵向堆叠**（标签在上、控件在下），不吃 `sm:` 断点。
   *
   * ⚠️ 默认的 `sm:flex-row` 是**视口**断点：在 460px 的顶栏抽屉里，视口仍是 1440px
   * ⇒ 判成「够宽」⇒ 右栏控件（如 `sm:w-80` 的输入框）把左栏标签挤成一列单字
   * （实测「更新代理地址」被拆成「更新代理地 / 址」）。抽屉形态必须传 `true`。
   */
  stacked?: boolean;
}

/** 设置字段行：左标签（可带说明）+ 右控件。 */
export function SettingsFieldRow({
  label,
  description,
  htmlFor,
  children,
  className,
  operational = false,
  stacked = false,
}: SettingsFieldRowProps) {
  return (
    <SettingsRow
      className={cn(
        "flex-col items-stretch gap-2",
        !stacked && "sm:flex-row sm:items-center",
        className,
      )}
    >
      <div className="min-w-0 flex-1">
        {htmlFor ? (
          <Label htmlFor={htmlFor} className="text-[12.5px] font-[550] leading-4">
            {label}
          </Label>
        ) : (
          <div className="text-[12.5px] font-[550] leading-4">{label}</div>
        )}
        {description && (
          <p className="mt-0.5 text-[11.5px] leading-4 text-muted-foreground">{description}</p>
        )}
      </div>
      <div className={cn("flex min-w-0 shrink-0 justify-end", stacked ? "w-full" : "w-full sm:w-auto")}>
        {operational ? (
          <DemoAction className={stacked ? "w-full" : "w-full sm:w-auto"}>{children as ReactElement}</DemoAction>
        ) : (
          children
        )}
      </div>
    </SettingsRow>
  );
}
