import * as React from "react";
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { Check, Minus } from "lucide-react";

import { cn } from "@/lib/utils";

/**
 * 勾选框（shadcn `Checkbox`，基于 `@radix-ui/react-checkbox`）。
 *
 * ## 为什么用 Radix 而不是原生 `<input type="checkbox">`
 *
 * 原生勾选框在 Tauri WebView 里由系统绘制，无法跟随本仓 token 主题，尺寸也不受控；
 * 且它没有 `indeterminate` 视觉状态（只能靠 CSS 伪元素硬凑），
 * 而「部分选中」在本仓有真实语义（会话树的部分子项、备份区的三态勾选）。
 * Radix 版本同时给出无障碍语义（`role="checkbox"` + `aria-checked="mixed"` + Space 切换）。
 *
 * ## 为什么用 `Check` / `Minus` 而不是带 `Icon` 后缀的名字
 *
 * 本仓既有组件统一用无后缀图标名（`account-card.tsx` 的 `Check`），保持一致。
 *
 * ## 几何与三态逐值对齐原型 `.check`（`demo.css:315-320 / 638 / 735-736`）
 *
 * ## 尺寸
 * 原型 `17×17`、圆角 `5px`、描边 **`1.5px solid var(--border-strong)`**
 * ⇒ shadcn 默认是 `size-4`(16px) / `rounded-[4px]` / 1px `border-input`，三项全偏。
 *
 * ## ★ 三态（此前**半选态完全没有样式**，是本轮修掉的真缺陷）
 *
 * 原型定义了两个半选视觉：
 * - `.check.part`（`:638`）：底 `primary 45%` + 描边 primary —— 会话树的部分子项被选中；
 * - `.check.mixed`（`:735-736`）：**实底** primary + `::after` 内容 `"–"` —— 备份区的三态勾选，
 *   语义是 `aria-checked="mixed"`（§3.11）。
 *
 * ⚠️此前 `data-[state=checked]` 只覆盖 `checked`，**`indeterminate` 落不到任何底色**
 * ⇒ 「部分选中」渲染成一个空心框，用户完全看不出状态。
 * 且指示器恒为 `<Check>`（✓）—— 半选画勾是错的。
 *
 * ⇒ 半选：**底 primary + `Minus`（短横）**；实选：底 primary + `Check`。
 */
function Checkbox({ className, ...props }: React.ComponentProps<typeof CheckboxPrimitive.Root>) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        /* 17×17 / r5 / 1.5px 描边（原型 `:315-317`） */
        "border-[1.5px] border-border-strong",
        "data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground data-[state=checked]:border-primary",
        /* 半选：实底 primary（原型 `.check.mixed`，`:735`） */
        "data-[state=indeterminate]:bg-primary data-[state=indeterminate]:text-primary-foreground data-[state=indeterminate]:border-primary",
        "focus-visible:border-ring focus-visible:ring-ring/20",
        "aria-invalid:ring-destructive/20 aria-invalid:border-destructive",
        "peer size-[17px] shrink-0 cursor-pointer rounded-[5px] shadow-xs transition-shadow outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        /* 半选画短横「–」、实选画勾「✓」—— 原型 `.check.mixed::after{content:"–"}`（`:736`） */
        className="flex items-center justify-center text-current transition-none"
      >
        {props.checked === "indeterminate" ? (
          <Minus className="size-3" strokeWidth={3} />
        ) : (
          <Check className="size-3.5" strokeWidth={3} />
        )}
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}

export { Checkbox };
