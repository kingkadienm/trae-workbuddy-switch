import { Cpu, Loader2, RefreshCw } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { DemoAction } from "@/components/demo-action";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import type { TraeClientModel, TraeClientModelList } from "@/lib/trae-types";

/**
 * Trae 模型清单 —— 读**客户端（上游下发）**的清单缓存。
 *
 * ## 为什么现在**有**刷新按钮（推翻了上一版的裁定）
 *
 * 上一版这里是静态常量（`buddy-switch-gateway` 的 `payload::MODEL_NAMES`），
 * 「刷新」永远不可能改变结果，因此当时判定为假控件、刻意不加按钮，只留一行说明。
 *
 * issue #4 之后数据源换成了**客户端 `state.vscdb` 里的上游清单缓存**
 * （见 core 的 `trae::model_list`）：客户端刷新过模型列表之后，这里重新读一次
 * **真的**会拿到新清单。按钮不再是假控件，因此恢复。
 *
 * ## 空态不是错误
 *
 * 客户端没启动过 / 没登录 / 还没拉过清单时，后端返回 `source = "missing"` 与
 * 一句可读的 `note`。这里按空态呈现，**不报错** —— 「少一张清单」远好过整页红。
 */
export function TraeModelList({
  data,
  defaultModel,
  gatewayModelCount,
  refreshing,
  onRefresh,
  className,
}: {
  /** `get_trae_client_models` 的结果；未加载时为 `null`。 */
  data: TraeClientModelList | null;
  /** 网关配置里的默认模型（仅用于摘要文案）。 */
  defaultModel: string;
  /**
   * **网关对外清单**的模型数（`get_trae_gateway_models`）。
   *
   * 与 `data` 同源但**口径不同**：`data` 是分组视图（同一模型会在多个分组里重复出现），
   * 这里是扁平去重后的条数 —— 也就是外部客户端连本网关时**实际看到**的数量。
   * `null` = 没取到，不显示这一项。
   */
  gatewayModelCount: number | null;
  refreshing: boolean;
  onRefresh: () => void;
  className?: string;
}) {
  const t = useT();

  const groups = data?.groups ?? [];
  /**
   * 摘要用**去重后**的数量。
   *
   * 同一模型会在多个 function 分组里重复出现（实测 `solo_work_lite` 与
   * `solo_work_remote` 内容完全相同），直接累加会得到「共 99 个」这种与
   * 「网关对外 27 个」对不上的数字 —— 用户会以为哪里算错了。
   */
  const total = new Set(groups.flatMap((group) => group.models.map((model) => model.name))).size;
  const loaded = data?.source === "client-cache" && total > 0;

  return (
    <Card className={cn("gap-0 py-0", className)}>
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 px-5 py-3">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <Cpu className="size-4 stroke-[1.75]" />
          {t("trae.gateway.models.title")}
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-xs text-muted-foreground">
            {t("trae.gateway.models.summary", { count: total, model: defaultModel })}
          </span>
          <DemoAction>
            <Button variant="ghost" size="sm" onClick={() => onRefresh()} disabled={refreshing}>
              {refreshing ? <Loader2 className="animate-spin" /> : <RefreshCw />}
              {t("trae.gateway.models.refresh")}
            </Button>
          </DemoAction>
        </div>
      </div>

      <div className="px-5 py-4">
        <p className="mb-3 text-xs text-muted-foreground">{t("trae.gateway.models.note")}</p>

        {(data || gatewayModelCount !== null) && (
          <div className="mb-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
            {data && (
              <span className="flex items-center gap-1.5">
                {t("trae.gateway.models.source")}
                <Badge variant={loaded ? "success" : "warning"} className="rounded-md">
                  {t(
                    loaded ? "trae.gateway.models.sourceCache" : "trae.gateway.models.sourceMissing",
                  )}
                </Badge>
              </span>
            )}
            {data && loaded && (
              <span>{t("trae.gateway.models.readAt", { time: formatTime(data.readAt) })}</span>
            )}
            {gatewayModelCount !== null && (
              <span>{t("trae.gateway.models.gatewayCount", { count: gatewayModelCount })}</span>
            )}
          </div>
        )}

        {data?.note && <p className="mb-3 text-xs text-amber-600">{data.note}</p>}

        {!loaded ? (
          <p className="py-4 text-sm text-muted-foreground">{t("trae.gateway.models.empty")}</p>
        ) : (
          <div className="space-y-4">
            {groups.map((group) => (
              <div key={group.function} className="space-y-2">
                <div className="flex items-center gap-2">
                  <code className="font-mono text-xs text-muted-foreground">{group.function}</code>
                  <span className="text-xs text-muted-foreground">
                    {t("trae.gateway.models.groupCount", { count: group.models.length })}
                  </span>
                </div>
                <div className="flex flex-wrap gap-2">
                  {group.models.map((model) => (
                    <ModelChip key={model.name} model={model} />
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </Card>
  );
}

/** 单个模型条目：展示名为主，模型名（调用时真正要传的）用等宽小字跟随。 */
function ModelChip({ model }: { model: TraeClientModel }) {
  const t = useT();
  const context = model.contextWindow
    ? t("trae.gateway.models.context", { tokens: formatTokens(model.contextWindow) })
    : "";

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs",
        model.isDefault ? "border-foreground/30 bg-foreground/[0.06]" : "border-border bg-muted/40",
      )}
      title={context ? `${model.name} · ${context}` : model.name}
    >
      <span className="font-medium">{model.displayName}</span>
      {model.displayName !== model.name && (
        <code className="font-mono text-[10px] text-muted-foreground">{model.name}</code>
      )}
      {model.isDefault && (
        <Badge variant="success" className="rounded-md px-1.5 py-0 text-[10px]">
          {t("trae.gateway.models.badgeDefault")}
        </Badge>
      )}
      {model.isNew && (
        <Badge variant="secondary" className="rounded-md px-1.5 py-0 text-[10px]">
          {t("trae.gateway.models.badgeNew")}
        </Badge>
      )}
      {model.isBeta && (
        <Badge variant="warning" className="rounded-md px-1.5 py-0 text-[10px]">
          {t("trae.gateway.models.badgeBeta")}
        </Badge>
      )}
      {!model.isPreset && (
        <Badge variant="secondary" className="rounded-md px-1.5 py-0 text-[10px]">
          {t("trae.gateway.models.badgeCustom")}
        </Badge>
      )}
    </span>
  );
}

/** 毫秒时间戳 → `HH:mm`；无效值返回 `—`。 */
function formatTime(ts: number): string {
  if (!ts) return "—";
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return "—";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/** 上下文窗口 → 人类可读（`256000` → `256K`，`1000000` → `1M`）。 */
function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) {
    const millions = tokens / 1_000_000;
    return `${Number.isInteger(millions) ? millions : millions.toFixed(1)}M`;
  }
  if (tokens >= 1000) {
    const thousands = tokens / 1000;
    return `${Number.isInteger(thousands) ? thousands : thousands.toFixed(1)}K`;
  }
  return String(tokens);
}
