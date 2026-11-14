import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * 「上下文上限」只读说明卡（WorkBuddy / Qoder 的模型页）。
 *
 * ## 为什么这两个平台没有开关
 *
 * Max Mode 是 **Trae 上游**的请求级布尔字段（`is_max_mode`）：同一个模型有 `dev` 与 `max`
 * 两个窗口，才需要「开 / 关」。WorkBuddy 的目录只有一个 `context_window`、Qoder 的模型
 * 元数据只有一个 `max_input_tokens` —— 它们**本来就是模型上限**，没有第二个模式可切。
 *
 * 开关的语义是「清单声明上限档还是默认档」：上游给两档（如 300k 默认 / 1M 上限），
 * 默认使用上限。Trae 另有请求级 `is_max_mode`，在网关页单独给开关。
 */
export function ContextSourceCard({
  className,
  checked,
  disabled,
  onChange,
  title,
  description,
}: {
  className?: string;
  /** 传入即渲染开关（默认 true = 使用上限）。 */
  checked?: boolean;
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
  /** 标题 / 描述覆盖（Trae 的开关叫 Max Mode，文案不同）。 */
  title?: string;
  description?: string;
}) {
  const t = useT();
  return (
    <div
      className={cn(
        "mb-3 rounded-lg border border-border/60 bg-muted/20 px-4 py-3",
        className,
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-medium">
            {title ?? t("shared.gateway.models.contextTitle")}
          </div>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            {description ?? t("shared.gateway.models.contextFixedDesc")}
          </p>
        </div>
        {onChange ? (
          <Switch
            checked={checked ?? true}
            disabled={disabled}
            onCheckedChange={onChange}
            aria-label={title ?? t("shared.gateway.models.contextTitle")}
          />
        ) : (
          <Badge variant="secondary" className="shrink-0 rounded-md">
            {t("shared.gateway.models.contextFromCatalog")}
          </Badge>
        )}
      </div>
    </div>
  );
}
