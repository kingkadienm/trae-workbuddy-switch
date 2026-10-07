import { useEffect, useMemo, useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useT } from "@/lib/i18n";
import { REGIONS, regionDescriptor } from "@/lib/region";
import type { AccountMeta, Region } from "@/lib/types";
import { useAccountsStore } from "@/stores/accounts";

function poolStatus(account: AccountMeta): "available" | "cooldown" | "disabled" | "unknown" {
  if (account.needsRelogin) return "disabled";
  const expiresAt = account.expiresAt ?? account.refreshExpiresAt ?? null;
  if (!expiresAt) return "unknown";
  const ms = expiresAt - Date.now();
  if (ms <= 0) return "cooldown";
  if (ms < 7 * 24 * 60 * 60 * 1000) return "cooldown";
  return "available";
}

const STATUS_STYLE: Record<string, { cls: string; label: string }> = {
  available: { cls: "border-emerald-500/40 bg-emerald-500/10 text-emerald-300", label: "可用" },
  cooldown: { cls: "border-amber-500/40 bg-amber-500/10 text-amber-300", label: "冷却中" },
  disabled: { cls: "border-red-500/40 bg-red-500/10 text-red-300", label: "会话失效" },
  unknown: { cls: "border-muted-foreground/30 bg-muted-foreground/10 text-muted-foreground", label: "未知" },
};

export default function AccountPoolPage() {
  const t = useT();
  const [region, setRegion] = useState<Region>("cn");
  const [loading, setLoading] = useState(false);
  const accounts = useAccountsStore((s) => (region === "cn" ? s.accounts : s.global.accounts));
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

  const total = accounts.length;

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("nav.modelCatalog", { fallback: "Account pool" })}</h1>
          <p className="text-xs text-muted-foreground">
            {t("shared.api.requestFailed", { status: String(total) })}
          </p>
        </div>
        <Button variant="secondary" onClick={() => void fetchAllRegions()} disabled={loading}>
          {loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}
          {t("growth.actions.refresh", { fallback: "Refresh" })}
        </Button>
      </div>

      <Tabs value={region} onValueChange={(v) => setRegion(v as Region)}>
        <TabsList>
          {REGIONS.map((r) => (
            <TabsTrigger key={r} value={r}>{regionDescriptor(r).versionLabel}</TabsTrigger>
          ))}
        </TabsList>

        {REGIONS.map((r) => (
          <TabsContent key={r} value={r} className="mt-4">
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              {(["available", "cooldown", "disabled", "unknown"] as const).map((key) => (
                <Card key={key} className="border-sidebar-border bg-sidebar-accent/40 p-3">
                  <div className="text-[11px] text-muted-foreground">{STATUS_STYLE[key].label}</div>
                  <div className="text-xl font-semibold">{counts[key]}</div>
                </Card>
              ))}
            </div>

            <div className="mt-4 flex flex-col gap-2">
              {accounts.map((account) => {
                const status = poolStatus(account);
                const style = STATUS_STYLE[status];
                const label = account.nickname || account.email || account.uid || account.id;
                return (
                  <Card key={account.id} className="border-sidebar-border bg-sidebar-accent/20">
                    <div className="flex flex-wrap items-center justify-between gap-3 p-4">
                      <div className="min-w-0">
                        <div className="text-sm font-medium">{label}</div>
                        <div className="text-[11px] text-muted-foreground">{account.id}</div>
                      </div>
                      <span className={cn("inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium", style.cls)}>
                        {style.label}
                      </span>
                    </div>
                  </Card>
                );
              })}
              {!accounts.length && !loading && (
                <div className="flex flex-col items-center gap-2 py-20 text-muted-foreground">
                  <RefreshCw className="size-5" />
                  <p className="text-sm">{t("growth.empty", { fallback: "No accounts" })}</p>
                </div>
              )}
            </div>
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}

function cn(...args: Array<string | false | undefined | null>) {
  return args.filter(Boolean).join(" ").trim() || "";
}
