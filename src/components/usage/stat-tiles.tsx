/**
 * 用量四视图 · **统计 tile**（P3）。
 *
 * ## 为什么收敛到一处
 *
 * 各视图的 tile 行形态完全一致（图标 + 标签 + 大号数字 + 可选副标题），固定 4 块；
 * 唯一例外是 WB 用量趋势视图的「网关 Token」tile（口径独立、不与客户端量求和 ⇒ 5 块），
 * 组件按 `tiles.length` 选列数档。
 * 改造前每个页面各写一份（`TokenStatsPage` / `CreditStatsPage` / `Trae*` 里的 `StatMetric`），
 * 四处漂移。这里收成一处，四视图共用。
 *
 * ⚠️ 展示值一律由调用方**先格式化为字符串**（数字格式化 / `displayText` 归一都在调用方），
 * 本组件只渲染字符串 ⇒ 从结构上排除「对象当 React 子节点」的白屏（issue #2）。
 */

import type { LucideIcon } from "lucide-react";

import { Card } from "@/components/ui/card";
import { cn } from "@/lib/utils";

export interface StatTile {
  icon: LucideIcon;
  label: string;
  value: string;
  hint?: string;
  tone?: "default" | "warn" | "up" | "down";
  /**
   * 口径说明（hover 可见）。
   *
   * ⚠️ 与 `hint` 分工：`hint` 是**随数据变**的副标题（对比昨日 / 剩余合计），
   * 而本字段解释「这个数**是什么**」—— 当两个同名指标口径不同时（Q3：概览只算客户端、
   * 账号卡含网关），只有 tooltip 容得下这句解释又不挤掉副标题。
   */
  title?: string;
}

const TONE_CLASS: Record<NonNullable<StatTile["tone"]>, string> = {
  default: "text-foreground",
  warn: "text-warning",
  up: "text-success",
  down: "text-warning",
};

export function StatTiles({
  tiles,
  ariaLabel,
  separated = true,
}: {
  tiles: StatTile[];
  ariaLabel?: string;
  /**
   * `.grid.g4` 的形态：**每块一张独立 `.stat` 卡**（原型 `buddy-switch-prototype.html:2105-2110`
   * / 四视图的 `statTile()` 都是独立卡），而非把四块收进一张带分隔线的卡。
   *
   * ⚠️ **默认 `true`**：原型的用量四视图（趋势 / 模型 / 会话 / 积分）全部是「一排独立统计卡」，
   * 合并成一张带分隔线的卡是改造前的旧形态（与原型不符）。保留 `false` 分支仅作历史兜底。
   */
  separated?: boolean;
}) {
  if (tiles.length === 0) return null;

  /* 概览「一排 N 块」（N = 4，趋势视图 5）：每块一张卡（原型 `.stat`：padding 15px 16px / k 11.5px / v 20px·640 / d 11px）。 */
  if (separated) {
    return (
      <div
        /* `data-grid-cols` 让 `index.css` 的原型断点（1100px→2 列 / 760px→1 列，
           `demo.css:227-228`）能作用到这块。⚠️ 这里的 `sm:`/`lg:` 是 Tailwind 自带的
           640/1024 断点，与原型不同 ⇒ 保留它们做兜底，但**原型值以 data 属性为准**。

           「5」档宿主：趋势视图的网关 Token tile（PR #40 起，与客户端量**不求和**、
           口径独立才单列）。其余视图仍是 4 块 ⇒ 保持 `"4"`，列数宽档随之 4→5。 */
        data-grid-cols={tiles.length >= 5 ? "5" : "4"}
        className={cn(
          "grid min-w-0 grid-cols-1 gap-3 sm:grid-cols-2",
          tiles.length >= 5 ? "lg:grid-cols-5" : "lg:grid-cols-4",
        )}
        aria-label={ariaLabel}
      >
        {tiles.map((tile) => (
          <Card
            key={tile.label}
            className="min-w-0 gap-0 overflow-hidden rounded-lg bg-card px-4 py-[15px] shadow-none"
            title={tile.title}
          >
            <div className="flex min-w-0 items-center gap-1.5 text-[11.5px] text-muted-foreground">
              <tile.icon className="size-3.5 shrink-0 stroke-[1.75]" aria-hidden="true" />
              <span className="truncate">{tile.label}</span>
            </div>
            <div
              className={cn(
                "mt-1.5 max-w-full truncate text-[20px] font-[640] tracking-[-0.02em] tabular-nums",
                TONE_CLASS[tile.tone ?? "default"],
              )}
            >
              {tile.value}
            </div>
            {tile.hint ? (
              /* ⚠️ 刻意**不** `truncate`：账本的合计卡副标题是**算式**
                 （`客户端 187.8M + 网关 655.7K − 重叠 350K`），单行截断会把
                 「− 重叠」砍成「− 重…」，正好砍掉用户最需要核对的那一项 ⇒ 换行显示。 */
              <div className="mt-1 max-w-full break-words text-[11px] leading-snug text-muted-foreground">
                {tile.hint}
              </div>
            ) : null}
          </Card>
        ))}
      </div>
    );
  }

  return (
    <Card
      className="min-w-0 gap-0 overflow-hidden rounded-2xl bg-card py-0 shadow-none"
      aria-label={ariaLabel}
    >
      <div
        className={cn(
          "grid min-w-0 grid-cols-1 divide-y divide-border/60 sm:divide-y-0 sm:py-5",
          tiles.length >= 5
            ? "sm:grid-cols-5"
            : tiles.length >= 4
              ? "sm:grid-cols-4"
              : tiles.length === 3
                ? "sm:grid-cols-3"
                : "sm:grid-cols-2",
        )}
      >
        {tiles.map((tile, index) => (
          <div
            key={tile.label}
            className={cn(
              "flex min-w-0 flex-col items-center justify-center px-4 py-5 text-center sm:py-3",
              index > 0 && "sm:border-l sm:border-border/60",
            )}
          >
            <div className="flex max-w-full items-center justify-center gap-2 text-[13px] font-medium leading-5 text-muted-foreground">
              <tile.icon className="size-4 shrink-0 stroke-[1.75]" aria-hidden="true" />
              <span className="truncate">{tile.label}</span>
            </div>
            <div
              className={cn(
                "mt-3 max-w-full truncate text-[26px] font-semibold leading-8 tracking-[-0.025em] tabular-nums",
                TONE_CLASS[tile.tone ?? "default"],
              )}
            >
              {tile.value}
            </div>
            {tile.hint ? (
              <div className="mt-1.5 max-w-full truncate text-xs text-muted-foreground">
                {tile.hint}
              </div>
            ) : null}
          </div>
        ))}
      </div>
    </Card>
  );
}
