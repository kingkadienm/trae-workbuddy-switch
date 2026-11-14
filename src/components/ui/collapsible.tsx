/**
 * 可折叠区块（方案 §5.7：优先 shadcn/Radix 组合；缺组件就补到 `src/components/ui/`，
 * **禁** `details/summary`）。
 *
 * ## 为什么是**零依赖手写**而不是 Radix
 *
 * shadcn 的 `Collapsible` 包 `@radix-ui/react-collapsible`，但本仓库**没有**这条依赖
 * （`package.json` 里只有 alert-dialog / checkbox / dialog / dropdown-menu / label /
 * popover / radio-group / scroll-area / select / separator / slot / switch / tabs /
 * tooltip 这一族），而补依赖要动 `package.json` + lockfile。为守住「只改约定文件」的
 * 约束，这里给一个**语义与 Radix 对齐**的最小实现：
 *
 * - `Collapsible`：受控 / 非受控（`open` / `defaultOpen` / `onOpenChange`）双形态，
 *   在容器上打 `data-state="open|closed"`（Tailwind 的 `data-[state=…]` 钩子照旧可用）；
 * - `CollapsibleTrigger`：真按钮（`type="button"`，Enter / Space 原生可用），
 *   带 `aria-expanded` + `aria-controls`；
 * - `CollapsibleContent`：收起时用 **`hidden`** 真正移出可达性树（不只是视觉隐藏），
 *   带 `data-state`。
 *
 * ⚠️ 刻意**不做** `asChild`：调用点直接给 `className` 即可，避免再引一层 `Slot` 语义。
 * 若日后引入了 `@radix-ui/react-collapsible`，本文件可直接替换为 shadcn 标准实现
 * （对外 API 有意保持一致）。
 */

import * as React from "react";

import { cn } from "@/lib/utils";

interface CollapsibleContextValue {
  open: boolean;
  setOpen: (open: boolean) => void;
  contentId: string;
  triggerId: string;
}

const CollapsibleContext = React.createContext<CollapsibleContextValue | null>(null);

function useCollapsible(component: string): CollapsibleContextValue {
  const ctx = React.useContext(CollapsibleContext);
  if (!ctx) throw new Error(`<${component}> 必须放在 <Collapsible> 内使用。`);
  return ctx;
}

export interface CollapsibleProps {
  /** 受控开合；不传即非受控（配合 `defaultOpen`）。 */
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  className?: string;
  children?: React.ReactNode;
}

function Collapsible({ open, defaultOpen = false, onOpenChange, className, children }: CollapsibleProps) {
  const [uncontrolled, setUncontrolled] = React.useState(defaultOpen);
  const isControlled = open !== undefined;
  const isOpen = isControlled ? open : uncontrolled;
  const baseId = React.useId();

  const setOpen = React.useCallback(
    (next: boolean) => {
      if (!isControlled) setUncontrolled(next);
      onOpenChange?.(next);
    },
    [isControlled, onOpenChange],
  );

  const ctx = React.useMemo<CollapsibleContextValue>(
    () => ({ open: isOpen, setOpen, contentId: `${baseId}-content`, triggerId: `${baseId}-trigger` }),
    [isOpen, setOpen, baseId],
  );

  return (
    <CollapsibleContext.Provider value={ctx}>
      <div data-slot="collapsible" data-state={isOpen ? "open" : "closed"} className={cn(className)}>
        {children}
      </div>
    </CollapsibleContext.Provider>
  );
}

function CollapsibleTrigger({ className, onClick, ...props }: React.ComponentProps<"button">) {
  const ctx = useCollapsible("CollapsibleTrigger");
  return (
    <button
      type="button"
      data-slot="collapsible-trigger"
      data-state={ctx.open ? "open" : "closed"}
      id={ctx.triggerId}
      aria-expanded={ctx.open}
      aria-controls={ctx.contentId}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) ctx.setOpen(!ctx.open);
      }}
      className={cn("cursor-pointer", className)}
      {...props}
    />
  );
}

function CollapsibleContent({ className, ...props }: React.ComponentProps<"div">) {
  const ctx = useCollapsible("CollapsibleContent");
  return (
    <div
      data-slot="collapsible-content"
      data-state={ctx.open ? "open" : "closed"}
      id={ctx.contentId}
      hidden={!ctx.open}
      className={cn(className)}
      {...props}
    />
  );
}

export { Collapsible, CollapsibleContent, CollapsibleTrigger };
