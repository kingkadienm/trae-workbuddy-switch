import { useEffect, useMemo, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { CloudOff, Coins, Loader2, MonitorSmartphone, RefreshCw, ShieldAlert, Users } from "lucide-react";

import { QoderLocalSourceRow } from "@/components/qoder/qoder-local-source-row";
import { ScopeChip, scopeLabel } from "@/components/scope-chip";
import { StatTiles, type StatTile } from "@/components/usage/stat-tiles";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import * as api from "@/lib/api";
import type { QoderAccountsView, QoderLocalSourcesView } from "@/lib/api";
import { useT } from "@/lib/i18n";
import { platformOfPath } from "@/lib/platforms/registry";
import { scopeOf } from "@/lib/scope";
import { useScope } from "@/lib/use-scope";
import { useRefreshAction } from "@/lib/use-refresh-action";
import { todayKey } from "@/lib/usage-window";
import { formatCredits } from "@/components/usage/format";

/** 账号概览里最多平铺几个名字（再多就去账号页看）。 */
const MAX_ACCOUNT_CHIPS = 6;

/**
 * Qoder 概览页。
 *
 * ## 四格都是**读得到**的（§3.5：不假承诺）
 *
 * 账号库条数 / 两处官方凭据四态 / 区域域名这三格一直是真数据；第四格（今日用量）
 * 在 2026-10-18 接上了**本机客户端会话记录**（`api.qoderUsage(scope, 1)`，纯本地扫描，
 * 与「概览是纯本地视图」不冲突）—— 用量页既然已接回，概览再说「没有数据源」就是**自相矛盾**。
 *
 * ⚠️ 两条不可让步的纪律：
 * ① **取数失败 ⇒ `—`，绝不回落 0**（`0` 会被读成「今天确实没有记录」）；
 * ② **口径只承诺「本机客户端已记录」**：不含其它设备、不含经网关的第三方调用 ——
 *    故标签是「今日已记录积分」而不是「今日用量 / 已入账」（后者是结算语义，
 *    本地记录证明不了）。
 *
 * ## 为什么不复用 `OverviewStats`
 *
 * 那个组件的 `metrics` 形状是 WorkBuddy / Trae 的（token / credits / gateway 三类 KPI），
 * Qoder 侧**没有这三类数据源**。硬套只能造空值，等于把「没数据」伪装成「有指标」。
 * 这里复用它的**视觉语言**（同一个 `StatTiles`），而不是复用它的数据形状。
 */
export default function QoderOverviewPage() {
  const t = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const product = platformOfPath(location.pathname);
  const [region] = useScope(product);
  const scope = scopeOf(product, region);

  const [accounts, setAccounts] = useState<QoderAccountsView | null>(null);
  /* 今日积分（`null` = 未知/取数失败 ⇒ 显示 `—`）。**独立取数**：失败只让这一格
     回落 `—`，绝不能把整页带成错误态（其余三格仍有真数据）。 */
  const [todayCredits, setTodayCredits] = useState<number | null>(null);
  const [sources, setSources] = useState<QoderLocalSourcesView | null>(null);
  const [error, setError] = useState("");
  const [reloading, setReloading] = useState(false);
  /* 「刷新」按钮的进行中态归 `useRefreshAction`（含最小展示时长）；`reloading` 继续
     负责内容清空 / Skeleton 的进出，两者都以「进行中」参与按钮的图标与禁用判据。 */
  const { busy, run } = useRefreshAction();

  useEffect(() => {
    let cancelled = false;
    setError("");
    Promise.all([api.getQoderAccounts(scope), api.getQoderLocalSources(scope)])
      .then(([accountView, sourceView]) => {
        if (cancelled) return;
        setAccounts(accountView);
        setSources(sourceView);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(api.asError(err));
        setAccounts(null);
        setSources(null);
      })
      .finally(() => {
        if (!cancelled) setReloading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [scope]);

  useEffect(() => {
    let cancelled = false;
    api
      .qoderUsage(scope, 1)
      .then((view) => {
        if (cancelled) return;
        // 成功扫到就是事实：今天没有记录 ⇒ 0（不是未知）。
        setTodayCredits(view.days.find((day) => day.date === todayKey())?.credits ?? 0);
      })
      .catch(() => {
        if (!cancelled) setTodayCredits(null);
      });
    return () => {
      cancelled = true;
    };
  }, [scope]);

  const readyCount = useMemo(
    () => (sources?.sources ?? []).filter((source) => source.state === "ready").length,
    [sources],
  );
  const totalSources = sources?.sources.length ?? 2;

  const tiles: StatTile[] = [
    {
      icon: Users,
      label: t("qoder.page.overview.kpi.accounts"),
      value: accounts ? String(accounts.accounts.length) : "—",
      hint:
        accounts && accounts.foreignDomainCount > 0
          ? t("qoder.page.overview.kpi.accountsForeign", { n: accounts.foreignDomainCount })
          : t("qoder.page.overview.kpi.accountsHint"),
      tone: accounts && accounts.foreignDomainCount > 0 ? "warn" : "default",
      title: t("qoder.page.overview.kpi.accountsTitle"),
    },
    {
      icon: MonitorSmartphone,
      label: t("qoder.page.overview.kpi.clients"),
      value: sources ? `${readyCount}/${totalSources}` : "—",
      hint: sources
        ? readyCount > 0
          ? t("qoder.page.overview.kpi.clientsReady")
          : t("qoder.page.overview.kpi.clientsNone")
        : t("qoder.page.overview.kpi.clientsHint"),
      title: t("qoder.page.overview.kpi.clientsTitle"),
    },
    {
      icon: CloudOff,
      label: t("qoder.page.overview.kpi.domain"),
      value: accounts?.domain ?? "—",
      hint: t("qoder.page.overview.kpi.domainHint"),
      title: t("qoder.page.overview.kpi.domainTitle"),
    },
    {
      /* 概览里换图标语义：不再是「无数据源」，而是「本机已记录的消耗」。 */
      icon: Coins,
      label: t("qoder.page.overview.kpi.usage"),
      /* ★ 取数失败仍用 `—`，**不是** `0` —— 见模块头。 */
      value: todayCredits === null ? "—" : formatCredits(todayCredits),
      hint: t("qoder.page.overview.kpi.usageHint"),
      title: t("qoder.page.overview.kpi.usageTitle"),
    },
  ];

  const names = (accounts?.accounts ?? []).slice(0, MAX_ACCOUNT_CHIPS);

  return (
    <div className="mx-auto flex w-full max-w-[1240px] flex-col gap-5 px-7 pt-[26px] pb-[60px]">
      <header className="flex items-start gap-4">
        <div className="min-w-0">
          <h1 className="text-[28px] font-[640] tracking-[-0.02em]">{t("qoder.page.overview.title")}</h1>
          <p className="mt-1.5 text-[12.5px] text-muted-foreground">
            {t("qoder.page.overview.subtitle", { scope: scopeLabel(t, scope) })}
          </p>
        </div>
        <div className="grow" />
        <Button
          variant="outline"
          size="sm"
          disabled={busy || reloading}
          onClick={() =>
            void run(() => {
              setReloading(true);
              setAccounts(null);
              setSources(null);
              return Promise.all([api.getQoderAccounts(scope), api.getQoderLocalSources(scope)])
                .then(([accountView, sourceView]) => {
                  setAccounts(accountView);
                  setSources(sourceView);
                  setError("");
                })
                .catch((err) => setError(api.asError(err)))
                .finally(() => setReloading(false));
            })
          }
        >
          {busy || reloading ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
        </Button>
        <ScopeChip scope={scope} />
      </header>

      {error ? (
        <Card className="gap-0 border-destructive/40 py-0">
          <div className="flex items-start gap-2.5 px-4 py-3.5">
            <ShieldAlert className="mt-0.5 size-4 shrink-0 text-destructive" />
            <div className="min-w-0 text-[12.5px]">{t("qoder.page.overview.error", { error })}</div>
          </div>
        </Card>
      ) : null}

      <StatTiles tiles={tiles} ariaLabel={t("qoder.page.overview.kpi.aria")} separated />

      {/* ── 本机凭据来源（四态）── */}
      <Card className="gap-0 py-0">
        <div className="flex flex-wrap items-start gap-3 px-4 py-3.5">
          <MonitorSmartphone className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 grow">
            <div className="text-[13px] font-[550]">{t("qoder.page.overview.clients.title")}</div>
            <div className="mt-0.5 text-[11.5px] text-muted-foreground">
              {t("qoder.page.overview.clients.description")}
            </div>
          </div>
          <Button variant="outline" size="sm" onClick={() => navigate(`/s/${scope}/accounts`)}>
            {t("qoder.page.overview.clients.action")}
          </Button>
        </div>
        {sources ? (
          <div className="border-t">
            {sources.sources.map((source) => (
              <QoderLocalSourceRow key={source.kind} source={source} />
            ))}
          </div>
        ) : (
          <div className="space-y-2 border-t px-4 py-3">
            <Skeleton className="h-3 w-40" />
            <Skeleton className="h-3 w-64" />
          </div>
        )}
      </Card>

      {/* ── 账号概览（名字只给前几个，明细去账号页）── */}
      <Card className="gap-0 py-0">
        <div className="flex flex-wrap items-start gap-3 px-4 py-3.5">
          <Users className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 grow">
            <div className="text-[13px] font-[550]">{t("qoder.page.overview.accounts.title")}</div>
            <div className="mt-0.5 text-[11.5px] text-muted-foreground">
              {t("qoder.page.overview.accounts.description")}
            </div>
          </div>
          <Button variant="outline" size="sm" onClick={() => navigate(`/s/${scope}/accounts`)}>
            {t("qoder.page.overview.accounts.action")}
          </Button>
        </div>
        {accounts ? (
          names.length > 0 ? (
            <div className="flex flex-wrap items-center gap-[5px] border-t px-4 py-3">
              {names.map((account) => (
                <Badge key={account.uid} className="border-0 px-[7px]">
                  {account.name}
                </Badge>
              ))}
              {accounts.accounts.length > names.length ? (
                <span className="text-[11.5px] text-muted-foreground">
                  {t("qoder.page.overview.accounts.more", { n: accounts.accounts.length - names.length })}
                </span>
              ) : null}
            </div>
          ) : (
            <div className="border-t px-4 py-3 text-[12px] text-muted-foreground">
              {t("qoder.page.overview.accounts.empty")}
            </div>
          )
        ) : (
          <div className="space-y-2 border-t px-4 py-3">
            <Skeleton className="h-3 w-48" />
          </div>
        )}
      </Card>

      {/* ── 尚未落地（用量 / 网关）：说明原因 + 唯一出口 ── */}
      <Card className="gap-0 py-0">
        <EmptyState
          icon={CloudOff}
          title={t("qoder.page.overview.pending.title")}
          description={t("qoder.page.overview.pending.description")}
          action={
            <Button variant="outline" size="sm" onClick={() => navigate(`/s/${scope}/accounts`)}>
              {t("qoder.page.overview.pending.action")}
            </Button>
          }
        />
      </Card>
    </div>
  );
}
