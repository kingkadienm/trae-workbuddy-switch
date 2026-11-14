/**
 * 趋势 / 模型 · **模型占比环形**（recharts `PieChart` + `innerRadius`）。
 *
 * ## 契约
 *
 * **扇区数 = 模型数** —— 每个 `ModelSlice` 产出一个扇区（`lib/usage-charts.ts` 保证
 * 只保留 value > 0 的模型；调用方直接传入，本组件不裁剪、不合并「其他」）。
 *
 * ## 无障碍
 *
 * 每段 `<title>`（recharts 的 `Pie` 会把 `name` 放进 `<title>`），并配图例。
 */

import { Cell, Pie, PieChart } from "recharts";

import { ChartContainer, type ChartConfig } from "@/components/ui/chart";
import { useT } from "@/lib/i18n";
import type { ModelSlice } from "@/lib/usage-charts";

import { compactTokens } from "./format";
import { seriesColor } from "./palette";

export function ModelDonut({ slices, size = 170 }: { slices: ModelSlice[]; size?: number }) {
  const t = useT();
  if (slices.length === 0) return null;

  const data = slices.map((slice, index) => ({
    name: slice.name,
    value: slice.value,
    fill: seriesColor(index),
  }));
  const total = data.reduce((sum, item) => sum + item.value, 0);
  const config: ChartConfig = {};
  data.forEach((item) => {
    config[item.name] = { label: item.name, color: item.fill };
  });

  /**
   * ★ **半径必须由 `size` 推导，不能写死。**
   *
   * 曾经写死 `innerRadius={52}` / `outerRadius={78}`（按默认 `size = 170` 调的）——
   * 而调用方普遍传 `size = 150`（`TokenStatsPage` 的两个账本环形、`TraeCreditsPage` 的两个
   * 环形）。170 够放 156 的圆，150 放不下 ⇒ 圆比 `<svg>` 大，SVG 默认 `overflow: hidden`
   * ⇒ 上下左右各切掉 3px，**环形的顶部和底部被削平**（issue 报障的截图形态）。
   *
   * 留白 = `6px`（`strokeWidth = 2` 外沿多占 1px，再留 5px 吸收 `ResponsiveContainer`
   * 量出的亚像素误差）；内半径沿用原比例 0.66（52/78）。默认 `size = 170` 时外半径
   * 79 / 内半径 52，与改造前（78 / 52）几乎一致 —— 大尺寸的观感不变。
   */
  const outerRadius = Math.round(size / 2 - 6);
  const innerRadius = Math.round(outerRadius * 0.66);

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-4">
      <div className="relative shrink-0">
        <ChartContainer config={config} className="shrink-0" style={{ width: size, height: size }}>
          <PieChart>
            <Pie
              data={data}
              dataKey="value"
              nameKey="name"
              innerRadius={innerRadius}
              outerRadius={outerRadius}
              stroke="var(--background)"
              strokeWidth={2}
              isAnimationActive={false}
            >
              {data.map((item) => (
                <Cell key={item.name} fill={item.fill} />
              ))}
            </Pie>
          </PieChart>
        </ChartContainer>
        <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
          <span className="text-sm font-semibold tabular-nums">{compactTokens(total)}</span>
          <span className="text-[10px] text-muted-foreground">{t("wbStats.usage.trend.totalSegment")}</span>
        </div>
      </div>
      <div className="min-w-[160px] flex-1 space-y-1.5">
        {data.map((item) => (
          <div key={item.name} className="flex min-w-0 items-center gap-2 text-xs">
            <span className="size-2.5 shrink-0 rounded-[3px]" style={{ backgroundColor: item.fill }} aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate" title={item.name}>
              {item.name}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
