import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Play, ScanSearch, Users } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import * as api from "@/lib/api";
import { useT, type Translate } from "@/lib/i18n";
import type { GrowthQueueStatus, GrowthScanResult, Region } from "@/lib/types";

interface Props {
  region: Region;
  /** 任一队列项完成后刷新该账号积分（奖励即时可见）。 */
  onCreditRefresh?: () => void;
}

/** 轮询间隔（对照 panel 任务中心）。 */
const POLL_MS = 3000;

/**
 * 全账号成长任务中心（panel 任务中心移植；CN 专有）。
 *
 * 三段：「扫描」（各账号待办一览）→「执行全部待办」（并发 1-4 选择 + 启动）
 * → 队列表（状态徽标，3s 轮询；`seq` 变化时重置扫描视图，避免旧轮残留）。
 *
 * 双进程说明：桌面端与 WebUI-server 队列状态互不共享，约定「从一处发起」。
 */
export function GrowthTaskCenter({ region, onCreditRefresh }: Props) {
  const t = useT();
  const [scan, setScan] = useState<GrowthScanResult | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState("");
  const [conc, setConc] = useState("2");
  const [queue, setQueue] = useState<GrowthQueueStatus | null>(null);
  const [running, setRunning] = useState(false);
  const [starting, setStarting] = useState(false);
  /** 已报过「完成」toast 的 seq（同一轮只报一次）。 */
  const reportedSeqRef = useRef(-1);
  /** 最近一次见到的队列 seq（变化 = 新一轮启动 ⇒ 重置扫描视图）。 */
  const lastSeqRef = useRef<number | null>(null);

  const doScan = useCallback(async () => {
    setScanning(true);
    setScanError("");
    try {
      setScan(await api.growthTasksScanAll(region));
    } catch (e) {
      setScanError(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  }, [region]);

  useEffect(() => {
    void doScan();
  }, [doScan]);

  const poll = useCallback(async () => {
    try {
      const status = await api.growthQueueStatus();
      setQueue(status);
      setRunning(status.running);
      if (lastSeqRef.current !== null && status.seq !== lastSeqRef.current) {
        // 新一轮启动（本页或定时任务发起）：重置「完成汇报」并刷新待办视图。
        reportedSeqRef.current = -1;
        void doScan();
      }
      lastSeqRef.current = status.seq;
      // 队列刚结束且本轮未汇报过：报一次结果并刷新积分（奖励即时可见）。
      if (!status.running && status.started && status.total > 0 && reportedSeqRef.current !== status.seq) {
        reportedSeqRef.current = status.seq;
        const done = status.items.filter((i) => i.status === "done").length;
        const failed = status.items.filter((i) => i.status === "error").length;
        const skipped = status.items.filter((i) => i.status === "skipped").length;
        toast.success(t("wbAccounts.growth.queueDone", { done, error: failed, skipped }));
        onCreditRefresh?.();
      }
    } catch {
      // 轮询失败静默（下一轮再试）。
    }
  }, [onCreditRefresh, t]);

  useEffect(() => {
    if (!running && !starting) return;
    poll();
    const timer = window.setInterval(poll, POLL_MS);
    return () => window.clearInterval(timer);
  }, [running, starting, poll]);

  const startQueue = async () => {
    setStarting(true);
    try {
      const res = await api.growthRunQueue(region, Number(conc));
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

  return (
    <section aria-label={t("wbAccounts.page.taskCenter")} className="rounded-2xl border border-border bg-muted/30 px-5 py-4">
      <div className="flex flex-wrap items-center gap-2.5">
        <Users className="size-4 text-muted-foreground" aria-hidden="true" />
        <div className="min-w-0">
          <h3 className="text-sm font-semibold leading-tight">{t("wbAccounts.page.taskCenter")}</h3>
          <p className="truncate text-[11px] leading-tight text-muted-foreground">{t("wbAccounts.growth.centerDesc")}</p>
        </div>
        <div className="ml-auto flex items-center gap-2">
          <Button variant="outline" size="sm" className="h-7 text-xs" onClick={() => void doScan()} disabled={scanning}>
            {scanning ? <Loader2 className="size-3.5 animate-spin" /> : <ScanSearch className="size-3.5" />}
            {t("wbAccounts.growth.scan")}
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

      {scanError && <p className="mt-2 text-xs text-destructive">{scanError}</p>}

      {scan && scan.accounts.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-2">
          {scan.accounts.map((acc) => {
            const n = acc.growth?.length ?? 0;
            return (
              <Badge
                key={acc.uid}
                variant={n > 0 ? "default" : "secondary"}
                className="text-[11px] tabular-nums"
                title={acc.nickname || acc.uid}
              >
                {acc.nickname || acc.uid}
                {n > 0 ? ` ${t("wbAccounts.growth.pendingCount", { n })}` : acc.growthError ? ` · ${t("wbAccounts.growth.scanError")}` : ""}
              </Badge>
            );
          })}
          {typeof scan.pendingCount === "number" && scan.pendingCount === 0 && (
            <span className="text-xs text-muted-foreground">{t("wbAccounts.growth.noPending")}</span>
          )}
        </div>
      )}

      {(queue?.running || queueDone) && queue && (
        <div className="mt-3 max-h-40 overflow-y-auto rounded-lg border border-border/60 bg-background/60">
          {queue.items.map((item, i) => (
            <div key={`${item.uid}-${item.code}-${i}`} className="flex items-center gap-2 border-b border-border/40 px-3 py-1.5 text-xs last:border-b-0">
              <QueueStatusBadge t={t} status={item.status} />
              <span className="max-w-[10rem] truncate font-medium" title={item.uid}>{item.nickname || item.uid}</span>
              <span className="truncate text-muted-foreground" title={item.code}>{item.code}</span>
              {item.message && <span className="min-w-0 flex-1 truncate text-muted-foreground" title={item.message}>{item.message}</span>}
            </div>
          ))}
          {!queue.running && queue.started && (
            <p className="px-3 py-1.5 text-xs text-muted-foreground">
              {t("wbAccounts.growth.queueIdle")}
            </p>
          )}
        </div>
      )}
    </section>
  );
}

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
