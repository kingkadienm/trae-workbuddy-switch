import { useDeferredValue, useMemo, useState, useEffect } from "react";
import { Loader2, RefreshCw, Search } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useT } from "@/lib/i18n";
import { REGIONS, regionDescriptor } from "@/lib/region";
import type { TranslationKey } from "@/locales/zh";
import type { AccountMeta, Region } from "@/lib/types";
import { useAccountsStore } from "@/stores/accounts";

type PoolStatus = "available" | "cooldown" | "disabled" | "unknown";

function poolStatus(account: AccountMeta): PoolStatus {
  if (account.deactivated) return "disabled";
  if (account.needsRelogin) return "disabled";
  const expiresAt = account.expiresAt ?? account.refreshExpiresAt ?? null;
  if (!expiresAt) return "unknown";
  const ms = expiresAt - Date.now();
  if (ms <= 0) return "cooldown";
  if (ms < 7 * 24 * 60 * 60 * 1000) return "cooldown";
  return "available";
}

const STATUS_META: Record<PoolStatus, { labelKey: string; variant: "success" | "warning" | "destructive" | "secondary" }> = {
  available: { labelKey: "wbAccounts.pool.available", variant: "success" },
  cooldown: { labelKey: "wbAccounts.pool.cooldown", variant: "warning" },
  disabled: { labelKey: "wbAccounts.pool.disabled", variant: "destructive" },
  unknown: { labelKey: "wbAccounts.pool.unknown", variant: "secondary" },
};

function formatExpiry(ts: number | null | undefined): string {
  if (!ts) return "—";
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return "—";
  const now = Date.now();
  const ms = ts - now;
  if (ms <= 0) return "已过期";
  const days = Math.floor(ms / (24 * 60 * 60 * 1000));
  if (days === 0) return "今天到期";
  if (days === 1) return "明天到期";
  if (days < 7) return `${days} 天后到期`;
  return `${String(date.getMonth() + 1).padStart(2, "0")}/${String(date.getDate()).padStart(2, "0")}`;
}

export default function AccountPoolPage() {
  const t = useT();
  const [region, setRegion] = useState<Region>("cn");
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  const [statusFilter, setStatusFilter] = useState<PoolStatus | "all">("all");

  const accounts = useAccountsStore((s) => (region === "cn" ? s.accounts : s.global.accounts));
  const creditMap = useAccountsStore((s) => s.creditMap);
  const fetchAllRegions = useAccountsStore((s) => s.fetchAllRegions);

  useEffect(() => {
    let disposed = false;
    setLoading(true);
    void fetchAllRegions().finally(() => {
      if (!disposed) setLoading(false);
    });
    return () => {
      disposed = true;
    };
  }, [fetchAllRegions]);

  const counts = useMemo(() => {
    const c = { available: 0, cooldown: 0, disabled: 0, unknown: 0 };
    for (const a of accounts) {
      const s = poolStatus(a);
      c[s] += 1;
    }
    return c;
  }, [accounts]);

  const filtered = useMemo(() => {
    let list = accounts;
    const q = deferredSearch.trim().toLowerCase();
    if (q) {
      list = list.filter((a) => {
        const name = (a.nickname || a.email || a.uid || a.id || "").toLowerCase();
        return name.includes(q) || a.id.toLowerCase().includes(q);
      });
    }
    if (statusFilter !== "all") {
      list = list.filter((a) => poolStatus(a) === statusFilter);
    }
    return list;
  }, [accounts, deferredSearch, statusFilter]);

  const total = accounts.length;

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-4">
      {/* 页头 */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("nav.accountPool")}</h1>
          <p className="text-xs text-muted-foreground">
            {t("wbAccounts.pool.summary", { total, available: counts.available, cooldown: counts.cooldown, disabled: counts.disabled, unknown: counts.unknown })}
          </p>
        </div>
        <Button variant="secondary" onClick={() => void fetchAllRegions()} disabled={loading}>
          {loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}
          {t("growth.actions.refresh", { fallback: "刷新" })}
        </Button>
      </div>

      {/* 筛选栏 */}
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative flex-1 min-w-[180px] max-w-xs">
          <Search className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("wbAccounts.pool.searchPlaceholder", { fallback: "搜索账号名称或 ID…" })}
            className="h-8 pl-8 text-xs"
          />
        </div>
        <div className="flex items-center gap-1.5">
          {(["all", "available", "cooldown", "disabled", "unknown"] as const).map((key) => (
            <Button
              key={key}
              variant={statusFilter === key ? "secondary" : "ghost"}
              size="sm"
              className="h-7 text-[11px]"
              onClick={() => setStatusFilter(key)}
            >
              {key === "all" ? t("wbAccounts.pool.all" as TranslationKey, { fallback: "全部" }) : t(STATUS_META[key].labelKey as TranslationKey)}
              {key !== "all" && <span className="ml-1 text-muted-foreground">{counts[key]}</span>}
            </Button>
          ))}
        </div>
      </div>

      {/* Region tabs */}
      <Tabs value={region} onValueChange={(v) => { setRegion(v as Region); setSearch(""); setStatusFilter("all"); }}>
        <TabsList>
          {REGIONS.map((r) => (
            <TabsTrigger key={r} value={r}>{regionDescriptor(r).versionLabel}</TabsTrigger>
          ))}
        </TabsList>

        {REGIONS.map((r) => (
          <TabsContent key={r} value={r} className="mt-4">
            {/* 状态统计卡片 */}
            <div className="mb-4 grid grid-cols-2 gap-2 sm:grid-cols-4">
              {(Object.entries(STATUS_META) as [PoolStatus, typeof STATUS_META[PoolStatus]][]).map(([key, meta]) => (
                <Card key={key} className="border-border/60 bg-muted/20 p-3">
                  <div className="text-[11px] text-muted-foreground">{t(meta.labelKey as TranslationKey)}</div>
                  <div className="text-xl font-semibold">{counts[key]}</div>
                </Card>
              ))}
            </div>

            {/* 账号列表 */}
            <div className="flex flex-col gap-2">
              {filtered.map((account) => {
                const status = poolStatus(account);
                const statusMeta = STATUS_META[status];
                const label = account.nickname || account.email || account.uid || account.id;
                const credit = creditMap[account.id];
                const expiry = account.expiresAt ?? account.refreshExpiresAt ?? null;

                return (
                  <Card key={account.id} className="border-border/60 bg-background hover:bg-muted/20 transition-colors">
                    <div className="flex flex-wrap items-start justify-between gap-3 p-4">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="text-sm font-medium">{label}</span>
                          {account.deactivated && (
                            <Badge variant="destructive" className="text-[10px]">{t("wbAccounts.pool.deactivated" as TranslationKey, { fallback: "已停用" })}</Badge>
                          )}
                        </div>
                        <div className="mt-1 font-mono text-[11px] text-muted-foreground">{account.id}</div>
                        {account.remark && (
                          <p className="mt-1 text-xs text-muted-foreground">{account.remark}</p>
                        )}
                        <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                          {expiry && (
                            <span className={status === "cooldown" ? "text-orange-600" : status === "disabled" ? "text-destructive" : ""}>
                              {formatExpiry(expiry)}
                            </span>
                          )}
                          {credit?.ok && credit.totalRemaining !== undefined && (
                            <span className="text-emerald-600">{credit.totalRemaining.toLocaleString()} 积分</span>
                          )}
                          {account.needsRelogin && account.needsReloginReason && (
                            <span className="text-destructive">{account.needsReloginReason}</span>
                          )}
                        </div>
                      </div>
                      <Badge variant={statusMeta.variant} className="shrink-0 rounded-md">
                        {t(statusMeta.labelKey as TranslationKey)}
                      </Badge>
                    </div>
                  </Card>
                );
              })}
              {!filtered.length && !loading && (
                <div className="flex flex-col items-center gap-2 py-16 text-muted-foreground">
                  <Search className="size-5" />
                  <p className="text-sm">{t("growth.empty", { fallback: "暂无账号" })}</p>
                </div>
              )}
            </div>
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}
