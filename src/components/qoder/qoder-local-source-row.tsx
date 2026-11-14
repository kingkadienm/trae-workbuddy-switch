import { Badge } from "@/components/ui/badge";
import type { QoderLocalSource } from "@/lib/api";
import { useT } from "@/lib/i18n";
import type { TranslationKey } from "@/locales/zh";

/**
 * 一处 Qoder 本机凭据来源的**四态**展示（账号页与概览页共用）。
 *
 * ## 为什么必须是四态（不能压成一个「未检测到」）
 *
 * `missing`（没装）/ `unreadable`（装了但读不出来）/ `empty`（读到了但没账号）/
 * `ready`（可导入）对应**四种不同的用户动作**：去装 / 去客户端重新登录 / 换一个来源 /
 * 直接导入。压成一个布尔值就会给出错误的引导。
 *
 * ## 为什么单独一个文件
 *
 * 两个页面（账号页 / 概览页）都要渲染它 ⇒ 各写一份必然在「文案 / 色调 / 是否显示 detail」
 * 上分叉。与「同一件事同一做法」同一条纪律。
 */
export function QoderLocalSourceRow({ source }: { source: QoderLocalSource }) {
  const t = useT();
  const stateKey: Record<QoderLocalSource["state"], TranslationKey> = {
    missing: "qoder.page.accounts.source.missing",
    unreadable: "qoder.page.accounts.source.unreadable",
    empty: "qoder.page.accounts.source.empty",
    ready: "qoder.page.accounts.source.ready",
  };
  const tone = source.state === "ready" ? "ok" : source.state === "unreadable" ? "warn" : "";
  return (
    <div className="flex items-start gap-2.5 border-b px-4 py-2.5 last:border-b-0">
      <div className="min-w-0 grow">
        <div className="text-[12.5px] font-[550]">{source.label}</div>
        <div className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground" title={source.path}>
          {source.path}
        </div>
        {/* 「读不出来」必须带原因 —— 否则用户只知道失败，不知道下一步做什么。 */}
        {source.detail ? (
          <div className="mt-1 text-[11px] text-muted-foreground">{source.detail}</div>
        ) : null}
      </div>
      <Badge
        className={`border-0 px-[7px] ${
          tone === "ok"
            ? "bg-primary/15 text-primary"
            : tone === "warn"
              ? "bg-[var(--warning)]/15 text-[var(--warning)]"
              : ""
        }`}
      >
        {t(stateKey[source.state])}
      </Badge>
    </div>
  );
}
