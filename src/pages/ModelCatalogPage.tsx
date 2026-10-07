import { useMemo, useState } from "react";
import { Loader2, RefreshCw, Search } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useT } from "@/lib/i18n";
import { REGIONS, regionDescriptor } from "@/lib/region";
import type { Region } from "@/lib/types";
import * as api from "@/lib/api";

type CatalogModel = {
  id: string;
  name: string;
  context_window: number;
  max_tokens: number;
  supports_images: boolean;
  credits?: string | null;
  badges: string[];
  free: boolean;
};

type Snapshot = {
  region: string;
  source: string;
  fetched_at: number | null;
  models: CatalogModel[];
  note?: string | null;
};

export default function ModelCatalogPage() {
  const t = useT();
  const [region, setRegion] = useState<Region>("cn");
  const [refreshing, setRefreshing] = useState(false);
  const [search, setSearch] = useState("");
  const [onlyFree, setOnlyFree] = useState(false);
  const [badgeFilter, setBadgeFilter] = useState<string>("all");

  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [loading, setLoading] = useState(false);

  async function load() {
    setLoading(true);
    try {
      const res = await api.getGatewayModels(region);
      setSnapshot(res as Snapshot);
    } catch (e) {
      toast.error(t("shared.api.requestFailed", { status: "-" }));
    } finally {
      setLoading(false);
    }
  }

  async function refresh() {
    setRefreshing(true);
    try {
      const res = await api.refreshGatewayModels(region);
      setSnapshot(res as Snapshot);
      toast.success(t("wbStats.gateway.refreshed"));
    } catch (e) {
      toast.error(t("wbStats.gateway.refreshFail"), { description: api.asError?.(e) ?? String(e) });
    } finally {
      setRefreshing(false);
    }
  }

  const models = useMemo(() => {
    if (!snapshot) return [];
    let list = snapshot.models;
    const q = search.trim().toLowerCase();
    if (q) list = list.filter((m) => m.name.toLowerCase().includes(q) || m.id.toLowerCase().includes(q));
    if (onlyFree) list = list.filter((m) => m.free);
    if (badgeFilter !== "all") list = list.filter((m) => m.badges.includes(badgeFilter));
    return list;
  }, [snapshot, search, onlyFree, badgeFilter]);

  const badgeOptions = useMemo(() => {
    if (!snapshot) return [];
    const set = new Set<string>();
    for (const m of snapshot.models) for (const b of m.badges) set.add(b);
    return Array.from(set).sort();
  }, [snapshot]);

  return (
    <div className="mx-auto max-w-6xl space-y-4 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("wbStats.gateway.modelList")}</h1>
          <p className="text-xs text-muted-foreground">{snapshot ? `Source: ${snapshot.source}` : ""}</p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={load} disabled={loading}>
            {loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : <Search className="mr-2 size-4" />}
            {t("growth.actions.refresh", { fallback: "Refresh" })}
          </Button>
          <Button variant="outline" onClick={refresh} disabled={refreshing}>
            {refreshing ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}
            {t("wbStats.gateway.refresh")}
          </Button>
        </div>
      </div>

      <Tabs value={region} onValueChange={(v) => { setRegion(v as Region); setSearch(""); setOnlyFree(false); setBadgeFilter("all"); }}>
        <TabsList>
          {REGIONS.map((r) => (
            <TabsTrigger key={r} value={r}>{regionDescriptor(r).versionLabel}</TabsTrigger>
          ))}
        </TabsList>

        {REGIONS.map((r) => (
          <TabsContent key={r} value={r} className="mt-4">
            <Card className="space-y-3 p-4">
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  placeholder={t("growth.actions.refresh", { fallback: "Search models" })}
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="max-w-xs"
                />
                <label className="flex items-center gap-2 text-xs text-muted-foreground">
                  <input type="checkbox" checked={onlyFree} onChange={(e) => setOnlyFree(e.target.checked)} />
                  {t("wbStats.gateway.free")}
                </label>
                <select
                  value={badgeFilter}
                  onChange={(e) => setBadgeFilter(e.target.value)}
                  className="rounded-md border border-border bg-background px-2 py-1 text-xs"
                >
                  <option value="all">{t("wbStats.gateway.sourceCached", { fallback: "All badges" })}</option>
                  {badgeOptions.map((b) => (
                    <option key={b} value={b}>{b}</option>
                  ))}
                </select>
                <span className="text-xs text-muted-foreground">{models.length} models</span>
              </div>

              <div className="h-[calc(100vh-320px)] overflow-y-auto">
                <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
                  {models.map((model) => (
                    <Card key={model.id} className="border-sidebar-border bg-sidebar-accent/20 p-4">
                      <div className="flex items-start justify-between gap-2">
                        <div className="min-w-0">
                          <div className="truncate text-sm font-medium">{model.name}</div>
                          <div className="text-[11px] text-muted-foreground">{model.id}</div>
                        </div>
                        {model.free && (
                          <span className="shrink-0 rounded-md border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0 text-[10px] text-emerald-300">
                            {t("wbStats.gateway.free")}
                          </span>
                        )}
                      </div>
                      <div className="mt-2 text-xs text-muted-foreground">
                        context: {model.context_window.toLocaleString()} · max: {model.max_tokens.toLocaleString()}
                        {model.supports_images ? " · image" : ""}
                      </div>
                      {model.credits ? (
                        <div className="mt-1 text-xs text-amber-300">{model.credits}</div>
                      ) : null}
                      <div className="mt-2 flex flex-wrap gap-1">
                        {model.badges.map((b) => (
                          <span key={b} className="rounded-md border border-border bg-muted/40 px-1.5 py-0 text-[10px]">{b}</span>
                        ))}
                      </div>
                    </Card>
                  ))}
                  {!models.length && !loading && (
                    <div className="col-span-full flex flex-col items-center gap-2 py-20 text-muted-foreground">
                      <Search className="size-5" />
                      <p className="text-sm">{t("growth.empty", { fallback: "No data" })}</p>
                    </div>
                  )}
                </div>
              </div>
            </Card>
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}
