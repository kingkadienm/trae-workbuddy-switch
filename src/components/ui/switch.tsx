import * as React from "react";
import * as SwitchPrimitive from "@radix-ui/react-switch";

import { cn } from "@/lib/utils";

/**
 * 开关 —— 逐值对齐原型 `.switch`（`demo.css:387-396`）。
 *
 * ⚠️ 与 shadcn 默认的四处差异（原型值在括号里）：
 * 1. 尺寸 `38×22`（不是 `h-5 w-9` = 36×20）—— 差2px 但开关是全站高频控件，肉眼可辨；
 * 2. 轨道底色用 **`--muted` + `--border` 描边**（不是 `--input`）；
 *    ⚠️ 深色下 `--muted` 与 `--card` 同值（`index.css`），用 `--input` 会让关闭态的开关
 *    几乎消失在卡片里 —— 原型正是用 `--muted` 描边来保证「关闭也看得出是个开关」；
 * 3. 滑块是 **`--foreground` + `opacity:.75`**（不是 `--background`）——
 *    前景色带透明度，视觉上比纯色滑块更「沉」；
 * 4. 开启时滑块变 `--primary-foreground` 且 `opacity:1`（原型 `:396`）。
 */
function Switch({ className, ...props }: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        "peer inline-flex h-[22px] w-[38px] shrink-0 cursor-pointer items-center rounded-full border p-0.5 shadow-xs transition-all outline-none",
        "data-[state=checked]:border-transparent data-[state=checked]:bg-primary",
        /* 原型 `:388-389`：关闭态 `--muted` 底 + `--border` 描边 */
        "data-[state=unchecked]:border-border data-[state=unchecked]:bg-muted",
        "focus-visible:border-ring focus-visible:ring-ring/20 focus-visible:ring-[3px]",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={cn(
          /* 原型 `:392-394`：16px 圆点、`top/left:2px`、`--foreground`、`opacity:.75` */
          "pointer-events-none block size-4 rounded-full bg-foreground opacity-75 ring-0 transition-transform",
          "data-[state=checked]:translate-x-4 data-[state=checked]:bg-primary-foreground data-[state=checked]:opacity-100",
          "data-[state=unchecked]:translate-x-0",
        )}
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
