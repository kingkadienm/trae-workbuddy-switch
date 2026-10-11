import { useDeferredValue, useMemo, useState, useCallback, useEffect } from "react";
import { Copy, Loader2, RefreshCw, Search } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useT } from "@/lib/i18n";
import { REGIONS, regionDescriptor } from "@/lib/region";
import { copyText } from "@/lib/clipboard";
import type { TranslationKey } from "@/locales/zh";
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
  refreshError?: string | null;
};

type CatalogSource = "live" | "cached" | "builtin";

const SOURCE_META: Record<CatalogSource, { labelKey: string; variant: "success" | "secondary" | "warning" }> = {
  live: { labelKey: "wbStats.gateway.sourceLive", variant: "success" },
  cached: { labelKey: "wbStats.gateway.sourceCached", variant: "secondary" },
  builtin: { labelKey: "wbStats.gateway.sourceBuiltin", variant: "warning" },
};

function formatTime(ts: number | null): string {
  if (!ts) return "—";
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return "—";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`;
  }
  if (n >= 1000) {
    const k = n / 1000;
    return `${Number.isInteger(k) ? k : k.toFixed(1)}K`;
  }
  return String(n);
}

export default function ModelCatalogPage() {
  const t = useT();
  const [region, setRegion] = useState<Region>("cn");
  const [refreshing, setRefreshing] = useState(false);
  const [loading, setLoading] = useState(false);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  const [onlyFree, setOnlyFree] = useState(false);
  const [badgeFilter, setBadgeFilter] = useState<string>("all");

  // 进入页面 / 切换 region 时自动加载模型列表
  useEffect(() => {
    void load();
  }, [region]);

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
      if (res.refreshError) {
        toast.warning(t("wbStats.gateway.refreshPartialFail" as TranslationKey), {
          description: res.refreshError,
        });
      } else {
        toast.success(t("wbStats.gateway.refreshed" as TranslationKey));
      }
    } catch (e) {
      toast.error(t("wbStats.gateway.refreshFail" as TranslationKey), { description: api.asError?.(e) ?? String(e) });
    } finally {
      setRefreshing(false);
    }
  }

  const onCopyId = useCallback(
    async (modelId: string) => {
      await copyText(modelId, t("wbStats.gateway.copiedId" as TranslationKey));
    },
    [t],
  );

  const onTabChange = useCallback(
    (r: Region) => {
      setRegion(r);
      setSearch("");
      setOnlyFree(false);
      setBadgeFilter("all");
    },
    [],
  );

  const models = useMemo(() => {
    if (!snapshot) return [];
    let list = snapshot.models;
    const q = deferredSearch.trim().toLowerCase();
    if (q) list = list.filter((m) => m.name.toLowerCase().includes(q) || m.id.toLowerCase().includes(q));
    if (onlyFree) list = list.filter((m) => m.free);
    if (badgeFilter !== "all") list = list.filter((m) => m.badges.includes(badgeFilter));
    return list;
  }, [snapshot, deferredSearch, onlyFree, badgeFilter]);

  const badgeOptions = useMemo(() => {
    if (!snapshot) return [];
    const set = new Set<string>();
    for (const m of snapshot.models) for (const b of m.badges) set.add(b);
    return Array.from(set).sort();
  }, [snapshot]);

  const source = snapshot ? (SOURCE_META as Record<string, { labelKey: string; variant: "success" | "secondary" | "warning" }>)[snapshot.source] : null;

  return (
    <TooltipProvider delayDuration={300}>
      <div className="mx-auto max-w-6xl space-y-4 p-4">
        {/* 页头 */}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h1 className="text-lg font-semibold">{t("wbStats.gateway.modelList")}</h1>
            <p className="text-xs text-muted-foreground">
              {snapshot && source && (
                <>
                  {t("wbStats.gateway.source" as TranslationKey)}{" "}
                  <Badge variant={source.variant} className="rounded-md">{t(source.labelKey as TranslationKey)}</Badge>
                  {snapshot.fetched_at && <> · {t("wbStats.gateway.updatedAt", { time: formatTime(snapshot.fetched_at) })}</>}
                  {snapshot.models.length > 0 && <> · {snapshot.models.length} models</>}
                  {snapshot.refreshError && <> · <span className="text-amber-600">{snapshot.refreshError}</span></>}
                </>
              )}
            </p>
          </div>
          <div className="flex gap-2">
            <Button variant="secondary" size="sm" onClick={load} disabled={loading}>
              {loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : <Search className="mr-2 size-4" />}
              {t("growth.actions.refresh", { fallback: "Refresh" })}
            </Button>
            <Button variant="outline" size="sm" onClick={refresh} disabled={refreshing}>
              {refreshing ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}
              {t("wbStats.gateway.refresh")}
            </Button>
          </div>
        </div>

        {/* Region tabs */}
        <Tabs value={region} onValueChange={(v) => onTabChange(v as Region)}>
          <TabsList>
            {REGIONS.map((r) => (
              <TabsTrigger key={r} value={r}>{regionDescriptor(r).versionLabel}</TabsTrigger>
            ))}
          </TabsList>

          {REGIONS.map((r) => (
            <TabsContent key={r} value={r} className="mt-4">
              <Card className="overflow-hidden">
                {/* 工具栏 */}
                <div className="flex flex-wrap items-center gap-2 border-b border-border/60 px-4 py-2.5">
                  <div className="relative flex-1 min-w-[180px] max-w-xs">
                    <Search className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                    <Input
                      value={search}
                      onChange={(e) => setSearch(e.target.value)}
                      placeholder={t("wbStats.gateway.searchPlaceholder")}
                      className="h-8 pl-8 text-xs"
                    />
                  </div>
                  <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <input
                      type="checkbox"
                      checked={onlyFree}
                      onChange={(e) => setOnlyFree(e.target.checked)}
                      className="size-3.5 rounded border-border accent-primary"
                    />
                    {t("wbStats.gateway.showFreeOnly")}
                  </label>
                  <Select value={badgeFilter} onValueChange={setBadgeFilter}>
                    <SelectTrigger className="h-8 w-[160px] text-xs" aria-label={t("wbStats.gateway.filterByBadge")}>
                      <SelectValue placeholder={t("wbStats.gateway.filterByBadge")} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="all">{t("wbStats.gateway.allTags")}</SelectItem>
                      {badgeOptions.map((b) => (
                        <SelectItem key={b} value={b}>{b}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <span className="text-xs text-muted-foreground">{models.length} / {snapshot?.models.length ?? 0}</span>
                </div>

                {/* 表格 */}
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[720px] text-left text-sm">
                    <thead>
                      <tr className="text-xs text-muted-foreground">
                        <th className="px-4 py-2 font-medium">{t("wbStats.gateway.tableHeaderModel")}</th>
                        <th className="px-4 py-2 font-medium">{t("wbStats.gateway.tableHeaderContext")}</th>
                        <th className="px-4 py-2 font-medium">{t("wbStats.gateway.tableHeaderMax")}</th>
                        <th className="px-4 py-2 font-medium">{t("wbStats.gateway.tableHeaderImage")}</th>
                        <th className="px-4 py-2 font-medium">{t("wbStats.gateway.tableHeaderCredits")}</th>
                        <th className="px-4 py-2 font-medium">{t("wbStats.gateway.tableHeaderBadges")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {models.map((model) => (
                        <tr key={model.id} className="border-t border-border/50 hover:bg-muted/30">
                          <td className="px-4 py-2.5">
                            <div className="flex flex-col">
                              <span className="font-medium">{model.name}</span>
                              <div className="flex items-center gap-1.5">
                                <code className="font-mono text-[10px] text-muted-foreground">{model.id}</code>
                                <Tooltip>
                                  <TooltipTrigger asChild>
                                    <button
                                      onClick={() => onCopyId(model.id)}
                                      className="text-muted-foreground hover:text-foreground"
                                    >
                                      <Copy className="size-3" />
                                    </button>
                                  </TooltipTrigger>
                                  <TooltipContent side="top" className="text-xs">
                                    {t("wbStats.gateway.copyId")}
                                  </TooltipContent>
                                </Tooltip>
                              </div>
                            </div>
                            {model.free && (
                              <Badge variant="success" className="mt-1.5 rounded-md px-1.5 py-0 text-[10px]">
                                {t("wbStats.gateway.free")}
                              </Badge>
                            )}
                          </td>
                          <td className="px-4 py-2.5 text-xs text-muted-foreground">
                            {formatTokens(model.context_window)}
                          </td>
                          <td className="px-4 py-2.5 text-xs text-muted-foreground">
                            {formatTokens(model.max_tokens)}
                          </td>
                          <td className="px-4 py-2.5 text-xs text-muted-foreground">
                            {model.supports_images ? (
                              <Badge variant="secondary" className="rounded-md px-1.5 py-0 text-[10px]">
                                {t("wbStats.gateway.supportsImage", { fallback: "Yes" })}
                              </Badge>
                            ) : (
                              <span className="text-muted-foreground/60">—</span>
                            )}
                          </td>
                          <td className="px-4 py-2.5 text-xs">
                            {model.credits ? <span className="text-amber-300">{model.credits}</span> : <span className="text-muted-foreground/60">—</span>}
                          </td>
                          <td className="px-4 py-2.5">
                            <div className="flex flex-wrap gap-1">
                              {model.badges.map((b) => (
                                <span key={b} className="rounded-md border border-border bg-muted/40 px-1.5 py-0 text-[10px]">{b}</span>
                              ))}
                            </div>
                          </td>
                        </tr>
                      ))}
                      {!models.length && !loading && (
                        <tr>
                          <td colSpan={6} className="px-4 py-16">
                            <div className="flex flex-col items-center gap-2 text-muted-foreground">
                              <Search className="size-5" />
                              <p className="text-sm">{t("growth.empty", { fallback: "No data" })}</p>
                            </div>
                          </td>
                        </tr>
                      )}
                    </tbody>
                  </table>
                </div>
              </Card>
            </TabsContent>
          ))}
        </Tabs>
      </div>
    </TooltipProvider>
  );
}
