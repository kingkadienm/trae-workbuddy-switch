import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { cn } from "@/lib/utils";

function TooltipProvider({ delayDuration = 0, ...props }: React.ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider data-slot="tooltip-provider" delayDuration={delayDuration} {...props} />;
}

function Tooltip({ ...props }: React.ComponentProps<typeof TooltipPrimitive.Root>) {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />;
}

function TooltipTrigger({ ...props }: React.ComponentProps<typeof TooltipPrimitive.Trigger>) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />;
}

/**
 * Tooltip 内容浮层。
 *
 * ## 为什么默认带「视口内限高 + 可滚动」（issue #79）
 *
 * Radix 的 tooltip **既不自己限高、也不自己裁切**：内容比可视空间高时，
 * 它只是照原样摆出去，超出窗口的部分被浏览器直接切掉 —— 用户看到的是「说到一半没了」，
 * 而且**没有滚动条**（`overflow` 是 `visible`，滚不动也拖不动）。
 * 账号卡「最近 N 积分 · X 天后过期」的逐批明细就是典型：几十个到期批次，
 * `side="top"` 撑出窗口上沿，**最关键的最近几批**正好在被切掉的那一头。
 *
 * 修法只用一个 Radix 已经算好的量：popper 的 `size` 中间件会把**当前方向上的可用空间**
 * 写进 `--radix-popper-available-height`（挂在内容元素自身上，`collisionPadding` 已扣掉），
 * 拿它当 `max-height` ⇒ 永远不会越过视口，超出的部分变成**内部滚动**。
 * 变量尚未写入时（首帧）该声明无效 ⇒ 退回 `max-height: none`，行为与本改动前一致。
 *
 * ⚠️ 不要改成给内容包一层 `overflow` 容器：那会多出一层内层滚动区，
 * 且 `sticky` 标题（逐批明细里用来钉住「共 N 笔」）会失去参照的滚动容器。
 */
function TooltipContent({
  className,
  sideOffset = 4,
  /* 默认 0 会让浮层贴到窗口边缘；留 8px 既好看，也让上面算出的可用高度自带这段余量。 */
  collisionPadding = 8,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        collisionPadding={collisionPadding}
        className={cn(
          /* 退出动画刻意不加（issue #80）：`Presence` 等 `animationend` 才卸载，
             该事件在部分引擎上不保证送达 ⇒ 提示关不掉、幽灵节点留在 DOM 里。
             进入动画保留（挂载不等待事件）。 */
          "z-50 w-fit max-h-[var(--radix-popper-available-height)] overflow-y-auto overscroll-contain rounded-md bg-popover px-3 py-2 text-xs leading-5 text-popover-foreground shadow-lg ring-1 ring-black/5 animate-in fade-in-0 zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=top]:slide-in-from-bottom-2",
          className,
        )}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider };
