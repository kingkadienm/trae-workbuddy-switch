import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Play, RefreshCcw, Users } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import * as api from "@/lib/api";
import { useT, type Translate } from "@/lib/i18n";
import type { GrowthQueueStatus, GrowthScanResult } from "@/lib/types";

/** 轮询间隔（对照 panel 任务中心）。 */
const POLL_MS = 3000;
/** 成长任务仅 CN 区域。 */
const REGION = "cn" as const;

export default function GrowthTaskCenterPage() {
  const t = useT();
  const [scan, setScan] = useState<GrowthScanResult | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState("");
  const [queue, setQueue] = useState<GrowthQueueStatus | null>(null);
  const [running, setRunning] = useState(false);
  const [starting, setStarting] = useState(false);
  const runTextRef = useRef("");
  /** 已报过「完成」toast 的 seq（同一轮只报一次）。 */
  const reportedSeqRef = useRef(-1);
  /** 最近一次见到的队列 seq（变化 = 新一轮启动 ⇒ 重置扫描视图）。 */
  const lastSeqRef = useRef<number | null>(null);

  const doScan = useCallback(async () => {
    setScanning(true);
    setScanError("");
    try {
      setScan(await api.growthTasksScanAll(REGION));
    } catch (e) {
      setScanError(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    void doScan();
  }, [doScan]);

  const poll = useCallback(async () => {
    try {
      const status = await api.growthQueueStatus();
      setQueue(status);
      setRunning(status.running);
      if (lastSeqRef.current !== null && status.seq !== lastSeqRef.current) {
        reportedSeqRef.current = -1;
        void doScan();
      }
      lastSeqRef.current = status.seq;
      if (!status.running && status.started && status.total > 0 && reportedSeqRef.current !== status.seq) {
        reportedSeqRef.current = status.seq;
        const done = status.items.filter((i) => i.status === "done").length;
        const failed = status.items.filter((i) => i.status === "error").length;
        const skipped = status.items.filter((i) => i.status === "skipped").length;
        toast.success(t("wbAccounts.growth.queueDone", { done, error: failed, skipped }));
        void doScan();
      }
    } catch {
      // 轮询失败静默（下一轮再试）。
    }
  }, [doScan, t]);

  useEffect(() => {
    if (!running && !starting) return;
    poll();
    const timer = window.setInterval(poll, POLL_MS);
    return () => window.clearInterval(timer);
  }, [running, starting, poll]);

  const startQueue = async () => {
    setStarting(true);
    try {
      const res = await api.growthRunQueue(REGION);
      if (res.started && typeof res.seq === "number") {
        setRunning(true);
        void poll();
        toast.info(t("wbAccounts.growth.queueRunning", { done: 0, total: res.total ?? 0 }));
      } else {
        toast.info(res.message ?? t("wbAccounts.growth.noPending"));
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  };

  const queueDone = queue && !queue.running && queue.started ? queue : null;

  // 汇总所有账号待办
  const totalPending = scan
    ? scan.accounts.reduce((sum, acc) => sum + (acc.growth?.filter((t) => !t.claimed && t.status !== "completed").length ?? 0), 0)
    : 0;

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("wbAccounts.page.taskCenter")}</h1>
          <p className="text-xs text-muted-foreground">
            {totalPending > 0
              ? t("wbAccounts.growth.pendingCount", { n: totalPending })
              : t("wbAccounts.growth.noPending")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" className="h-7 text-xs" onClick={() => void doScan()} disabled={scanning}>
            {scanning ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCcw className="size-3.5" />}
            {scanning ? t("wbAccounts.growth.scanning") : t("wbAccounts.growth.scan")}
          </Button>
          <Button size="sm" className="h-7 text-xs" onClick={() => void startQueue()} disabled={starting || (queue?.running ?? false)}>
            {starting ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
            {t("wbAccounts.growth.runAll")}
          </Button>
        </div>
      </div>

      {scanError && <p className="text-xs text-destructive">{scanError}</p>}

      {scan && scan.accounts.length > 0 && (
        <Card className="p-4">
          <div className="mb-3 flex items-center gap-2 text-sm font-medium">
            <Users className="size-4 text-muted-foreground" />
            {t("wbAccounts.growth.scanResult", { fallback: "Scan results" })}
          </div>
          <div className="flex flex-wrap gap-2">
            {scan.accounts.map((acc) => {
              const n = acc.growth?.length ?? 0;
              const pendingN = acc.growth?.filter((t) => !t.claimed && t.status !== "completed").length ?? 0;
              return (
                <Badge
                  key={acc.uid}
                  variant={pendingN > 0 ? "default" : "secondary"}
                  className="text-[11px] tabular-nums"
                  title={acc.nickname || acc.uid}
                >
                  {acc.nickname || acc.uid}
                  {n > 0 ? ` ${t("wbAccounts.growth.pendingCount", { n: pendingN })}` : acc.growthError ? ` · ${t("wbAccounts.growth.scanError")}` : ""}
                </Badge>
              );
            })}
          </div>
        </Card>
      )}

      <Tabs defaultValue="tasks">
        <TabsList>
          <TabsTrigger value="tasks">
            {t("wbAccounts.growth.tasks", { fallback: "Tasks" })}
          </TabsTrigger>
          <TabsTrigger value="logs">
            {t("wbAccounts.growth.logs", { fallback: "Run logs" })}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="tasks">
          <Card className="p-4">
            {scan ? (
              scan.accounts.length === 0 ? (
                <p className="text-xs text-muted-foreground">{t("wbAccounts.growth.noAccounts", { fallback: "No accounts found" })}</p>
              ) : (
                <div className="space-y-3">
                  {scan.accounts.map((acc) => (
                    <AccountTasks key={acc.uid} account={acc} t={t} />
                  ))}
                </div>
              )
            ) : (
              <p className="text-xs text-muted-foreground">{t("wbAccounts.growth.scanFirst", { fallback: "Scan to load tasks" })}</p>
            )}
          </Card>
        </TabsContent>
        <TabsContent value="logs">
          <Card className="p-4">
            <div className="max-h-96 overflow-y-auto whitespace-pre-wrap font-mono text-xs">
              {runTextRef.current || t("wbAccounts.growth.noLogs", { fallback: "No run logs yet" })}
            </div>
          </Card>
        </TabsContent>
      </Tabs>

      {(queue?.running || queueDone) && queue && (
        <Card className="p-4">
          <div className="mb-2 text-sm font-medium">
            {queue.running
              ? t("wbAccounts.growth.queueRunning", { fallback: "Queue running…" })
              : t("wbAccounts.growth.queueIdle", { fallback: "Queue idle" })}
          </div>
          <div className="max-h-60 overflow-y-auto rounded-lg border border-border/60 bg-background/60">
            {queue.items.map((item, i) => (
              <div key={`${item.uid}-${item.code}-${i}`} className="flex items-center gap-2 border-b border-border/40 px-3 py-1.5 text-xs last:border-b-0">
                <QueueStatusBadge t={t} status={item.status} />
                <span className="max-w-[10rem] truncate font-medium" title={item.uid}>{item.nickname || item.uid}</span>
                <span className="truncate text-muted-foreground" title={item.code}>{item.code}</span>
                {item.message && <span className="min-w-0 flex-1 truncate text-muted-foreground" title={item.message}>{item.message}</span>}
              </div>
            ))}
          </div>
        </Card>
      )}
    </div>
  );
}

/** 单个账号的任务列表。 */
function AccountTasks({ account, t }: { account: { uid: string; nickname?: string; growth?: Array<{ taskCode?: string; task_code?: string; title?: string; desc?: string; claimed?: boolean; status?: string }>; growthError?: string }; t: Translate }) {
  const [tasks, setTasks] = useState(account.growth ?? []);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    setTasks(account.growth ?? []);
  }, [account.growth]);

  const loadDetail = async () => {
    setLoading(true);
    try {
      const res = await api.growthTasksList(account.uid, "cn");
      setTasks(res.tasks ?? []);
      setExpanded(true);
    } catch {
      // 静默失败，保持扫描结果
    } finally {
      setLoading(false);
    }
  };

  const pending = tasks.filter((t) => !t.claimed && t.status !== "completed");
  const done = tasks.filter((t) => t.claimed || t.status === "completed");

  return (
    <div className="rounded-lg border border-border/60 p-3">
      <div className="flex items-center justify-between">
        <div>
          <span className="text-sm font-medium">{account.nickname || account.uid}</span>
          <span className="ml-2 text-xs text-muted-foreground">
            {t("wbAccounts.growth.pending", { fallback: "pending" })}: {pending.length}
            {" · "}
            {t("wbAccounts.growth.completed", { fallback: "completed" })}: {done.length}
          </span>
        </div>
        <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={loadDetail} disabled={loading}>
          {loading ? <Loader2 className="size-3 animate-spin" /> : <RefreshCcw className="size-3" />}
        </Button>
      </div>

      {account.growthError && (
        <p className="mt-2 text-xs text-destructive">{account.growthError}</p>
      )}

      {expanded && tasks.length > 0 && (
        <div className="mt-3 max-h-48 space-y-1.5 overflow-y-auto">
          {tasks.map((task, idx) => {
            const code = task.taskCode ?? task.task_code ?? `task-${idx}`;
            const title = task.title ?? code;
            const isPending = !task.claimed && task.status !== "completed";
            return (
              <div key={code} className="flex items-center justify-between rounded-md bg-muted/50 px-2.5 py-1.5 text-xs">
                <div className="min-w-0">
                  <span className="font-medium">{title}</span>
                  {task.desc ? <span className="ml-1.5 text-muted-foreground truncate">· {task.desc}</span> : null}
                </div>
                <div className="shrink-0 ml-2">
                  {isPending ? (
                    <Button size="sm" variant="outline" className="h-6 text-[10px] px-2" onClick={async () => { await api.growthTasksAccept(account.uid, code, "cn"); await loadDetail(); }}>
                      {t("wbAccounts.growth.actions.accept", { fallback: "Accept" })}
                    </Button>
                  ) : (
                    <Button size="sm" variant="secondary" className="h-6 text-[10px] px-2" onClick={async () => { await api.growthTaskClaim(account.uid, code, "cn"); await loadDetail(); }}>
                      {t("wbAccounts.growth.actions.claim", { fallback: "Claim" })}
                    </Button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** 队列状态徽标。 */
const QUEUE_STATUS_KEYS: Record<string, string> = {
  pending: "wbAccounts.growth.status.pending",
  running: "wbAccounts.growth.status.running",
  done: "wbAccounts.growth.status.done",
  skipped: "wbAccounts.growth.status.skipped",
  error: "wbAccounts.growth.status.error",
};

function QueueStatusBadge({ t, status }: { t: Translate; status: string }) {
  const key = QUEUE_STATUS_KEYS[status];
  const variant = status === "done" ? "default" : status === "error" ? "destructive" : status === "running" ? "secondary" : "outline";
  return (
    <Badge variant={variant} className="shrink-0 text-[10px]">
      {key ? (t(key as Parameters<Translate>[0]) ?? key) : status}
    </Badge>
  );
}
