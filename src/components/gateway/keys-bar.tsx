/**
 * 密钥分区工具条（原型 `keysBar()`）—— **两个类型共用**同一形态。
 *
 * ## 为什么抽出来
 *
 * 独立网关与统一网关的「密钥」分区长得一样：一条工具条（计数 + 说明 + 创建出口）+ 裸表 + 说明段。
 * 改造前独立网关的「创建密钥」只在**空态**出现 —— 有密钥时反而找不到出口。
 * 两个类型各写一遍必然漂移，故只留一份，差异走 `children`（两个调用点的创建动作是不同命令）。
 *
 * ## 版式逐值对齐原型 `.cat-bar`
 *
 * `padding:10px 12px` · `margin-bottom:12px` · `border-radius:10px` · `background:var(--muted)`。
 * 与 `CatalogSourceBar`（模型分区的来源条）**同一套表面** —— 两个分区用同一种「一行工具区」。
 */

import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

interface Props {
  /** 密钥总数（含已吊销）。 */
  total: number;
  /** 启用中的数量。 */
  live: number;
  /** 创建出口。两个调用点的动作不同（独立网关 / 统一网关各一条命令），故走插槽。 */
  children: ReactNode;
  className?: string;
}

export function KeysBar({ total, live, children, className }: Props) {
  const t = useT();
  return (
    <div
      className={cn(
        "mb-3 flex flex-wrap items-center gap-2.5 rounded-[10px] bg-muted px-3 py-2.5",
        className,
      )}
    >
      <Badge variant="secondary" className="rounded-md">
        {t("gateway.keysBar.total", { count: total })}
      </Badge>
      <span className="text-[11.5px] text-muted-foreground">
        {t("gateway.keysBar.hint", { live })}
      </span>
      <div className="grow" />
      {children}
    </div>
  );
}
