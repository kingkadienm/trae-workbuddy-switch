/**
 * 用量 · **对账抽屉**（issue #63）。
 *
 * ## 它回答什么
 *
 * 「客户端 + 网关」为什么不能简单相加：同一请求会被两处各记一次。
 * 抽屉把「怎么算出重叠的」摊开：结论 → 算式 → 匹配结果 → 匹配规则 → 覆盖范围 → 未匹配样本。
 *
 * ## 三条不能省的内容（省了就成了黑箱）
 *
 * 1. **结论句**（`两本账基本独立` / `客户端走的正是网关` / `部分重叠`）——
 *    用户不该自己去读匹配率再推断拓扑；
 * 2. **账号不参与匹配**的理由 —— 这是最容易被后来人「顺手加上」的一条，
 *    而它一旦加上，`subset` 会被误判成 `independent`（漏掉全部真重叠）；
 * 3. **覆盖范围** —— 网关日志只留最近 N 条，更早的请求无法逐条对账，
 *    所以合计给的是**区间**，不是点值。
 */

import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { useT } from "@/lib/i18n";
import { compactTokens, exactTokens, formatCredits } from "@/components/usage/format";
import type { GatewayUsageRecon } from "@/lib/types";

/** 时间戳 → `HH:MM:SS`（样本表用；跨天对账不成立，所以不显示日期）。 */
function clockOf(ts: number): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return "—";
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex gap-2 border-b border-border/40 py-1.5 text-xs last:border-0">
      <span className="flex-1 text-muted-foreground">{label}</span>
      <span className="tabular-nums text-foreground">{value}</span>
    </div>
  );
}

export function ReconcileSheet({
  open,
  onOpenChange,
  recon,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  recon: GatewayUsageRecon | null;
}) {
  const t = useT();
  if (!recon) return null;
  const percent = recon.clientRows > 0 ? Math.round((recon.matched / recon.clientRows) * 100) : 0;
  const verdictKey =
    recon.verdict === "subset"
      ? "subset"
      : recon.verdict === "mixed"
        ? "mixed"
        : "independent";
  const formula =
    recon.verdict === "subset"
      ? t("wbStats.usage.trend.formulaSubset", {
          gateway: compactTokens(recon.gatewayTokens),
          direct: compactTokens(Math.max(0, recon.clientTokens - recon.matchedTokens)),
        })
      : t("wbStats.usage.trend.formulaAdditive", {
          client: compactTokens(recon.clientTokens),
          gateway: compactTokens(recon.gatewayTokens),
          overlap: compactTokens(recon.matchedTokens),
        });

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-full overflow-y-auto sm:max-w-[452px]">
        <SheetHeader>
          <SheetTitle className="text-[15px]">{t("wbStats.usage.recon.title")}</SheetTitle>
          <SheetDescription>{t("wbStats.usage.recon.subtitle")}</SheetDescription>
        </SheetHeader>

        <div className="mt-4 space-y-4 px-1 pb-6">
          {/* ① 结论：不让用户自己去读匹配率推断拓扑 */}
          <div
            className={
              recon.verdict === "mixed"
                ? "rounded-[10px] border border-warning/30 bg-warning/10 px-3.5 py-3 text-xs leading-relaxed"
                : "rounded-[10px] border border-primary/25 bg-primary/10 px-3.5 py-3 text-xs leading-relaxed"
            }
          >
            <h3 className={recon.verdict === "mixed" ? "mb-1 font-semibold text-warning" : "mb-1 font-semibold text-primary"}>
              {t(`wbStats.usage.recon.verdict.${verdictKey}` as "wbStats.usage.recon.verdict.subset")}
            </h3>
            <p className="text-muted-foreground">
              {t(`wbStats.usage.recon.verdict.${verdictKey}Body` as "wbStats.usage.recon.verdict.subsetBody", {
                rows: exactTokens(recon.clientRows),
                matched: exactTokens(recon.matched),
                percent,
              })}
            </p>
          </div>

          {/* ② 算式：这个数是怎么来的 */}
          <div className="rounded-[10px] border bg-card px-3.5 py-3">
            <div className="text-2xl font-medium tracking-tight tabular-nums">
              ≈ {compactTokens(recon.estimate)}
            </div>
            <div className="mt-1 text-xs tabular-nums text-muted-foreground">{formula}</div>
          </div>

          <section>
            <h4 className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground">
              {t("wbStats.usage.recon.matched")}
            </h4>
            <Row label={t("wbStats.usage.recon.clientRows")} value={`${exactTokens(recon.clientRows)}`} />
            <Row label={t("wbStats.usage.recon.gatewayRows")} value={`${exactTokens(recon.gatewayRows)}`} />
            <Row
              label={t("wbStats.usage.recon.matchedRows")}
              value={`${exactTokens(recon.matched)} · ${compactTokens(recon.matchedTokens)}`}
            />
            <Row
              label={t("wbStats.usage.recon.unmatchedRows")}
              value={`${exactTokens(recon.clientRows - recon.matched)} · ${compactTokens(
                Math.max(0, recon.clientTokens - recon.matchedTokens),
              )}`}
            />
            <Row label={t("wbStats.usage.credits.col")} value={formatCredits(recon.matchedCredit)} />
          </section>

          <section>
            <h4 className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground">
              {t("wbStats.usage.recon.rule")}
            </h4>
            <p className="text-[11.5px] leading-relaxed text-muted-foreground">
              {t("wbStats.usage.recon.ruleBody")}
            </p>
            <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground">
              <b className="font-medium text-foreground">{t("wbStats.usage.recon.noAccount")}</b>
            </p>
          </section>

          <section>
            <h4 className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground">
              {t("wbStats.usage.recon.coverage")}
            </h4>
            {/* 只报「日志里有多少条」，不猜 `log_keep` 配置值 —— 猜错了就是假数字。 */}
            <Row label={recon.window.date} value={`${exactTokens(recon.window.logRows)} 条`} />
            <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground">
              {t("wbStats.usage.recon.coverageBody", {
                lower: compactTokens(recon.bounds.lower),
                upper: compactTokens(recon.bounds.upper),
                estimate: compactTokens(recon.estimate),
              })}
            </p>
          </section>

          {recon.unmatchedSamples.length > 0 ? (
            <section>
              <h4 className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground">
                {t("wbStats.usage.recon.samples")}
              </h4>
              <table className="w-full text-xs">
                <thead>
                  <tr className="border-b text-[11px] text-muted-foreground">
                    <th className="py-1.5 text-left font-medium">{t("wbStats.usage.recon.sampleTime")}</th>
                    <th className="py-1.5 text-left font-medium">{t("wbStats.usage.recon.sampleModel")}</th>
                    <th className="py-1.5 text-right font-medium">{t("wbStats.usage.recon.sampleTokens")}</th>
                  </tr>
                </thead>
                <tbody>
                  {recon.unmatchedSamples.map((sample, index) => (
                    <tr key={`${sample.ts}-${index}`} className="border-b border-border/40 last:border-0">
                      <td className="py-1.5 tabular-nums text-muted-foreground">{clockOf(sample.ts)}</td>
                      <td className="truncate py-1.5" title={sample.model}>
                        {sample.model}
                      </td>
                      <td className="py-1.5 text-right tabular-nums text-muted-foreground">
                        {compactTokens(sample.tokens)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}
