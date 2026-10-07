import { useMemo, useState } from "react";
import { Download, Eraser, Loader2, RefreshCw } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useT } from "@/lib/i18n";
import { REGIONS, regionDescriptor } from "@/lib/region";
import type { Region } from "@/lib/types";
import { useGatewayStore } from "@/stores/gateway";

function fmtTime(ts: number): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return "—";
  return `${String(d.getHours()).padStart(2,"0")}:${String(d.getMinutes()).padStart(2,"0")}:${String(d.getSeconds()).padStart(2,"0")}`;
}

export default function RequestLogPage() {
  const t = useT();
  const logs = useGatewayStore((s) => s.logs);
  const loadLogs = useGatewayStore((s) => s.loadLogs);
  const clearLogs = useGatewayStore((s) => s.clearLogs);
  const [region, setRegion] = useState<Region>("cn");
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [loading, setLoading] = useState(false);
  const [clearing, setClearing] = useState(false);

  async function onRefresh() {
    setLoading(true);
    try {
      await loadLogs();
      toast.success(t("wbStats.gateway.refreshed"));
    } finally {
      setLoading(false);
    }
  }

  async function onClear() {
    setClearing(true);
    try {
      await clearLogs();
      await loadLogs();
      toast.success(t("wbStats.gateway.cleared"));
    } finally {
      setClearing(false);
    }
  }

  const visible = useMemo(() => {
    let list = logs;
    const q = query.trim().toLowerCase();
    if (q) list = list.filter((l) => (l.account ?? "").toLowerCase().includes(q) || (l.model ?? "").toLowerCase().includes(q) || String(l.status).includes(q));
    if (statusFilter !== "all") list = list.filter((l) => (statusFilter === "ok" ? l.status >= 200 && l.status < 300 : statusFilter === "err" ? l.status >= 400 : l.status === 429));
    return list;
  }, [logs, query, statusFilter]);

  function exportCsv() {
    const header = "ts,region,account,model,status,latencyMs,prompt,completion,stream\n";
    const rows = visible.map((l) => `${l.ts},${l.region},${l.account ?? ""},${l.model ?? ""},${l.status},${l.latencyMs},${l.promptTokens ?? ""},${l.completionTokens ?? ""},${l.stream ? "true" : "false"}`).join("\n");
    const blob = new Blob([header + rows], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `gateway-logs-${Date.now()}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <div className="mx-auto max-w-6xl space-y-4 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("wbStats.gateway.requestLog")}</h1>
          <p className="text-xs text-muted-foreground">{visible.length} / {logs.length} entries</p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={onRefresh} disabled={loading}>{loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}{t("growth.actions.refresh", { fallback: "Refresh" })}</Button>
          <Button variant="outline" onClick={exportCsv} disabled={!visible.length}><Download className="mr-2 size-4" />CSV</Button>
          <Button variant="destructive" onClick={onClear} disabled={clearing || !logs.length}>{clearing ? <Loader2 className="mr-2 size-4 animate-spin" /> : <Eraser className="mr-2 size-4" />}{t("wbStats.gateway.clear")}</Button>
        </div>
      </div>

      <Card className="p-4">
        <div className="flex flex-wrap items-center gap-2">
          <Input placeholder={t("growth.actions.refresh", { fallback: "Search account / model / status" })} value={query} onChange={(e) => setQuery(e.target.value)} className="max-w-xs" />
          <select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)} className="rounded-md border border-border bg-background px-2 py-1 text-xs">
            <option value="all">All</option>
            <option value="ok">2xx</option>
            <option value="err">4xx/5xx</option>
            <option value="429">429</option>
          </select>
        </div>
      </Card>

      <Card className="gap-0 py-0">
        <div className="border-b border-border/60 px-5 py-3">
          <Tabs value={region} onValueChange={(v) => setRegion(v as Region)}>
            <TabsList>
              {REGIONS.map((r) => (<TabsTrigger key={r} value={r}>{regionDescriptor(r).versionLabel}</TabsTrigger>))}
            </TabsList>
          </Tabs>
        </div>
        <div className="px-5 py-3">
          {!visible.length ? (<p className="py-4 text-sm text-muted-foreground">{t("wbStats.gateway.noRequestsLog")}</p>) : (
            <div className="h-[calc(100vh-320px)] overflow-y-auto">
              <div className="space-y-2">
                {visible.map((entry, idx) => (
                  <div key={`${entry.ts}-${idx}`} className="grid grid-cols-[auto_auto_minmax(0,1fr)_auto_auto_auto] items-center gap-3 border-b border-border/60 py-2 text-xs last:border-b-0 tabular-nums">
                    <span className="text-muted-foreground">{fmtTime(entry.ts)}</span>
                    <span className="text-muted-foreground">{regionDescriptor(entry.region).versionLabel}</span>
                    <span className="truncate font-medium" title={entry.account ?? undefined}>{entry.account || "—"} {entry.model ? <span className="ml-2 font-normal text-muted-foreground">{entry.model}</span> : null}</span>
                    <span className={entry.status >= 200 && entry.status < 300 ? "text-emerald-600" : entry.status === 429 ? "text-amber-600" : "text-destructive"}>{entry.status}</span>
                    <span className="text-muted-foreground">{entry.latencyMs ? `${(entry.latencyMs/1000).toFixed(1)}s` : "—"}</span>
                    <span className="text-right text-muted-foreground">{(entry.promptTokens ?? 0) + (entry.completionTokens ?? 0)} tok</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}
