/**
 * 积分 · **到期分布**（recharts `BarChart`，**只画有到期的日子**）。
 *
 * ## 契约（P3-ARCH 契约 10，2026-10-22 修订）
 *
 * x 轴是「有到期的那几个日子」，不是 D+1…D+30 的完整日历。每根柱的刻度都写着 `D+n`
 * ⇒ 「哪一天到期」由**标签**回答，「多密」由**柱高**回答，两者都不丢。
 *
 * ### 为什么改掉了「无到期的天补 0、不跳格」
 *
 * 积分到期天生是**稀疏事件**：30 天里通常只有 3~8 天有到期。把空格全留着时，
 * 实测（1220px 卡片）：柱 35px、**相邻柱之间 4px**，但只要隔一天没到期，两根柱之间就是
 * **122px 的空白**（= 3 个空槽 + 3 条缝）；用户读到的是「一排孤零零的点」，
 * 而且会误以为是**间距没调好**（同页「每日收支」那张 7 天柱图是柱柱相贴的）。
 *
 * 原规则担心的是「D+3 与 D+27 看起来一样密」——那是**没有标签**时才成立的坑。
 * 现在每根柱自带 `D+n` 刻度，日期直接写在柱子底下，所以跳格不再丢信息。
 *
 * ## 空数据
 *
 * 30 天全为 0（没有即将到期的积分）时调用方**不渲染本图**（无数据不渲染）；
 * 本组件兜底返回 `null`。
 *
 * ## 柱间距：固定像素缝、不用 `maxBarSize`（与 `stacked-daily-bar.tsx` 同一套规则）
 *
 * `maxBarSize` 会把柱压窄并在带内居中 ⇒ 剩余宽度全变成空白。规则见
 * `stacked-daily-bar.tsx` 文件头「柱间距」一节。副作用同样是「日子少时柱变胖」——
 * 这是「填满」而非「留缝」的必然结果。
 */

import { Bar, BarChart, CartesianGrid, XAxis, YAxis } from "recharts";

import { ChartContainer, ChartTooltip, ChartTooltipContent } from "@/components/ui/chart";
import { useT } from "@/lib/i18n";
import type { ExpiryBucket } from "@/lib/usage-charts";

import { formatCredits } from "./format";

/** 柱间缝隙的一半（px）：recharts 在带两侧各扣一份 ⇒ 实际缝 4px。 */
const BAR_CATEGORY_GAP_PX = 2;

/**
 * 刻度全显的上限（根）。
 *
 * 柱数 ≤ 此值时 `interval={0}`（每根柱都带 `D+n`），超过才让 recharts 自动抽稀 ——
 * 跳格后「日期只写在部分柱子上」才是真的会误导，所以能全显就全显。
 * 实测 30 根柱在 1180px 绘图区里步长 37px，`D+30` 约 26px ⇒ 30 根也放得下；
 * 留 12 是为了卡片被压窄（半宽布局）时仍不重叠。
 */
const MAX_FULL_LABELS = 12;

export function CreditExpiryBars({ buckets }: { buckets: ExpiryBucket[] }) {
  const t = useT();
  // 只留真的有到期的日子：空日占着槽位会把稀疏数据画成一排孤点（见文件头「为什么改」）。
  const data = buckets
    .filter((bucket) => bucket.value > 0)
    .map((bucket) => ({ label: `D+${bucket.day}`, value: bucket.value }));
  if (data.length === 0) return null;

  return (
    <ChartContainer
      config={{ value: { label: t("wbStats.usage.credits.expiryTitle"), color: "var(--data-series-amber)" } }}
      className="h-[170px] w-full"
    >
      <BarChart
        data={data}
        margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
        barCategoryGap={BAR_CATEGORY_GAP_PX}
      >
        <CartesianGrid vertical={false} strokeDasharray="3 3" />
        <XAxis
          dataKey="label"
          tickLine={false}
          axisLine={false}
          tickMargin={8}
          interval={data.length <= MAX_FULL_LABELS ? 0 : "preserveStartEnd"}
        />
        <YAxis tickLine={false} axisLine={false} width={48} tickFormatter={(value) => formatCredits(Number(value))} />
        <ChartTooltip cursor={{ fill: "var(--muted)", opacity: 0.4 }} content={<ChartTooltipContent indicator="dot" />} />
        <Bar dataKey="value" fill="var(--color-value)" radius={[4, 4, 0, 0]} isAnimationActive={false} />
      </BarChart>
    </ChartContainer>
  );
}
