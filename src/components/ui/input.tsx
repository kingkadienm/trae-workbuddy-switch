import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * 输入框 —— 几何**逐值对齐原型** `.field`（`demo.css:261-273`）。
 *
 * ⚠️ 三处与 shadcn 默认不同，都是原型的形：
 * 1. **字号 12.5px**（不是 shadcn 的 `text-base md:text-sm` = 14px）。
 *    原型全站以 13.5px 为基准，输入框比正文还小一档；用 14px 会比原型大 1.5px。
 * 2. **聚焦环 18%**（`0 0 0 3px color-mix(ring 18%)`，`:273`），
 *    不是 shadcn 的 `ring-ring/50` —— 50% 的环在深色下明显过重。
 * 3. 高度 32px（原型 `.field` 的 `height:32px`），不是 shadcn 的 `h-9`(36px)；
 *    圆角 9px（原型 `border-radius:9px`），不是 `rounded-md`(10px)。
 *
 * ⚠️ **底色不在这里给**：原型的 `--card-2` 底由调用点的容器（搜索框外壳等）提供，
 * 输入框自身透明 —— 与原型「`.field` 有底、`.field input` 透明」的分层一致。
 */
function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "file:text-foreground placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground border-input flex h-8 w-full min-w-0 rounded-[9px] border bg-transparent px-2.5 py-1 text-[12.5px] shadow-xs transition-[color,box-shadow] outline-none file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50",
        /* 原型 `:273` `box-shadow: 0 0 0 3px color-mix(in oklab, var(--ring) 18%, transparent)` */
        "focus-visible:border-ring focus-visible:ring-ring/20 focus-visible:ring-[3px]",
        "aria-invalid:ring-destructive/20 aria-invalid:border-destructive",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
