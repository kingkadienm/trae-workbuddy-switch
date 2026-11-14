/**
 * 模型表格（P11 §1.6）—— WorkBuddy 与 Trae **共用**的表格壳。
 *
 * ## 为什么抽共用壳，而不是两个模型页各写一份
 *
 * 改造前 `model-list.tsx` 把模型渲染成 `flex-wrap` 的弹性格（chips），Trae 侧另写了一份
 * 同构的分组 chips —— **都没有状态列、没有禁用按钮、没有计数**。P11 要给两边都加上
 * 「禁用 / 启用」这一整套交互，若各写一份，四组合的判据（下面那两列）会各漂一份。
 * 沿用 P4 / P10 的分层结论：**能合并的合并，差异走数据**。
 *
 * ## ★ 四组合可区分（P11 的核心）
 *
 * `available`（上游事实）与 `disabled`（用户偏好）是**两个正交维度**，UI 必须能表达全部四种：
 *
 * | available | disabled | 上游状态列（仅 Trae） | 网关暴露列 |
 * |:--:|:--:|:---|:---|
 * | ✓ | ✗ | 可用 | 已暴露 + [禁用] |
 * | ✓ | ✓ | 可用 | 已禁用 + [启用] |
 * | ✗ | ✗ | 不可用 | 已暴露 |
 * | ✗ | ✓ | 不可用 | 已禁用 |
 *
 * ⇒ **两列独立驱动**：上游状态列只看 `row.available`，网关暴露列只看 `row.disabled`。
 * **不得**把两者压成一个字段（压了之后「上游不可用」与「你自己禁用了」就分不清）。
 *
 * ## 列集
 *
 * `模型 / 上下文 / [可选] 图片输入 / [可选] 思考档位 / [可选] 积分倍率 / [仅 Trae] 上游状态 /
 * 网关暴露 / [可选] 附加列`。
 *
 * 能力两列（issue #65）由**可选** prop `effortColumn` / `creditsColumn` 驱动，内容由调用方
 * 用 `model-ability.tsx` 的 `<EffortCell>` / `<CreditsCell>` 注入 —— 三个平台的档位来自三个
 * 不同数据源，壳里**不认平台**。不传 ⇒ 整列不渲染（不做「一列全 `—`」的噪音）。
 *
 * ## 为什么列集里**没有**「归属」列、「协议」列
 *
 * - **无归属列**（决策 #1）：UI 只渲染**当前 region**（页面持有唯一 region）⇒ 全表同一 scope，
 *   归属列是常量、纯冗余；
 * - **无协议列**（决策 #2）：协议是**端点属性**、不是模型属性（同一模型两种协议都支持）⇒ 无数据支撑。
 *
 * ## 为什么「上游状态」列由 `showUpstream` 开关（而不是恒显）
 *
 * WorkBuddy 的 `available` **恒为 `true`**（在清单里即可用，无「非可服务」概念）⇒ 该列永远是
 * 同一个值，就是噪音（§3.4 规则 1「无数据不渲染」/ **决策 #3**）。Trae 有 `servable` 判据，
 * 该列有信息量，故 Trae 传 `showUpstream=true`。
 *
 * ## 硬约束（可证伪）
 *
 * - **每行两个可点区**：**复制模型 ID** + 禁用 / 启用按钮。**无行点击、无勾选**（与
 *   `AccountTable` 不同，故**不复用**它 —— 后者强制勾选 + 打开详情）。
 *   ⚠️ 复制按钮是 2026-10-13 新增的：Codex / Trae 自定义模型 / dsh 自定义 provider
 *   都要求**手填模型 id**，此前表格只把 id 当纯文本 ⇒ 用户只能手抄。
 *   它**不是**「第三个可点区」的开口 —— 再加任何行级交互前先看门禁
 *   `scripts/check-scope-registry.cjs` 的「每行 2 处 onClick」断言。
 * - **标灰仍可见**（D26）：`disabled` 行加 `row-off`（`opacity-60` + 虚边框），**仍渲染** ——
 *   彻底隐藏会让用户以为清单里没有它。
 * - **展示字段一律过 `displayText`**（白屏防线，issue #2）：`name` / `id` 运行时可能是对象，
 *   当 React 子节点渲染会卸载整棵树。
 * - **共用壳内禁止平台判断**：差异一律走 `rows`（数据）+ `showUpstream`（开关）+ `labels`（文案）。
 * - **切换按钮是「按钮操作中」态**（§3.6）：`<Loader2>` 在按钮内 + `disabled` 防重复点击，
 *   **不是**区块加载态（不用 `<Skeleton>`）。
 *
 * ## 为什么自定义组件（AGENTS.md 要求记录理由）
 *
 * 它是**纯展示的组合**（表头 + 行 + 徽标 + 一个按钮），由项目现有 `Badge` / `Button` 原语与
 * `cn()` 拼成，不涉及焦点管理 / 键盘交互，因此不需要 Radix 层。唯一的交互（切换按钮）已复用
 * `Button` 并包在 `DemoAction` 里（与其余网关控件同口径）。
 */

import { useState, type ReactNode } from "react";
import { Ban, Check, Copy, Loader2, RotateCcw } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DemoAction } from "@/components/demo-action";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { copyText } from "@/lib/clipboard";
import { displayText } from "@/lib/display-text";
import { cn } from "@/lib/utils";
import type { GatewayModelRow } from "@/lib/types";

/** 已 i18n 的列头 / 状态文案 / 按钮标签（**无 `colScope`** —— 无归属列，决策 #1）。 */
export interface ModelTableLabels {
  colModel: string;
  /** 上下文窗口列头（如「上下文」/「Context」）。 */
  colContext: string;
  colUpstream: string;
  colExposed: string;
  upstreamAvailable: string;
  upstreamUnavailable: string;
  exposed: string;
  disabled: string;
  disable: string;
  enable: string;
  /** 「复制模型 ID」的按钮名 / 无障碍名。 */
  copyId: string;
  /** 复制成功后的提示（toast 与 tooltip 共用）。 */
  idCopied: string;
}

export interface ModelTableProps {
  rows: GatewayModelRow[];
  /** 正在切换的行 id（驱动按钮 `<Loader2>` + `disabled`）。 */
  pendingId: string | null;
  /** 点「禁用 / 启用」。`disabled` 是**目标态**（`true` = 禁用）。 */
  onToggleDisabled: (row: GatewayModelRow, disabled: boolean) => void;
  /** 是否渲染「上游状态」列 —— WB `false`（恒可用，§3.4 规则 1 / 决策 #3），Trae `true`。 */
  showUpstream: boolean;
  labels: ModelTableLabels;
  /**
   * 可选「思考档位」列（issue #65）。不传 ⇒ 该列不渲染。
   *
   * 内容由调用方注入（`model-ability.tsx` 的 `<EffortCell>`）—— 三个平台的档位来自
   * 不同数据源（WB 目录 / Trae 客户端缓存 / Qoder 官方清单），壳里不认平台。
   */
  effortColumn?: { header: string; render: (row: GatewayModelRow) => ReactNode };
  /**
   * 可选「积分倍率」列（issue #65）。不传 ⇒ 该列不渲染。
   *
   * 与 `effortColumn` 分开而不是塞进同一个「计费」列：倍率是**单值**、档位是**集合**，
   * 挤在一个单元格里既扫不出来、也没法各自给 tooltip 说明来源。
   */
  creditsColumn?: { header: string; render: (row: GatewayModelRow) => ReactNode };
  /**
   * 可选「图片输入」列（issue #101）。不传 ⇒ 该列不渲染。
   *
   * 与 `effortColumn` / `creditsColumn` **同构**：壳里只放表头 + 单元格容器，
   * 内容（徽标 + 覆盖按钮）由调用方用 `model-image-cell.tsx` 注入。
   * 为什么不把交互写进壳里：那样「每行恰好 2 处可点区」的门禁会失守，
   * 而这条约束保护的是「整行可点 / 勾选」这类会抢事件的交互不再溜进来。
   */
  imageColumn?: { header: string; render: (row: GatewayModelRow) => ReactNode };
  /**
   * 可选**附加列**（2026-10-16：限流探测结果折回模型表）。
   *
   * 只有 WB（`model-list.tsx`）传它渲染「限流（探测）」；Trae / Qoder **不传** ⇒ 该列
   * 完全不渲染。内容由调用方以 `render` 注入（那里才有 `onClick`），**本壳内不放任何
   * click handler** ⇒ 保持「每行 2 处可点区」的门禁断言（`check-scope-registry.cjs`）。
   */
  extraColumn?: { header: string; render: (row: GatewayModelRow) => ReactNode };
}

/** 上下文窗口 → 人类可读：200000→`200k`、981504→`982k`、1000000→`1M`；未知给 `—`。 */
function formatContext(value: number | null | undefined): string {
  if (!value || value <= 0) return "—";
  if (value >= 1_000_000) return `${Math.round(value / 100_000) / 10}M`;
  return `${Math.round(value / 1000)}k`;
}

export function ModelTable({
  rows,
  pendingId,
  onToggleDisabled,
  showUpstream,
  labels,
  effortColumn,
  creditsColumn,
  imageColumn,
  extraColumn,
}: ModelTableProps) {
  /** 刚复制成功的行 id（驱动图标 `Copy` → `Check`；1.6s 后复位）。 */
  const [copiedId, setCopiedId] = useState<string | null>(null);

  /** 复制模型 id 到剪贴板。**可反复点**（不是一次性），失败时由 `copyText` 给出可读提示。 */
  async function onCopyId(id: string) {
    await copyText(id, labels.idCopied);
    setCopiedId(id);
    window.setTimeout(() => setCopiedId((current) => (current === id ? null : current)), 1600);
  }

  return (
    /* TooltipProvider 挂在**本组件根**（与 `account-toolbar` 同做法）：本表每行一个
       图标钮，若依赖全局 provider 则单测 / 嵌入场景下 tooltip 会静默失效。
       Radix Provider 不渲染 DOM，包住根节点不改变表格结构。 */
    <TooltipProvider delayDuration={400}>
      <div className="overflow-x-auto">
      <table className="w-full border-collapse text-sm" data-act="gw-model-table">
        <thead>
          <tr className="border-b border-border/60 text-left text-xs text-muted-foreground">
            <th scope="col" className="px-2 py-2 font-medium">
              {labels.colModel}
            </th>
            <th scope="col" className="px-2 py-2 font-medium">
              {labels.colContext}
            </th>
            {/* 能力列排在上下文之后、状态列之前：它们是模型**属性**，
                与「上游状态 / 网关暴露」这两列**状态**分开读。不传 ⇒ 整列不渲染。
                顺序 = 图片输入（能不能读图）→ 思考档位 → 积分倍率（成本）。 */}
            {imageColumn && (
              <th scope="col" className="px-2 py-2 font-medium">
                {imageColumn.header}
              </th>
            )}
            {effortColumn && (
              <th scope="col" className="px-2 py-2 font-medium">
                {effortColumn.header}
              </th>
            )}
            {creditsColumn && (
              <th scope="col" className="px-2 py-2 font-medium">
                {creditsColumn.header}
              </th>
            )}
            {showUpstream && (
              <th scope="col" className="px-2 py-2 font-medium">
                {labels.colUpstream}
              </th>
            )}
            <th scope="col" className="px-2 py-2 font-medium">
              {labels.colExposed}
            </th>
            {extraColumn && (
              <th scope="col" className="px-2 py-2 font-medium">
                {extraColumn.header}
              </th>
            )}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            // 白屏防线：`name` / `id` 运行时可能是对象，一律过 `displayText` 收敛成字符串。
            const id = displayText(row.id) ?? "";
            const name = displayText(row.name) ?? id;
            const pending = pendingId === row.id;
            return (
              <tr
                key={row.id}
                // 验收钩子（与 trae-model-list 的 `data-model` 同口径）：CDP / 断言据此
                // 逐个判「这一行可不可用 / 有没有被禁用」，不必靠 className / 文案反查。
                data-model={id}
                data-available={row.available ? "yes" : "no"}
                data-disabled={row.disabled ? "yes" : "no"}
                className={cn(
                  "border-b border-border/40 last:border-0",
                  // D26：被禁用**可见但标灰**（不彻底隐藏）。
                  row.disabled && "row-off border-dashed opacity-60",
                )}
              >
                <td className="px-2 py-2">
                  <div className="flex items-center gap-1.5">
                    <span className="font-medium">{name}</span>
                    {name !== id && (
                      <code className="font-mono text-[11px] text-muted-foreground">{id}</code>
                    )}
                    {/* 复制模型 ID：图标钮（`size-6` 覆盖 `size="icon"` 的 34px —— `cn` 走
                        tailwind-merge，同组 `size-*` 后者胜）。复制的是 **id**，
                        与旁边的展示名无关（填展示名不生效）。 */}
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="size-6 shrink-0"
                          data-act="gw-model-copy"
                          aria-label={labels.copyId}
                          onClick={() => void onCopyId(id)}
                        >
                          {copiedId === id ? <Check /> : <Copy />}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent side="top">
                        {copiedId === id ? labels.idCopied : labels.copyId}
                      </TooltipContent>
                    </Tooltip>
                  </div>
                </td>
                <td className="px-2 py-2">
                  {/* 上下文窗口：数字来自官方目录（逐模型不同），未知就显示 `—`，不猜。 */}
                  <span className="font-mono text-xs">{formatContext(row.context_window)}</span>
                </td>
                {/* 能力列：内容全部由调用方注入 ⇒ 壳里仍然只有那 2 处 onClick。 */}
                {imageColumn && <td className="px-2 py-2">{imageColumn.render(row)}</td>}
                {effortColumn && <td className="px-2 py-2">{effortColumn.render(row)}</td>}
                {creditsColumn && <td className="px-2 py-2">{creditsColumn.render(row)}</td>}
                {showUpstream && (
                  <td className="px-2 py-2">
                    {/* 上游状态列**只看 `row.available`**。 */}
                    <Badge variant={row.available ? "success" : "warning"} className="rounded-md">
                      {row.available ? labels.upstreamAvailable : labels.upstreamUnavailable}
                    </Badge>
                  </td>
                )}
                <td className="px-2 py-2">
                  <div className="flex flex-wrap items-center gap-2">
                    {/* 网关暴露列**只看 `row.disabled`**。 */}
                    <Badge variant={row.disabled ? "secondary" : "success"} className="rounded-md">
                      {row.disabled ? labels.disabled : labels.exposed}
                    </Badge>
                    <DemoAction>
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-7 px-2"
                        data-act="gw-model-toggle"
                        disabled={pending}
                        onClick={() => onToggleDisabled(row, !row.disabled)}
                      >
                        {pending ? (
                          <Loader2 className="animate-spin" />
                        ) : row.disabled ? (
                          <RotateCcw />
                        ) : (
                          <Ban />
                        )}
                        {row.disabled ? labels.enable : labels.disable}
                      </Button>
                    </DemoAction>
                  </div>
                </td>
                {/* 附加列（WB 的「限流（探测）」）：内容由调用方注入，壳内**无 onClick**。 */}
                {extraColumn && <td className="px-2 py-2">{extraColumn.render(row)}</td>}
              </tr>
            );
          })}
        </tbody>
      </table>
      </div>
    </TooltipProvider>
  );
}
