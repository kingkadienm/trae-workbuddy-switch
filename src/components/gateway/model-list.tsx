import { useDeferredValue, useMemo, useState, useCallback, useEffect } from "react";
import { Copy, Loader2, RefreshCw } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { DemoAction } from "@/components/demo-action";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import * as api from "@/lib/api";
import { useT } from "@/lib/i18n";
import { REGIONS, regionDescriptor } from "@/lib/region";
import { copyText } from "@/lib/clipboard";
import { cn } from "@/lib/utils";
import type { TranslationKey } from "@/locales/zh";
import type { CatalogSource, Region } from "@/lib/types";
import { useGatewayStore } from "@/stores/gateway";

const SOURCE_LABEL: Record<CatalogSource, { labelKey: TranslationKey; variant: "success" | "secondary" | "warning" }> = {
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

export function ModelList({ className }: { className?: string }) {
  const t = useT();
  const [region, setRegion] = useState<Region>("cn");
  const [refreshing, setRefreshing] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  const [onlyFree, setOnlyFree] = useState(false);

  const snapshot = useGatewayStore((s) => s.models[region]);
  const refreshModels = useGatewayStore((s) => s.refreshModels);
  const load = useGatewayStore((s) => s.loadModels);

  // 切换 region 时自动从本地缓存 / 接口加载模型列表
  useEffect(() => {
    void load(region);
  }, [region, load]);

  async function onRefresh() {
    setRefreshing(true);
    try {
      await refreshModels(region);
      toast.success(t("wbStats.gateway.refreshed"));
    } catch (e) {
      toast.error(t("wbStats.gateway.refreshFail"), { description: api.asError(e) });
    } finally {
      setRefreshing(false);
    }
  }

  async function onLoad() {
    try {
      await load(region);
    } catch {
      // load 已吞错并 toast
    }
  }

  const handleCopyId = useCallback(async (modelId: string) => {
    await copyText(modelId, t("wbStats.gateway.copiedId" as TranslationKey));
    setCopiedId(modelId);
    setTimeout(() => setCopiedId(null), 1500);
  }, [t]);

  const source = snapshot ? SOURCE_LABEL[snapshot.source] : null;

  const filtered = useMemo(() => {
    if (!snapshot) return [];
    let list = snapshot.models;
    const q = deferredSearch.trim().toLowerCase();
    if (q) list = list.filter((m) => m.name.toLowerCase().includes(q) || m.id.toLowerCase().includes(q));
    if (onlyFree) list = list.filter((m) => m.free);
    return list;
  }, [snapshot, deferredSearch, onlyFree]);

  return (
    <TooltipProvider delayDuration={300}>
      <Card className={cn("gap-0 py-0 overflow-hidden", className)}>
        {/* 顶栏 */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 px-4 py-2.5">
          <div className="flex flex-wrap items-center gap-3">
            <span className="text-sm font-semibold">{t("wbStats.gateway.modelList")}</span>
            <Tabs value={region} onValueChange={(value) => { setRegion(value as Region); setSearch(""); setOnlyFree(false); }}>
              <TabsList>
                {REGIONS.map((r) => (
                  <TabsTrigger key={r} value={r}>
                    {regionDescriptor(r).versionLabel}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {source && snapshot && (
              <span className="text-xs text-muted-foreground">
                <Badge variant={source.variant} className="rounded-md">{t(source.labelKey)}</Badge>
                {snapshot.fetched_at && <> · {formatTime(snapshot.fetched_at)} · {snapshot.models.length} models</>}
              </span>
            )}
            <DemoAction>
              <Button variant="ghost" size="sm" onClick={() => void onLoad()} disabled={refreshing} title="Reload cached">
                <RefreshCw className="size-3.5" />
              </Button>
            </DemoAction>
            <DemoAction>
              <Button variant="ghost" size="sm" onClick={() => void onRefresh()} disabled={refreshing} title={t("wbStats.gateway.refresh")}>
                {refreshing ? <Loader2 className="animate-spin size-3.5" /> : <RefreshCw className="size-3.5" />}
              </Button>
            </DemoAction>
          </div>
        </div>

        {/* 筛选栏 */}
        {snapshot && snapshot.models.length > 0 && (
          <div className="flex flex-wrap items-center gap-2 border-b border-border/40 px-4 py-2">
            <div className="relative flex-1 min-w-[160px] max-w-xs">
              <RefreshCw className="absolute left-2.5 top-1/2 size-3 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("wbStats.gateway.searchPlaceholder")}
                className="h-7 pl-7 text-xs"
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
            <span className="text-[10px] text-muted-foreground">{filtered.length} / {snapshot.models.length}</span>
          </div>
        )}

        {/* 表格 */}
        <div className="overflow-x-auto">
          <table className="w-full min-w-[640px] text-left text-sm">
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
              {filtered.map((model) => (
                <tr key={model.id} className="border-t border-border/50 hover:bg-muted/20">
                  <td className="px-4 py-2">
                    <div className="flex flex-col">
                      <span className="font-medium">{model.name}</span>
                      <div className="flex items-center gap-1.5">
                        <code className="font-mono text-[10px] text-muted-foreground">{model.id}</code>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <button
                              onClick={() => handleCopyId(model.id)}
                              className="text-muted-foreground hover:text-foreground"
                            >
                              <Copy className="size-3" />
                            </button>
                          </TooltipTrigger>
                          <TooltipContent side="top" className="text-xs">
                            {copiedId === model.id ? t("wbStats.gateway.copiedId") : t("wbStats.gateway.copyId")}
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
                  <td className="px-4 py-2 text-xs text-muted-foreground">{formatTokens(model.context_window)}</td>
                  <td className="px-4 py-2 text-xs text-muted-foreground">{formatTokens(model.max_tokens)}</td>
                  <td className="px-4 py-2 text-xs text-muted-foreground">
                    {model.supports_images ? (
                      <Badge variant="secondary" className="rounded-md px-1.5 py-0 text-[10px]">
                        {t("wbStats.gateway.supportsImage", { fallback: "Yes" })}
                      </Badge>
                    ) : (
                      <span className="text-muted-foreground/50">—</span>
                    )}
                  </td>
                  <td className="px-4 py-2 text-xs">
                    {model.credits ? <span className="text-amber-300">{model.credits}</span> : <span className="text-muted-foreground/50">—</span>}
                  </td>
                  <td className="px-4 py-2">
                    <div className="flex flex-wrap gap-1">
                      {model.badges.map((b) => (
                        <span key={b} className="rounded-md border border-border bg-muted/40 px-1.5 py-0 text-[10px]">{b}</span>
                      ))}
                    </div>
                  </td>
                </tr>
              ))}
              {filtered.length === 0 && snapshot && (
                <tr>
                  <td colSpan={6} className="px-4 py-12">
                    <p className="text-center text-sm text-muted-foreground">{t("growth.empty", { fallback: "No matching models" })}</p>
                  </td>
                </tr>
              )}
              {!snapshot && (
                <tr>
                  <td colSpan={6} className="px-4 py-12">
                    <p className="text-center text-sm text-muted-foreground">{t("wbStats.gateway.noModels")}</p>
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </Card>
    </TooltipProvider>
  );
}
