/**
 * 「清除缓存」卡片（设置页「备份与恢复」分区）。
 *
 * 顺序即流程：**扫描 → 勾选 → 二次确认（列出类别与体积）→ 清除 → 重新扫描**。
 *
 * ## 硬约束
 *
 * - **先扫描再勾选**：每类旁显示真实体积（`cache_scan`，只读），不猜不写死；
 * - **无数据不渲染勾选**：某类 0 字节 ⇒ 复选框禁用（无可清除内容，见 §3.4 规则 1）；
 * - **清除不可逆**：确认弹窗列出所选类别 + 合计体积，并如实说明
 *   「账号、配置与用量账本不受影响」——不写「安全」「放心」这类无法兑现的词；
 * - **部分失败如实报**：`errors` 非空 ⇒  toast 报「已释放 X，N 个项目未能清除」，
 *   不假装全部成功；随后**重新扫描**（界面回到真实状态）。
 */

import { useCallback, useEffect, useState } from "react";
import { AlertTriangle, HardDrive, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { SettingsGroup } from "@/components/settings-primitives";
import * as api from "@/lib/api";
import { displayText } from "@/lib/display-text";
import { useT } from "@/lib/i18n";
import type { CacheCategoryId, CacheScanReport } from "@/lib/types";
import type { TranslationKey } from "@/locales/zh";

/** 类别展示顺序（与 core `cache::CATEGORY_IDS` 一致）。 */
const CATEGORY_ORDER: CacheCategoryId[] = ["session_backups", "snapshots", "caches", "logs"];

const LABEL_KEY: Record<CacheCategoryId, TranslationKey> = {
  session_backups: "cache.category.sessionBackups",
  snapshots: "cache.category.snapshots",
  caches: "cache.category.caches",
  logs: "cache.category.logs",
};

const DESC_KEY: Record<CacheCategoryId, TranslationKey> = {
  session_backups: "cache.category.sessionBackupsDesc",
  snapshots: "cache.category.snapshotsDesc",
  caches: "cache.category.cachesDesc",
  logs: "cache.category.logsDesc",
};

/** 字节量级（`—` 表示无数据；不猜）。 */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "—";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

/** 「清除缓存」分区。 */
export function CacheSection() {
  const t = useT();

  const [report, setReport] = useState<CacheScanReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<CacheCategoryId[]>([]);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [clearing, setClearing] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setReport(await api.cacheScan());
    } catch (e) {
      toast.error(t("cache.loadFailed"), { description: api.asError(e) });
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const stats = report?.categories ?? [];
  const statOf = (id: CacheCategoryId) => stats.find((s) => s.id === id);
  const selectedBytes = selected.reduce((sum, id) => sum + (statOf(id)?.bytes ?? 0), 0);
  const hasSelection = selected.length > 0;

  function toggle(id: CacheCategoryId) {
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  }

  async function runClear() {
    if (!hasSelection || clearing) return;
    setClearing(true);
    try {
      const result = await api.cacheClear(selected);
      const errors = Array.isArray(result.errors) ? result.errors : [];
      if (errors.length > 0) {
        // 部分失败：如实报（不假装全部成功）。
        toast.warning(
          t("cache.clearedPartial", {
            size: formatBytes(result.freedBytes ?? 0),
            count: errors.length,
          }),
        );
      } else {
        toast.success(t("cache.cleared", { size: formatBytes(result.freedBytes ?? 0) }));
      }
      setConfirmOpen(false);
      setSelected([]);
      await load();
    } catch (e) {
      toast.error(t("cache.clearFailed"), { description: api.asError(e) });
    } finally {
      setClearing(false);
    }
  }

  return (
    <SettingsGroup id="settings-cache-clear" title={t("cache.title")}>
      <CardContent className="space-y-0 p-0">
        <p className="border-b border-border/60 bg-muted/25 px-4 py-3 text-xs leading-5 text-muted-foreground">
          {t("cache.intro")}
        </p>

        {loading ? (
          <div className="space-y-2 px-4 py-3">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
          </div>
        ) : (
          <div className="divide-y divide-border/50">
            {CATEGORY_ORDER.map((id) => {
              const stat = statOf(id);
              const bytes = stat?.bytes ?? 0;
              const label = t(LABEL_KEY[id]);
              return (
                <div key={id} className="flex flex-col gap-2 px-4 py-2.5 sm:flex-row sm:items-start sm:justify-between">
                  <label className="flex min-w-0 cursor-pointer items-start gap-2.5">
                    <Checkbox
                      checked={selected.includes(id)}
                      onCheckedChange={() => toggle(id)}
                      disabled={bytes <= 0}
                      aria-checked={selected.includes(id)}
                      aria-label={t("cache.aria", { category: label })}
                    />
                    <span className="min-w-0">
                      <span className="block text-[13px] font-medium">{label}</span>
                      <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
                        {t(DESC_KEY[id])}
                      </span>
                    </span>
                  </label>
                  <div className="shrink-0 pl-6 text-xs tabular-nums text-muted-foreground sm:pl-0 sm:text-right">
                    <div className="font-medium text-foreground/80">{formatBytes(bytes)}</div>
                    {(stat?.entries ?? 0) > 0 && (
                      <div className="mt-0.5">{t("cache.entries", { count: stat?.entries ?? 0 })}</div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}

        <div className="border-t border-border/60 px-4 py-3 text-xs tabular-nums text-muted-foreground">
          {t("cache.total", { size: formatBytes(report?.totalBytes ?? 0) })}
          {" · "}
          {t("cache.storePath", { path: displayText(report?.storePath) ?? "—" })}
        </div>

        <div className="flex flex-wrap items-center gap-2 border-t border-border/60 px-4 py-3">
          <Button
            size="sm"
            variant="destructive"
            onClick={() => setConfirmOpen(true)}
            disabled={!hasSelection || loading || clearing}
          >
            {clearing ? <Loader2 className="animate-spin" /> : <Trash2 />}
            {clearing ? t("cache.clearing") : t("cache.clear")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void load()} disabled={loading || clearing}>
            {loading ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            {t("cache.refresh")}
          </Button>
          {!hasSelection && !loading && (
            <span className="text-xs text-muted-foreground">{t("cache.clearNone")}</span>
          )}
        </div>
      </CardContent>

      {/* 二次确认：列出所选类别与合计体积（不可逆操作） */}
      <Dialog open={confirmOpen} onOpenChange={(open) => !clearing && setConfirmOpen(open)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("cache.confirmTitle")}</DialogTitle>
            <DialogDescription>
              {t("cache.confirmBody", { size: formatBytes(selectedBytes) })}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            {selected.map((id) => (
              <div key={id} className="flex items-center justify-between gap-3 text-[13px]">
                <span className="flex min-w-0 items-center gap-2">
                  <HardDrive className="shrink-0 text-muted-foreground" />
                  <span className="truncate">{t(LABEL_KEY[id])}</span>
                </span>
                <span className="shrink-0 tabular-nums text-muted-foreground">
                  {formatBytes(statOf(id)?.bytes ?? 0)}
                </span>
              </div>
            ))}
          </div>
          <Alert variant="destructive">
            <AlertTriangle />
            <AlertDescription>{t("cache.confirmBody", { size: formatBytes(selectedBytes) })}</AlertDescription>
          </Alert>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmOpen(false)} disabled={clearing}>
              {t("cache.confirmCancel")}
            </Button>
            <Button variant="destructive" onClick={() => void runClear()} disabled={clearing}>
              {clearing ? <Loader2 className="animate-spin" /> : null}
              {t("cache.confirmOk")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsGroup>
  );
}
