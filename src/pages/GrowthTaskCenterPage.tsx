import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Play, RefreshCcw, ScanSearch, Users } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import * as api from "@/lib/api";
import { useT, type Translate } from "@/lib/i18n";
import type { GrowthQueueStatus, GrowthScanResult, GrowthTask, Region } from "@/lib/types";

/** 轮询间隔（对照 panel 任务中心）。 */
const POLL_MS = 3000;
/** 成长任务仅 CN 区域。 */
const REGION = "cn" as const;

/** 单账号任务区（替代旧折叠的 `AccountTasks`）。 */
function AccountSection({ account, region, onCreditRefresh }: {
  account: { uid: string; nickname?: string; growth?: GrowthTask[]; growthError?: string };
  region: Region;
  onCreditRefresh?: () => void;
}) {
  const t = useT();
  const [tasks, setTasks] = useState<GrowthTask[]>(account.growth ?? []);
  const [loading, setLoading] = useState(false);
  const [rows, setRows] = useState<Record<string, { busy: boolean; note: string; failed: boolean }>>({});

  // scan 刷新时同步本地 tasks
  useEffect(() => {
    setTasks(account.growth ?? []);
  }, [account.growth]);

  const loadDetail = useCallback(async () => {
    setLoading(true);
    try {
      const res = await api.growthTasksList(account.uid, region);
      setTasks(res.tasks ?? []);
    } catch {
      // 静默
    } finally {
      setLoading(false);
    }
  }, [account.uid, region]);

  // 有待办时自动拉取详情
  const pendingInScan = account.growth?.filter((x) => !x.claimed && x.status !== "completed").length ?? 0;
  useEffect(() => {
    if (pendingInScan > 0) void loadDetail();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingInScan]);

  const setRow = (code: string, patch: { busy?: boolean; note?: string; failed?: boolean }) => {
    setRows((prev) => {
      const base = prev[code] ?? { busy: false, note: "", failed: false };
      return { ...prev, [code]: { ...base, ...patch } };
    });
  };

  const doAccept = async (task: GrowthTask) => {
    setRow(task.taskCode, { busy: true });
    try {
      await api.growthTasksAccept(account.uid, task.taskCode, region);
      setRow(task.taskCode, { note: t("wbAccounts.growth.statusAccepted"), failed: false });
      await loadDetail();
    } catch (e) {
      setRow(task.taskCode, { note: errMsg(e), failed: true });
    } finally {
      setRow(task.taskCode, { busy: false });
    }
  };

  const doClaim = async (task: GrowthTask) => {
    setRow(task.taskCode, { busy: true });
    try {
      const res = await api.growthTaskClaim(account.uid, task.taskCode, region);
      const credit = res.credit ?? 0;
      const energy = res.energy ?? 0;
      setRow(task.taskCode, {
        note: credit > 0 || energy > 0 ? t("wbAccounts.growth.reward", { credit, energy }) : t("wbAccounts.growth.statusClaimed"),
        failed: false,
      });
      onCreditRefresh?.();
      await loadDetail();
    } catch (e) {
      setRow(task.taskCode, { note: errMsg(e), failed: true });
    } finally {
      setRow(task.taskCode, { busy: false });
    }
  };

  const doAuto = async (task: GrowthTask) => {
    setRow(task.taskCode, { busy: true });
    try {
      const res = await api.growthAutoTask(account.uid, task.taskCode, region);
      const msg = typeof res.message === "string" ? res.message : "";
      setRow(task.taskCode, { note: msg || t("wbAccounts.growth.resultDone"), failed: false });
      await loadDetail();
    } catch (e) {
      setRow(task.taskCode, { note: errMsg(e), failed: true });
    } finally {
      setRow(task.taskCode, { busy: false });
    }
  };

  const doAcceptAll = async () => {
    setLoading(true);
    try {
      const res = await api.growthAcceptAll(account.uid, region);
      toast.success(t("wbAccounts.growth.acceptAllDone", { n: res.accepted ?? 0 }));
      await loadDetail();
    } catch (e) {
      toast.error(errMsg(e));
    } finally {
      setLoading(false);
    }
  };

  const doAutoAll = async () => {
    setLoading(true);
    try {
      const res = await api.growthAutoAll(account.uid, region);
      toast.success(t("wbAccounts.growth.autoAllDone", { n: res.results?.length ?? 0 }));
      await loadDetail();
    } catch (e) {
      toast.error(errMsg(e));
    } finally {
      setLoading(false);
    }
  };

  const pending = tasks.filter((x) => !x.claimed && x.status !== "completed");
  const done = tasks.filter((x) => x.claimed || x.status === "completed");

  return (
    <Card className="overflow-hidden">
      {/* 账号头 */}
      <div className="flex flex-wrap items-center gap-2 border-b border-border/40 px-4 py-2.5">
        <Users className="size-3.5 text-muted-foreground" />
        <span className="text-sm font-medium">{account.nickname || account.uid}</span>
        <span className="text-[11px] text-muted-foreground">
          {t("wbAccounts.growth.pending", { fallback: "pending" })}: {pending.length} · {t("wbAccounts.growth.completed", { fallback: "completed" })}: {done.length}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          <Button variant="outline" size="sm" className="h-6 text-[10px] px-2" onClick={doAcceptAll} disabled={loading || pending.length === 0}>
            {t("wbAccounts.growth.acceptAll")}
          </Button>
          <Button variant="outline" size="sm" className="h-6 text-[10px] px-2" onClick={doAutoAll} disabled={loading || pending.length === 0}>
            {t("wbAccounts.growth.autoAll")}
          </Button>
          <Button variant="ghost" size="sm" className="h-6 w-6 p-0" onClick={loadDetail} disabled={loading} title={t("wbAccounts.growth.refresh")}>
            {loading ? <Loader2 className="size-3 animate-spin" /> : <RefreshCcw className="size-3" />}
          </Button>
        </div>
      </div>

      {account.growthError && <p className="px-4 py-1.5 text-[11px] text-destructive">{account.growthError}</p>}

      {/* 任务列表 */}
      <div className="divide-y divide-border/40">
        {tasks.length === 0 && !loading && (
          <p className="px-4 py-6 text-center text-xs text-muted-foreground">{t("wbAccounts.growth.empty", { fallback: "No tasks" })}</p>
        )}
        {tasks.map((task) => {
          const code = task.taskCode ?? `task-${task.title ?? ""}`;
          const title = task.title ?? code;
          const acceptStatus = task.acceptStatus ?? "";
          const rs = rows[code];
          return (
            <div key={code} className="flex items-center gap-3 px-4 py-2.5">
              {/* 左：标题 + 进度 + 奖励 */}
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <span className="truncate text-xs font-medium" title={title}>{title}</span>
                  {acceptStatus === "accepted" && <Badge variant="secondary" className="text-[10px]">{t("wbAccounts.growth.statusAccepted")}</Badge>}
                  {Boolean(task.claimable) && <Badge className="text-[10px]">{t("wbAccounts.growth.statusClaimable")}</Badge>}
                  {Boolean(task.claimed) && <Badge variant="outline" className="text-[10px]">{t("wbAccounts.growth.statusClaimed")}</Badge>}
                  {Boolean(task.locked) && <Badge variant="outline" className="text-[10px] text-muted-foreground">{t("wbAccounts.growth.statusLocked")}</Badge>}
                </div>
                {typeof task.current === "number" && typeof task.target === "number" && task.target > 0 && (
                  <p className="mt-0.5 text-[11px] tabular-nums text-muted-foreground">
                    {task.current} / {task.target}
                    {(typeof task.credit === "number" || typeof task.energy === "number") && <> · {t("wbAccounts.growth.reward", { credit: task.credit ?? 0, energy: task.energy ?? 0 })}</>}
                  </p>
                )}
                {!task.current && (typeof task.credit === "number" || typeof task.energy === "number") && (
                  <p className="mt-0.5 text-[11px] text-muted-foreground">
                    {t("wbAccounts.growth.reward", { credit: task.credit ?? 0, energy: task.energy ?? 0 })}
                  </p>
                )}
                {rs?.note && <p className={`mt-0.5 truncate text-[11px] ${rs.failed ? "text-destructive" : "text-muted-foreground"}`}>{rs.note}</p>}
              </div>

              {/* 右：操作 */}
              <div className="flex shrink-0 items-center gap-1">
                {rs?.busy ? (
                  <Loader2 className="size-3.5 animate-spin text-muted-foreground" />
                ) : (
                  <>
                    {acceptStatus !== "accepted" && !task.claimed && (
                      <Button variant="ghost" size="sm" className="h-6 px-2 text-[11px]" onClick={() => void doAccept(task)}>
                        {t("wbAccounts.growth.accept")}
                      </Button>
                    )}
                    {Boolean(task.claimable) && (
                      <Button variant="ghost" size="sm" className="h-6 px-2 text-[11px]" onClick={() => void doClaim(task)}>
                        {t("wbAccounts.growth.claim")}
                      </Button>
                    )}
                    <Button variant="ghost" size="sm" className="h-6 px-2 text-[11px]" disabled={Boolean(task.locked)} onClick={() => void doAuto(task)}>
                      {t("wbAccounts.growth.auto")}
                    </Button>
                  </>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </Card>
  );
}

/** 队列状态徽标（与 panel 任务中心共用）。 */
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

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** 独立页：成长任务中心（左侧菜单入口）。 */
export default function GrowthTaskCenterPage() {
  const t = useT();
  const [scan, setScan] = useState<GrowthScanResult | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState("");
  const [queue, setQueue] = useState<GrowthQueueStatus | null>(null);
  const [running, setRunning] = useState(false);
  const [starting, setStarting] = useState(false);
  const [conc, setConc] = useState("2");
  const runTextRef = useRef("");
  const reportedSeqRef = useRef(-1);
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
      // 静默
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
      const res = await api.growthRunQueue(REGION, Number(conc));
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
  const totalPending = scan
    ? scan.accounts.reduce((sum, acc) => sum + (acc.growth?.filter((x) => !x.claimed && x.status !== "completed").length ?? 0), 0)
    : 0;

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-4">
      {/* 页头 */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("wbAccounts.page.taskCenter")}</h1>
          <p className="text-xs text-muted-foreground">
            {totalPending > 0 ? t("wbAccounts.growth.pendingCount", { n: totalPending }) : t("wbAccounts.growth.noPending")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" className="h-7 text-xs" onClick={() => void doScan()} disabled={scanning}>
            {scanning ? <Loader2 className="size-3.5 animate-spin" /> : <ScanSearch className="size-3.5" />}
            {scanning ? t("wbAccounts.growth.scanning") : t("wbAccounts.growth.scan")}
          </Button>
          <Select value={conc} onValueChange={setConc}>
            <SelectTrigger className="h-7 w-[76px] text-xs" aria-label={t("wbAccounts.growth.concurrency")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {[1, 2, 3, 4].map((n) => (
                <SelectItem key={n} value={String(n)}>
                  {n}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button size="sm" className="h-7 text-xs" onClick={() => void startQueue()} disabled={starting || (queue?.running ?? false)}>
            {starting ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
            {t("wbAccounts.growth.runAll")}
          </Button>
        </div>
      </div>

      {scanError && <p className="text-xs text-destructive">{scanError}</p>}

      {/* 账号任务列表 */}
      {scan && scan.accounts.length > 0 && (
        <div className="space-y-3">
          {scan.accounts.map((acc) => (
            <AccountSection key={acc.uid} account={acc} region={REGION} />
          ))}
        </div>
      )}

      {scan && scan.accounts.length === 0 && !scanError && (
        <Card className="p-8 text-center">
          <p className="text-sm text-muted-foreground">{t("wbAccounts.growth.noAccounts", { fallback: "No accounts" })}</p>
        </Card>
      )}

      {/* 队列 + 日志 */}
      <Tabs defaultValue="queue">
        <TabsList>
          <TabsTrigger value="queue">
            {t("wbAccounts.growth.queue", { fallback: "Queue" })}
          </TabsTrigger>
          <TabsTrigger value="logs">
            {t("wbAccounts.growth.logs", { fallback: "Logs" })}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="queue" className="mt-3">
          {(queue?.running || queueDone) && queue ? (
            <Card className="overflow-hidden">
              <div className="border-b border-border/40 px-4 py-2 text-xs font-medium">
                {queue.running
                  ? t("wbAccounts.growth.queueRunning", { done: 0, total: queue.total ?? 0 })
                  : t("wbAccounts.growth.queueIdle")}
              </div>
              <div className="max-h-60 overflow-y-auto">
                {queue.items.map((item, i) => (
                  <div key={`${item.uid}-${item.code}-${i}`} className="flex items-center gap-2 border-b border-border/30 px-3 py-1.5 text-xs last:border-b-0">
                    <QueueStatusBadge t={t} status={item.status} />
                    <span className="max-w-[10rem] truncate font-medium" title={item.uid}>{item.nickname || item.uid}</span>
                    <span className="truncate text-muted-foreground" title={item.code}>{item.code}</span>
                    {item.message && <span className="min-w-0 flex-1 truncate text-muted-foreground" title={item.message}>{item.message}</span>}
                  </div>
                ))}
              </div>
            </Card>
          ) : (
            <p className="py-4 text-center text-xs text-muted-foreground">{t("wbAccounts.growth.queueIdle", { fallback: "Queue idle" })}</p>
          )}
        </TabsContent>
        <TabsContent value="logs" className="mt-3">
          <Card className="p-4">
            <div className="max-h-96 overflow-y-auto whitespace-pre-wrap font-mono text-xs">
              {runTextRef.current || t("wbAccounts.growth.noLogs", { fallback: "No run logs yet" })}
            </div>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}
