/**
 * 分段控件（原型 `.seg`，`demo.css:275-282`）。
 *
 * ## 为什么收成一个原语
 *
 * 用量页的**视图参数条**上有两个切换器 —— 「近 N 天」与「仅本作用域 / 合并全部作用域」。
 * 它们在原型里是**同一个 `.seg`**，几何必须逐值一致；各写一遍必然漂移
 * （改造前：范围条与合并条一个是 `p-1/rounded-lg`、另一个也是 `p-1/rounded-lg` 但
 * 按钮内边距不同，肉眼就是「两个不像一套的控件」）。
 *
 * 逐值（`demo.css:275-282`）：
 * - 容器 `padding: 2px` · `gap: 2px` · `border-radius: 10px`（= `rounded-md`，
 *   本主题 `--radius-md: calc(var(--radius) - 2px)` = 10px）· `background: var(--muted)`；
 * - 选项 `height: 26px` · `padding: 0 11px` · `border-radius: 8px`（= `rounded-sm`，
 *   `--radius-sm: calc(var(--radius) - 4px)` = 8px）· `font-size: 12px`；
 * - 选中 `font-weight: 550` + `box-shadow: var(--shadow-sm)`。
 *
 * ⚠️ **半径一律走语义类**（`rounded-md` / `rounded-sm`），不写 `rounded-[10px]` ——
 * 后者与主题令牌脱钩，改 `--radius` 时不会跟着走。
 *
 * ## 选中态底色 = `bg-card`（**照抄**原型 `var(--card)`）
 *
 * ⚠️ 本主题深色下 `--card` 与 `--muted` **同值**（`index.css:88` / `:98` 都是
 * `oklch(0.279 0.041 260.031)`）⇒ 选中项在深色下主要靠 `font-weight:550` + `shadow-sm`
 * 区分。这是**原型自带的取舍**，不是本组件的 bug：`Tabs`（`tabs.tsx:15`
 * `data-[state=active]:bg-card`）与账号页工具栏（`account-toolbar.tsx:217`
 * `data-[on=true]:bg-card`）两处已对齐的控件都是这个值。三处必须同值 ——
 * 否则同一个「用量」页里，子 Tab 条与参数条会呈现出两种选中态。
 *
 * ## 色调（`tone`）：账本那一组**不是**默认选中态
 *
 * ⚠️ 原型对「账本」这一枚 `.seg` 单独加了 `.book`（`demo.css:1362`）：
 * `button.on { background: color-mix(in oklab, var(--primary) 18%, transparent); color: var(--primary) }`，
 * 即**青绿 18% 底 + `primary` 文字**（`font-weight:550` 与 `shadow-sm` 由 `.seg button.on` 继承、
 * 未被覆盖）。理由：账本不是「同一件事的档位」，而是**换一本账本**看数据 ——
 * 换账之后整页数字的来源都变了，这一点必须在控件上看得见。
 *
 * 不给 `book` 会怎样：深色主题下 `--card` 与 `--muted` 同值（见下节），
 * 选中项只剩字重差别，用户看不出「我现在看的是哪本账」—— 截图里就是「完全看不出」。
 * 所以**色调不许由调用点拼类名**（那样又会退回两套按钮外观），走 `tone` 参数。
 *
 * ## 与 `Tabs` 的分工
 *
 * `Tabs` 管**内容面板切换**（有 `TabsContent`、有 roving tabindex 语义）；
 * 本原语只管**一个参数取哪一档**（`role="group"` + `aria-pressed`，不承载面板）。
 * 用法用量页的「趋势 / 模型 / 会话 / 积分」仍走 `Tabs`，参数条上的两枚走这里。
 *
 * ★ 唯一例外是 `popup` 那一档（用量页的「自定义时间」）：它**不是取值，而是开浮层**，
 * 于是需要 `aria-haspopup` / `aria-expanded`。仍**留在本原语里**、不交给调用点写按钮 ——
 * 否则同一个 `.seg` 里会出现两套按钮类名。
 */

import { cn } from "@/lib/utils";

export interface SegOption<T extends string | number> {
  value: T;
  label: string;
}

export interface SegProps<T extends string | number> {
  value: T;
  onChange: (next: T) => void;
  options: readonly SegOption<T>[];
  /** 无障碍分组名（读屏会先念它，再念每一档）。 */
  ariaLabel: string;
  disabled?: boolean;
  /**
   * 某一档**不是「直接取值」而是「打开浮层」**时，给它补上 `aria-haspopup` / `aria-expanded`。
   *
   * 目前只有一档需要（用量页的「自定义时间」——它开一个日期弹层）。
   * ⚠️ 刻意**不做成 ReactNode 插槽**：插槽会让调用点自己写按钮类名，
   * 而那正是本原语要消灭的漂移（同一个 `.seg` 里两枚按钮长得不一样）。
   * 少了这两个属性，读屏用户听到的只是一枚普通按钮 —— 不知道它会开浮层、也不知道开没开。
   */
  popup?: { value: T; expanded: boolean };
  /**
   * 色调。缺省 = 原型 `.seg`（选中 `bg-card` + `foreground`）。
   *
   * `book` 专给用量页的「账本」那枚（原型 `.seg.book`，`demo.css:1362`）：
   * 选中态是 `primary 18%` 底 + `primary` 文字 —— 「换了一本账」比「换了一档」重得多，
   * 必须在控件本身上看得出来（深色下 `card` 与 `muted` 同值，默认色调只剩字重差异）。
   */
  tone?: "default" | "book";
  className?: string;
}

export function Seg<T extends string | number>({
  value,
  onChange,
  options,
  ariaLabel,
  disabled = false,
  popup,
  tone = "default",
  className,
}: SegProps<T>) {
  return (
    <div
      className={cn(
        "inline-flex shrink-0 gap-0.5 rounded-md bg-muted p-0.5",
        disabled && "opacity-60",
        className,
      )}
      role="group"
      aria-label={ariaLabel}
    >
      {options.map((option) => {
        const isPopup = popup !== undefined && option.value === popup.value;
        return (
          <button
            key={String(option.value)}
            type="button"
            className={cn(
              "h-[26px] cursor-pointer rounded-sm px-[11px] text-xs transition-colors",
              /* 原型 `.seg button.on` 的三个共有项：550 字重 + `card` 底 + `shadow-sm`；
                 `.seg.book button.on` 只改**底色与文字色**，前两项仍保留（CSS 层叠如此）。 */
              value === option.value
                ? cn(
                    "font-[550] shadow-sm",
                    tone === "book"
                      ? "bg-primary/18 text-primary"
                      : "bg-card text-foreground",
                  )
                : "text-muted-foreground hover:text-foreground",
              disabled && "cursor-not-allowed",
            )}
            onClick={() => onChange(option.value)}
            disabled={disabled}
            aria-haspopup={isPopup ? "dialog" : undefined}
            aria-expanded={isPopup ? popup.expanded : undefined}
            aria-pressed={value === option.value}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
