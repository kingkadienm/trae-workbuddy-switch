/**
 * 模型能力单元格（issue #65）：**思考档位** + **积分倍率**。
 *
 * ## 为什么单独抽一个文件，而不是三页各写一份
 *
 * 这两列是**同一件事在三平台的同一份数据**（WB 目录 `reasoning` / Trae 客户端
 * `reasoning_effort_config` / Qoder `thinking_efforts`）。三页各拼一次 JSX ⇒ 迟早出现
 * 「WB 显示 `low / high`、Trae 显示 `low,high`、Qoder 显示 `思考 low high`」三种写法
 * —— 那正是 P10 §1.2 反复收敛掉的分叉。**渲染只在这里**，页面只负责把数据传进来。
 *
 * ## 为什么 `—` 要说清是哪一种 `—`
 *
 * 「没有档位」有**三种**成因，用户看到的结果都是空，但含义完全不同：
 *
 * | 成因 | 取值 | 文案 |
 * |:--|:--|:--|
 * | 该模型确实不能选档 | `unsupported` | 该模型不支持选档 |
 * | 这条程序位**没下发**这项能力 | `unknown` | 该程序位的清单未下发思考档位信息 |
 * | 上游没声明、内置表也没有 | `undeclared` | 上游未声明档位，内置档位表也没有 |
 *
 * ★ 实测依据（2026-10-21）：TraeWork 的 `solo_work_lite` 22 条里只有 1 条带档位 ——
 * 若把 `unknown` 写成「不支持选档」，用户会以为这张表在骗他。
 *
 * ## 纯展示，**不可点**
 *
 * 模型表每行恰好 2 个可点区（复制 id + 禁用/启用），门禁
 * `scripts/check-scope-registry.cjs` 钉死了这一点。这里刻意为纯文本 + `title`：
 * 想按档位筛选或按倍率排序，先改那条门禁并说明理由，**不要**在这里偷偷加 onClick。
 */

import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import type { TranslationKey } from "@/locales/zh";
import { useT } from "@/lib/i18n";

/** 「没有档位」的成因（决定 `—` 的 tooltip 文案）。 */
export type EffortUnavailable = "unknown" | "unsupported" | "undeclared";

/** 档位来源：上游目录 / 网关内置静态表 / 无。 */
export type EffortSource = "upstream" | "static" | null;

export interface EffortCellProps {
  /** 支持的档位（上游原词表，保序）。各平台词表**不互通**，不做归一映射。 */
  efforts: string[];
  /** 默认档；不在 `efforts` 里时后端已丢弃。 */
  defaultEffort?: string | null;
  /** `efforts` 为空时的成因。 */
  unavailable?: EffortUnavailable;
  /** 档位来源（决定 tooltip 的第一句）。 */
  source?: EffortSource;
}

const UNAVAILABLE_KEY: Record<EffortUnavailable, TranslationKey> = {
  unknown: "shared.gateway.models.effortNoneUnknown",
  unsupported: "shared.gateway.models.effortNoneUnsupported",
  undeclared: "shared.gateway.models.effortNoneUndeclared",
};

/**
 * 「思考档位」单元格。
 *
 * 有档位时：`low · high · max`，**默认档加粗**（用户最常问「默认是哪一档」）。
 * 词表按平台原样显示（`xhigh` 就是 `xhigh`）—— 把两套词表强行映射会造出
 * 一个实际不存在的等价关系（`xhigh` ≠ Trae 的 `extra_high`）。
 */
export function EffortCell({ efforts, defaultEffort, unavailable = "undeclared", source = null }: EffortCellProps) {
  const t = useT();
  if (efforts.length === 0) {
    return (
      <span
        className="text-xs text-muted-foreground"
        data-act="gw-model-efforts"
        data-efforts=""
        title={t(UNAVAILABLE_KEY[unavailable])}
      >
        —
      </span>
    );
  }

  const sourceTip =
    source === "upstream"
      ? t("shared.gateway.models.effortSourceUpstream")
      : source === "static"
        ? t("shared.gateway.models.effortSourceStatic")
        : null;

  return (
    <span
      className="inline-flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs"
      data-act="gw-model-efforts"
      data-efforts={efforts.join(",")}
      title={sourceTip ?? undefined}
    >
      {efforts.map((effort, index) => (
        <span key={effort} className="inline-flex items-center gap-x-1.5">
          {index > 0 && <span className="text-muted-foreground/60">·</span>}
          <span
            className={cn(
              "font-mono",
              effort === defaultEffort
                ? "font-semibold text-foreground"
                : "text-muted-foreground",
            )}
            title={
              effort === defaultEffort
                ? t("shared.gateway.models.effortDefault", { effort })
                : undefined
            }
          >
            {effort}
          </span>
        </span>
      ))}
    </span>
  );
}

export interface CreditsCellProps {
  /** 积分倍率（倍率不是金额）；`null` = 上游没给。 */
  rate: number | null | undefined;
  /** 上游标记的免费模型（倍率为 0）。 */
  free?: boolean;
  /** 同格附加内容（如 Qoder 的错峰活动徽标）—— 由调用方注入，本文件不认识平台。 */
  extra?: ReactNode;
}

/**
 * 倍率 → 显示文本：`0.2` → `×0.2`、`1.83` → `×1.83`、`6.67` → `×6.67`。
 *
 * 不用 `toFixed(2)`：上游给的是 `0.2`，写成 `×0.20` 会让用户以为精度更高。
 * 也不四舍五入到 1 位：`0.06` 会变成 `0.1`，倍率差一倍。
 */
function formatRate(rate: number): string {
  return String(Math.round(rate * 1000) / 1000);
}

/** 「积分倍率」单元格：免费 → 「免费」；有倍率 → `×0.79`；都没有 → `—`（带原因）。 */
export function CreditsCell({ rate, free = false, extra }: CreditsCellProps) {
  const t = useT();
  const hasRate = typeof rate === "number" && Number.isFinite(rate);
  return (
    <span className="inline-flex flex-wrap items-center gap-1.5 text-xs">
      {free || rate === 0 ? (
        <span className="font-medium text-success" data-act="gw-model-credits" data-credits="0">
          {t("shared.gateway.models.creditsFree")}
        </span>
      ) : hasRate ? (
        <span
          className="font-mono text-muted-foreground"
          data-act="gw-model-credits"
          data-credits={rate}
          title={t("shared.gateway.models.creditsHint")}
        >
          ×{formatRate(rate)}
        </span>
      ) : (
        <span
          className="text-muted-foreground"
          data-act="gw-model-credits"
          data-credits=""
          title={t("shared.gateway.models.creditsNone")}
        >
          —
        </span>
      )}
      {extra}
    </span>
  );
}
