/**
 * 密钥 · **用量排行**（issue #76 / 规范 U4）。
 *
 * 「按密钥看用量」是 issue #76 的原始诉求之一：`key` 分给别人用之后，
 * 要能回答「**哪把 key 用掉了我多少额度**」。
 *
 * ## 数据源：网关账本的 `keys` 维度（2026-10-28 换源）
 *
 * 旧实现聚合的是**请求日志**（`get_gateway_logs`）。日志是环形缓冲（保留最近
 * `log_keep` 条，缺省 200）—— 本机实测繁忙时 200 条只覆盖 **17 分钟**，页面却标
 * 「近 30 天」⇒ 用户看到「44.2M Token / 200 次 / 100%」，量级像真的、区间是假的。
 * 现在改为读账本 `days[].keys`（与「按模型 / 按账号」同一份账、同一个水位线，
 * 按本地日累积 90 天），窗口才有意义。见 `src/lib/key-usage.ts` 文件头。
 *
 * ## 为什么「没有该维度」必须单独披露
 *
 * `keys` 是 v4 才加的维度：升级前的日期里它是**空数组**。画成「0 用量」等于说
 * 「那几天没人用过 Key」—— 一句凭空捏造的事实。所以窗口里的覆盖天数、以及
 * 「这一天的 key 维度只是下界」都要写出来（`keyCoverage` / `keysPartial`）。
 */

import { useEffect, useMemo, useState } from "react";

import {
  aggregateByKeyDim,
  firstKeyDay,
  keyCoverage,
  type KeyUsageSlice,
} from "@/lib/key-usage";
import { useT } from "@/lib/i18n";
import type { GatewayUsageDim } from "@/lib/types";
import { usageWindowKeys, type UsageWindow } from "@/lib/usage-window";

import { compactTokens, sharePercent } from "./format";
import { seriesColor } from "./palette";

export function KeyRanking({ slices }: { slices: KeyUsageSlice[] }) {
  if (slices.length === 0) return null;
  const total = slices.reduce((sum, slice) => sum + slice.tokens, 0);
  // token 全为 0（例如只有业务代理请求）⇒ 占比按**请求数**算，否则会全是「—」。
  const totalRequests = slices.reduce((sum, slice) => sum + slice.requests, 0);
  const useTokens = total > 0;

  return (
    <div className="space-y-3">
      {slices.map((slice, index) => {
        const value = useTokens ? slice.tokens : slice.requests;
        const denominator = useTokens ? total : totalRequests;
        const share = sharePercent(value, denominator);
        return (
          <div key={slice.bucket} className="min-w-0">
            <div className="flex min-w-0 items-center gap-3 text-xs">
              <span className="w-5 shrink-0 font-mono text-muted-foreground">
                {String(index + 1).padStart(2, "0")}
              </span>
              <span
                className="min-w-0 flex-1 truncate font-medium"
                title={slice.bucket}
              >
                {slice.name}
              </span>
              <span className="shrink-0 tabular-nums" title={`${slice.tokens} Token`}>
                {compactTokens(slice.tokens)}
              </span>
              <span className="w-14 shrink-0 text-right text-muted-foreground tabular-nums">
                {slice.requests} 次
              </span>
              <span className="w-12 shrink-0 text-right text-muted-foreground tabular-nums">
                {share}
              </span>
            </div>
            <div className="ml-8 mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                className="h-full rounded-full"
                style={{
                  width: share === "—" ? "0%" : `${(value / denominator) * 100}%`,
                  backgroundColor: seriesColor(index),
                }}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** 取密钥名的函数（**只取名字**：用量在账本里，不必再读日志）。 */
export type KeyNameLoader = () => Promise<ReadonlyMap<string, string>>;

/**
 * 「按密钥看用量」视图（**数据源注入**，WB / Trae / Qoder 共用）。
 *
 * 用量来自各平台的 `usage_dim`（三条产品线是三个命令，形状同构），
 * 名字来自各自的密钥表 —— 两者**分开注入**：账本给数、密钥表给名，
 * 任一取不到都不该让另一侧消失（名字缺失会退化成短 id）。
 *
 * @param dim 该平台的网关账本维度（`null` = 还没取到）
 * @param window 时间窗口（用 `usageWindowKeys` 过滤，与另三个视图同口径）
 * @param selfScope 本页所属台账（跨台账的 Key 会标出归属）
 * @param loadNames 取 `id → 展示名` 映射
 */
export function KeysView({
  dim,
  window,
  selfScope,
  loadNames,
}: {
  dim: GatewayUsageDim | null;
  window: UsageWindow;
  selfScope: string;
  loadNames: KeyNameLoader;
}) {
  const t = useT();
  const [keyNames, setKeyNames] = useState<ReadonlyMap<string, string>>(new Map());

  useEffect(() => {
    let cancelled = false;
    /* 名字取不到不算错：`aggregateByKeyDim` 会用短 id 兜底。
       这里**不设错误态** —— 为了一列展示名把整张排行换成错误页，是把可用降成不可用。 */
    void loadNames()
      .then((names) => {
        if (!cancelled) setKeyNames(names);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [loadNames]);

  const windowKeyList = useMemo(() => usageWindowKeys(window), [window]);
  const windowKeySet = useMemo(() => new Set(windowKeyList), [windowKeyList]);

  const slices = useMemo(
    () => (dim ? aggregateByKeyDim(dim.days, windowKeySet, keyNames, selfScope) : []),
    [dim, windowKeySet, keyNames, selfScope],
  );
  const coverage = useMemo(
    () => keyCoverage(dim?.days ?? [], windowKeyList),
    [dim, windowKeyList],
  );
  const since = useMemo(() => firstKeyDay(dim?.days ?? [], windowKeySet), [dim, windowKeySet]);

  if (!dim) return <div className="h-[180px] animate-pulse rounded-xl bg-muted" />;

  if (slices.length === 0) {
    return (
      <div className="rounded-xl border border-border p-6 text-center">
        <p className="text-sm font-medium">{t("wbStats.usage.keys.empty")}</p>
        <p className="mt-1 text-xs text-muted-foreground">
          {coverage.days === 0
            ? t("wbStats.usage.keys.emptyNoDim")
            : t("wbStats.usage.keys.emptyHint")}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="text-xs font-medium">{t("wbStats.usage.keys.title")}</div>
      <KeyRanking slices={slices} />
      <div className="space-y-1">
        <p className="text-[11.5px] leading-5 text-muted-foreground">
          {t("wbStats.usage.keys.caption")}
        </p>
        {/* ★ 窗口内缺这个维度的天数必须说出来：「没有记录」≠「用量为 0」。 */}
        {coverage.missing > 0 ? (
          <p className="text-[11.5px] leading-5 text-muted-foreground">
            {t("wbStats.usage.keys.coverageMissing", {
              days: coverage.days,
              missing: coverage.missing,
              since: since ?? "—",
            })}
          </p>
        ) : null}
        {coverage.partial > 0 ? (
          <p className="text-[11.5px] leading-5 text-muted-foreground">
            {t("wbStats.usage.keys.coveragePartial", { days: coverage.partial })}
          </p>
        ) : null}
      </div>
    </div>
  );
}
