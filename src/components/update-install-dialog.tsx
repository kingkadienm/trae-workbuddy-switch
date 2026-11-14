import { useEffect, useRef, useState } from "react";
import { ArrowUpCircle, ExternalLink, Loader2, RefreshCw, TriangleAlert } from "lucide-react";
import type { DownloadEvent } from "@tauri-apps/plugin-updater";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { ReleaseNotesView } from "@/components/release-notes-view";
import * as api from "@/lib/api";
import { resolveNotesFor } from "@/lib/bundled-notes";
import { t as translate, useLocale, useT } from "@/lib/i18n";
import { formatNotesDate, formatNotesSize } from "@/lib/release-notes";
import { GITHUB_RELEASE_URL, openReleaseUrl } from "@/lib/update";
import { markUpdateSeen } from "@/lib/update-preference";
import type { UpdateInfo } from "@/lib/types";

/**
 * 自动更新的弹窗 —— **先说明，后动作**。
 *
 * ## 为什么不再「打开就下载」
 *
 * 改造前：`open` 一变就在 `useEffect` 里 `downloadAndInstall()`。两个后果 ——
 * ① 用户根本没机会读「这一版改了什么」（用户原话：「每次升级都不知道这个版本的变化内容」）；
 * ② **不可逆动作发生在读懂之前**：整包替换 + 装完要重启。
 *
 * 现在默认态是说明态（`available`），下载只能由 `startInstall()` 触发 —— 它只被
 * 「立即升级 / 重试」两个按钮调用。**没有任何 effect 会自己走到下载**，这条由
 * `scripts/check-scope-registry.cjs` 的「说明先于下载」断言钉死（改回去必红）。
 *
 * ## 阶段
 *
 * `available`（说明）→ `checking`（拉签名包）→ `downloading`（进度）→ `ready`（等重启）；
 * 另有 `latest`（点升级时发现已被追上）与 `error`。
 *
 * ## 三个「不做」
 *
 * - **下载中不允许关闭**（`busy`）：Tauri updater 的安装被中断就是半装状态。
 * - **失败态不摆版本说明**：查不到更新源时，缓存的说明与包大小都是旧值，
 *   摆出来等于告诉用户「这次也查到了」。失败只说「为什么 + 怎么办」。
 * - **不把 release body 当 HTML 渲染**：说明来自发布资产，只按结构化纯文本插值
 *   （见 `lib/release-notes.ts` 的归一化）。
 *
 * 设计依据：`docs/specs/UPDATE-NOTES-ARCH.md`。
 */
type UpdateStage = "available" | "checking" | "downloading" | "latest" | "ready" | "error";

const UPDATE_CHECK_TIMEOUT_MS = 15_000;

function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => {
      reject(new Error(translate("wbSettings.update.timeoutError")));
    }, timeoutMs);

    promise.then(resolve, reject).finally(() => window.clearTimeout(timer));
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 从 Tauri updater 返回的原始 manifest 中取出当前平台的安装包地址。 */
function getDownloadUrl(rawJson: Record<string, unknown>): string | null {
  if (typeof rawJson.url === "string") return rawJson.url;
  if (!isRecord(rawJson.platforms)) return null;

  const ua = navigator.userAgent;
  const preferredTargets = ua.includes("Windows")
    ? ["windows-x86_64-nsis", "windows-x86_64"]
    : ua.includes("Linux")
      ? ["linux-x86_64"]
      : ua.includes("Intel")
        ? ["darwin-x86_64", "darwin-aarch64"]
        : ["darwin-aarch64", "darwin-x86_64"];
  for (const target of preferredTargets) {
    const platform = rawJson.platforms[target];
    if (isRecord(platform) && typeof platform.url === "string") return platform.url;
  }

  for (const platform of Object.values(rawJson.platforms)) {
    if (isRecord(platform) && typeof platform.url === "string") return platform.url;
  }
  return null;
}

interface UpdateInstallDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  update: UpdateInfo | null;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function UpdateInstallDialog({ open, onOpenChange, update }: UpdateInstallDialogProps) {
  const t = useT();
  const locale = useLocale();
  const [stage, setStage] = useState<UpdateStage>("available");
  const [error, setError] = useState<string | null>(null);
  const [targetVersion, setTargetVersion] = useState(update?.latest ?? t("wbSettings.update.fallbackVersion"));
  const [received, setReceived] = useState(0);
  const [total, setTotal] = useState(0);
  const [downloadUrl, setDownloadUrl] = useState<string | null>(null);
  const [restarting, setRestarting] = useState(false);

  const releaseUrl = update?.releaseUrl ?? GITHUB_RELEASE_URL;
  /* 网络取不到说明时用**打包内置**的那份兜底 —— 但只在版本号对得上时才用
     （见 `lib/bundled-notes.ts`：拿旧版说明冒充新版比不显示更坏）。 */
  const notes = resolveNotesFor(update);
  const latest = update?.latest;
  const busy = stage === "checking" || stage === "downloading";

  /* 组件卸载 / 弹窗重开时，在途的下载回调不该再写状态。 */
  const cancelledRef = useRef(false);
  useEffect(() => {
    cancelledRef.current = false;
    return () => {
      cancelledRef.current = true;
    };
  }, []);

  /* 打开 / 换目标版本时**只把界面重置到说明态**，不发起任何下载（这就是本次改造的核心）。 */
  useEffect(() => {
    if (!open) return;
    setStage("available");
    setError(null);
    setReceived(0);
    setTotal(0);
    setDownloadUrl(null);
    setRestarting(false);
    setTargetVersion(update?.latest ?? t("wbSettings.update.fallbackVersion"));
  }, [open, update?.latest, t]);

  /** 真正的下载入口：只有「立即升级 / 重试」会调它。 */
  async function startInstall() {
    cancelledRef.current = false;
    setError(null);
    setReceived(0);
    setTotal(0);
    setDownloadUrl(null);
    setRestarting(false);

    if (api.isWebui()) {
      setError(t("wbSettings.update.webuiError"));
      setStage("error");
      return;
    }

    setStage("checking");
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const configuredProxy = await api
        .getGithubConfig()
        .then((config) => config.proxy?.trim() || "")
        .catch(() => "");
      const candidate = await withTimeout(
        check({ proxy: configuredProxy || undefined }),
        UPDATE_CHECK_TIMEOUT_MS,
      );
      if (cancelledRef.current) return;

      if (!candidate) {
        if (update?.hasUpdate) {
          setError(t("wbSettings.update.noSignaturePackage"));
          setStage("error");
        } else {
          setStage("latest");
        }
        return;
      }

      setTargetVersion(candidate.version);
      setDownloadUrl(getDownloadUrl(candidate.rawJson));
      setStage("downloading");
      await candidate.downloadAndInstall((event: DownloadEvent) => {
        if (cancelledRef.current) return;
        if (event.event === "Started") {
          setTotal(event.data.contentLength ?? 0);
          setReceived(0);
        } else if (event.event === "Progress") {
          setReceived((current) => current + event.data.chunkLength);
        }
      });
      if (!cancelledRef.current) setStage("ready");
    } catch (e) {
      if (!cancelledRef.current) {
        setError(api.asError(e));
        setStage("error");
      }
    }
  }

  const percent = total > 0 ? Math.min(100, Math.round((received / total) * 100)) : null;

  /** 关掉弹窗 = 「这一版说明看过了」：下次自动检查不再主动弹同一版（见 `update-preference.ts`）。 */
  function handleOpenChange(next: boolean) {
    if (!next) {
      if (busy) return;
      if (update?.hasUpdate) markUpdateSeen(latest);
    }
    onOpenChange(next);
  }

  async function openRelease() {
    try {
      await openReleaseUrl(releaseUrl);
    } catch (e) {
      setError(api.asError(e));
    }
  }

  async function openDownload() {
    if (!downloadUrl) return;
    try {
      await openReleaseUrl(downloadUrl);
    } catch (e) {
      setError(api.asError(e));
    }
  }

  async function restartApp() {
    setRestarting(true);
    try {
      await api.relaunchApp();
    } catch (e) {
      setRestarting(false);
      setError(t("wbSettings.update.restartFailed", { error: api.asError(e) }));
      setStage("error");
    }
  }

  const title = (() => {
    switch (stage) {
      case "available":
        return t("wbSettings.update.titleAvailable", { version: latest ?? "?" });
      case "checking":
        return t("wbSettings.update.titleChecking");
      case "downloading":
        return t("wbSettings.update.titleUpgrading", { version: targetVersion });
      case "latest":
        return t("wbSettings.update.titleLatest");
      case "ready":
        return t("wbSettings.update.titleSuccess");
      case "error":
        return t("wbSettings.update.titleError");
    }
  })();

  const description = (() => {
    switch (stage) {
      case "available":
        return t("wbSettings.update.descAvailable");
      case "checking":
        return t("wbSettings.update.descChecking");
      case "downloading":
        return t("wbSettings.update.descDownloading");
      case "latest":
        return t("wbSettings.update.descLatest");
      case "ready":
        return t("wbSettings.update.descSuccess");
      case "error":
        return t("wbSettings.update.descError");
    }
  })();

  /* 页头第二行：当前版本在任何阶段都是事实；发布日期与体积只在有可信说明时显示 ——
     失败态里它们来自上一次缓存，摆出来等于骗人。 */
  const meta = (() => {
    const parts = [t("wbSettings.update.metaCurrent", { version: update?.current ?? "?" })];
    const published = stage === "error" ? null : formatNotesDate(notes?.publishedAt, locale);
    if (published) parts.push(t("wbSettings.update.metaPublished", { date: published }));
    const size = stage === "error" ? null : formatNotesSize(notes?.size);
    if (size) parts.push(size);
    return parts.join(" · ");
  })();

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        width="wide"
        flush
        showCloseButton={!busy && !restarting}
        /* 三段式：页头 / 可滚正文 / 固定动作条。`grid-rows` 让正文吃掉剩余高度并自己滚。 */
        className="max-h-[86vh] grid-rows-[auto_minmax(0,1fr)_auto]"
        data-update-stage={stage}
      >
        {/* ⚠️ 不用 `DialogHeader` / `DialogFooter`：它们自带 `flex-col` / `flex-col-reverse`
            与这里要的 `flex-row` 是**同族冲突**，谁生效取决于 Tailwind 生成顺序而不是书写顺序。
            三段式布局自己写，形状才是确定的。`DialogTitle` / `DialogDescription` 照用
            （Radix 的 `aria-labelledby` 需要它们）。 */}
        <div className="flex items-start gap-3 border-b border-border px-4 py-4">
          <span
            className={`grid size-[34px] shrink-0 place-items-center rounded-[10px] ${
              stage === "error" ? "bg-warning/15 text-warning" : "bg-primary/15 text-primary"
            }`}
            aria-hidden="true"
          >
            {stage === "error" ? <TriangleAlert className="size-[17px]" /> : <ArrowUpCircle className="size-[17px]" />}
          </span>
          <div className="min-w-0 flex-1">
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription className="mt-1 tabular-nums">{meta}</DialogDescription>
            <DialogDescription className="mt-0.5">{description}</DialogDescription>
          </div>
        </div>

        <div className="flex min-h-0 flex-col gap-3.5 overflow-y-auto px-4 py-3.5" data-update-body>
          {stage === "checking" && (
            <div className="flex items-center gap-2 rounded-md bg-muted/50 p-3 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              {t("wbSettings.update.connecting")}
            </div>
          )}

          {stage === "downloading" && (
            <div className="space-y-2 rounded-md border p-3">
              <div className="flex items-center justify-between text-sm">
                <span className="flex items-center gap-2">
                  <Loader2 className="size-4 animate-spin text-primary" aria-hidden="true" />
                  {t("wbSettings.update.downloadingLabel")}
                </span>
                <span className="font-mono text-xs text-muted-foreground tabular-nums">
                  {percent === null ? t("wbSettings.update.downloading") : `${percent}%`}
                </span>
              </div>
              <div className="h-2 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full bg-primary transition-[width] duration-200"
                  style={{ width: `${percent ?? 30}%` }}
                />
              </div>
              {total > 0 && (
                <div className="text-right text-xs text-muted-foreground tabular-nums">
                  {formatBytes(received)} / {formatBytes(total)}
                </div>
              )}
            </div>
          )}

          {stage === "ready" && (
            <div className="rounded-md border border-success/30 bg-success/10 p-3 text-sm text-success">
              {t("wbSettings.update.readyToRestart", { version: targetVersion })}
            </div>
          )}

          {stage === "error" && (
            <div className="space-y-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm">
              <p className="text-destructive">{error || t("wbSettings.update.unknownError")}</p>
              <p className="text-xs leading-relaxed text-muted-foreground">
                {t("wbSettings.update.errorProxyHint")}
              </p>
              {downloadUrl && (
                <button
                  type="button"
                  className="flex w-full items-start gap-1 break-all text-left text-primary underline-offset-2 hover:underline"
                  onClick={() => void openDownload()}
                >
                  <ExternalLink className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
                  <span>{downloadUrl}</span>
                </button>
              )}
              <p className="break-all text-xs text-muted-foreground">{releaseUrl}</p>
            </div>
          )}

          {/* 说明只在这些阶段出现：失败态不摆旧数据（见文件头「三个不做」）。 */}
          {stage !== "error" &&
            (notes ? (
              <ReleaseNotesView notes={notes} current={update?.current} />
            ) : (
              <div className="flex flex-col gap-1">
                <p className="text-xs text-muted-foreground">{t("wbSettings.update.notesEmpty")}</p>
                <p className="text-[10.5px] leading-relaxed text-muted-foreground/80">
                  {t("wbSettings.update.notesEmptyHint")}
                </p>
              </div>
            ))}
        </div>

        <div className="flex items-center gap-2 border-t border-border bg-card-2 px-4 py-3">
          {stage === "available" && (
            <>
              <Button variant="ghost" size="sm" onClick={() => void openRelease()}>
                <ExternalLink />
                {t("wbSettings.update.viewFullNotes")}
              </Button>
              <span className="flex-1" />
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {t("wbSettings.update.later")}
              </Button>
              <Button onClick={() => void startInstall()}>
                <ArrowUpCircle />
                {t("wbSettings.update.startNow")}
              </Button>
            </>
          )}

          {busy && (
            <>
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                {t("wbSettings.update.downloadingHint")}
              </span>
              <span className="flex-1" />
              <Button disabled>
                <Loader2 className="animate-spin" />
                {t("wbSettings.update.downloadingBusy")}
              </Button>
            </>
          )}

          {stage === "ready" && (
            <>
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                {t("wbSettings.update.readyToRestart", { version: targetVersion })}
              </span>
              <span className="flex-1" />
              <Button variant="outline" onClick={() => onOpenChange(false)} disabled={restarting}>
                {t("wbSettings.update.restartLater")}
              </Button>
              <Button onClick={() => void restartApp()} disabled={restarting}>
                {restarting ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                {restarting ? t("wbSettings.update.restarting") : t("wbSettings.update.restartNow")}
              </Button>
            </>
          )}

          {stage === "error" && (
            <>
              <Button variant="ghost" size="sm" onClick={() => void openRelease()}>
                <ExternalLink />
                {t("wbSettings.update.openDownloadPage")}
              </Button>
              <span className="flex-1" />
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {t("wbSettings.common.close")}
              </Button>
              <Button onClick={() => void startInstall()}>
                <RefreshCw />
                {t("wbSettings.update.retry")}
              </Button>
            </>
          )}

          {stage === "latest" && (
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              {t("wbSettings.common.close")}
            </Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
