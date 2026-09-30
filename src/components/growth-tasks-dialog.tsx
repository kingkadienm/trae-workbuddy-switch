import { useCallback, useEffect, useState } from "react";
import { Gift, Loader2, RotateCcw, Trophy } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import * as api from "@/lib/api";
import { useT, type Translate } from "@/lib/i18n";
import { cn } from "@/lib/utils";
import type { AccountMeta, GrowthAutoAllItem, GrowthTask, Region } from "@/lib/types";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  account: AccountMeta | null;
  region: Region;
}

/** 任务行在表里的本地状态（行操作后即时反馈，不重拉列表）。 */
interface RowState {
  busy: boolean;
  note: string;
  failed: boolean;
}

/**
 * 单账号成长任务弹窗（panel 任务中心移植）：
 * 合并任务表（默认 + 小程序口径）+ 行操作（接受 / 领取 / 自动完成）
 * + 头部「接受全部」「一键完成」（逐项结果列出）。
 *
 * CN 专有：Global 区域不挂载本弹窗（卡片上的入口本来就不渲染）。
 */
export function GrowthTasksDialog({ open, onOpenChange, account, region }: Props) {
  const t = useT();
  const [tasks, setTasks] = useState<GrowthTask[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [rows, setRows] = useState<Record<string, RowState>>({});
  const [acceptAllBusy, setAcceptAllBusy] = useState(false);
  const [autoAllBusy, setAutoAllBusy] = useState(false);
  const [autoAllResults, setAutoAllResults] = useState<GrowthAutoAllItem[] | null>(null);
  const [autoAllError, setAutoAllError] = useState("");

  const uid = account?.uid ?? "";

  const load = useCallback(async () => {
    if (!uid) return;
    setLoading(true);
    setError("");
    setRows({});
    try {
      const res = await api.growthTasksList(uid, region);
      setTasks(res.tasks ?? []);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
  }, [uid, region]);

  useEffect(() => {
    if (open) {
      setAutoAllResults(null);
      setAutoAllError("");
      void load();
    }
  }, [open, load]);

  function setRow(code: string, patch: Partial<RowState>) {
    setRows((prev) => {
      const base: RowState = prev[code] ?? { busy: false, note: "", failed: false };
      return { ...prev, [code]: { ...base, ...patch } };
    });
  }

  async function doAccept(task: GrowthTask) {
    setRow(task.taskCode, { busy: true });
    try {
      await api.growthTasksAccept(uid, task.taskCode, region);
      setRow(task.taskCode, { note: t("wbAccounts.growth.statusAccepted"), failed: false });
    } catch (e) {
      setRow(task.taskCode, { note: errMsg(e), failed: true });
    } finally {
      setRow(task.taskCode, { busy: false });
    }
  }

  async function doClaim(task: GrowthTask) {
    setRow(task.taskCode, { busy: true });
    try {
      const res = await api.growthTaskClaim(uid, task.taskCode, region);
      const credit = res.credit ?? 0;
      const energy = res.energy ?? 0;
      setRow(task.taskCode, {
        note:
          credit > 0 || energy > 0
            ? t("wbAccounts.growth.reward", { credit, energy })
            : t("wbAccounts.growth.statusClaimed"),
        failed: false,
      });
    } catch (e) {
      setRow(task.taskCode, { note: errMsg(e), failed: true });
    } finally {
      setRow(task.taskCode, { busy: false });
    }
  }

  async function doAuto(task: GrowthTask) {
    setRow(task.taskCode, { busy: true });
    try {
      const res = await api.growthAutoTask(uid, task.taskCode, region);
      const msg = typeof res.message === "string" ? res.message : "";
      setRow(task.taskCode, { note: msg || t("wbAccounts.growth.resultDone"), failed: false });
    } catch (e) {
      setRow(task.taskCode, { note: errMsg(e), failed: true });
    } finally {
      setRow(task.taskCode, { busy: false });
    }
  }

  async function doAcceptAll() {
    setAcceptAllBusy(true);
    setAutoAllResults(null);
    try {
      const res = await api.growthAcceptAll(uid, region);
      toast.success(t("wbAccounts.growth.acceptAllDone", { n: res.accepted ?? 0 }));
      void load();
    } catch (e) {
      toast.error(errMsg(e));
    } finally {
      setAcceptAllBusy(false);
    }
  }

  async function doAutoAll() {
    setAutoAllBusy(true);
    setAutoAllResults(null);
    setAutoAllError("");
    try {
      const res = await api.growthAutoAll(uid, region);
      setAutoAllResults(res.results ?? []);
      void load();
    } catch (e) {
      setAutoAllError(errMsg(e));
    } finally {
      setAutoAllBusy(false);
    }
  }

  const name = account?.nickname || account?.uid || "";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            <Gift className="size-4" aria-hidden="true" />
            {t("wbAccounts.growth.dialogTitle")}
            {name && <span className="ml-2 text-sm font-normal text-muted-foreground">{name}</span>}
          </DialogTitle>
          <DialogDescription>{t("wbAccounts.growth.dialogDesc")}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" onClick={doAcceptAll} disabled={acceptAllBusy || loading || !uid}>
            {acceptAllBusy ? <Loader2 className="size-3.5 animate-spin" /> : <Trophy className="size-3.5" />}
            {t("wbAccounts.growth.acceptAll")}
          </Button>
          <Button size="sm" onClick={doAutoAll} disabled={autoAllBusy || loading || !uid}>
            {autoAllBusy ? <Loader2 className="size-3.5 animate-spin" /> : <RotateCcw className="size-3.5" />}
            {t("wbAccounts.growth.autoAll")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading} className="ml-auto">
            {loading ? <Loader2 className="size-3.5 animate-spin" /> : <RotateCcw className="size-3.5" />}
            {t("wbAccounts.growth.refresh")}
          </Button>
        </div>

        {error && (
          <Alert variant="destructive">
            <AlertTitle>{t("wbAccounts.growth.opFail")}</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {loading && tasks.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">{t("wbAccounts.growth.loading")}</p>
        ) : tasks.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">{t("wbAccounts.growth.empty")}</p>
        ) : (
          <div className="max-h-72 space-y-1.5 overflow-y-auto pr-1">
            {tasks.map((task) => {
              const rs = rows[task.taskCode];
              const acceptStatus = task.acceptStatus ?? "";
              return (
                <div key={task.taskCode} className="flex items-center gap-2 rounded-lg border border-border/60 bg-muted/20 px-3 py-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-xs font-medium" title={task.title || task.taskCode}>
                        {task.title || task.taskCode}
                      </span>
                      {acceptStatus === "accepted" && (
                        <Badge variant="secondary" className="text-[10px]">{t("wbAccounts.growth.statusAccepted")}</Badge>
                      )}
                      {Boolean(task.claimable) && (
                        <Badge className="text-[10px]">{t("wbAccounts.growth.statusClaimable")}</Badge>
                      )}
                      {Boolean(task.claimed) && (
                        <Badge variant="outline" className="text-[10px]">{t("wbAccounts.growth.statusClaimed")}</Badge>
                      )}
                      {Boolean(task.locked) && (
                        <Badge variant="outline" className="text-[10px] text-muted-foreground">{t("wbAccounts.growth.statusLocked")}</Badge>
                      )}
                    </div>
                    {typeof task.current === "number" && typeof task.target === "number" && task.target > 0 && (
                      <p className="text-[11px] tabular-nums text-muted-foreground">
                        {task.current}/{task.target}
                        {typeof task.credit === "number" || typeof task.energy === "number"
                          ? ` · ${t("wbAccounts.growth.reward", { credit: task.credit ?? 0, energy: task.energy ?? 0 })}`
                          : ""}
                      </p>
                    )}
                    {rs?.note && (
                      <p className={cn("truncate text-[11px]", rs.failed ? "text-destructive" : "text-muted-foreground")}>{rs.note}</p>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    {rs?.busy ? (
                      <Loader2 className="size-3.5 animate-spin text-muted-foreground" />
                    ) : (
                      <>
                        {acceptStatus !== "accepted" && !task.claimed && (
                          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={() => void doAccept(task)}>
                            {t("wbAccounts.growth.accept")}
                          </Button>
                        )}
                        {task.claimable && (
                          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={() => void doClaim(task)}>
                            {t("wbAccounts.growth.claim")}
                          </Button>
                        )}
                        <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" disabled={Boolean(task.locked)} onClick={() => void doAuto(task)}>
                          {t("wbAccounts.growth.auto")}
                        </Button>
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {(autoAllResults || autoAllError) && (
          <div className="space-y-1.5">
            <p className="text-xs font-semibold text-muted-foreground">{t("wbAccounts.growth.resultTitle")}</p>
            {autoAllError && (
              <Alert variant="destructive">
                <AlertDescription>{autoAllError}</AlertDescription>
              </Alert>
            )}
            {autoAllResults?.map((item, i) => (
              <div key={i} className="flex items-center gap-2 rounded-md border border-border/60 bg-muted/20 px-2.5 py-1.5 text-xs">
                <AutoAllStatusBadge t={t} status={item.status} />
                <span className="truncate font-medium" title={item.taskCode}>{item.desc || item.taskCode}</span>
                {item.message && (
                  <span className="min-w-0 flex-1 truncate text-muted-foreground" title={item.message}>
                    {item.message}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("wbAccounts.dialog.cancel")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** 一键完成逐项结果的状态徽标（done / skipped / error）。 */
function AutoAllStatusBadge({ t, status }: { t: Translate; status: string }) {
  const key =
    status === "done"
      ? "wbAccounts.growth.resultDone"
      : status === "skipped"
        ? "wbAccounts.growth.resultSkipped"
        : "wbAccounts.growth.resultError";
  return (
    <Badge variant={status === "done" ? "default" : status === "skipped" ? "secondary" : "destructive"} className="shrink-0 text-[10px]">
      {t(key)}
    </Badge>
  );
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
