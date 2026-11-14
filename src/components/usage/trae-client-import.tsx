/**
 * 客户端账本 · **导入面板**（Trae，issue #63 平台补齐 · 第二轮）。
 *
 * ## 为什么这个平台需要「导入」这一步
 *
 * Trae 的对话库是 **SQLCipher 加密**的，本工具**不内置解密**（见 `trae/client_import.rs`
 * 模块头）。所以要读客户端那一本账，只能由用户自己先导出一份**明文副本**，再把路径填进来。
 * 面板因此要回答三件事，缺一件用户就卡住：
 *
 * 1. **怎么导出** —— `helpBody` 的 sqlcipher 命令（折叠块，默认收起，不占版面）；
 * 2. **选对了没有** —— 「校验」把后端 `probe` 的结果回显成可读结论
 *    （读得到 / 不是明文库 / 认不出结构 / 没有用量字段），**认不出时把库里的表名列出来**，
 *    否则用户无法反馈该适配什么；
 * 3. **读出来的到底是什么** —— `probeOk` 给「会话 / 消息 / 带用量」三个计数，
 *    「可读」与「有用量」是两件事，分开说。
 *
 * ## 三条约束
 *
 * - 文案**全部**走 `trae.stats.token.client.*`，一个字都不硬编码（中英都要能出）；
 * - 「选择文件」失败（webui / 演示站没有 `plugin-dialog`）**不弹错**：把焦点留在输入框，
 *   Shift+Insert 粘贴路径一样能走通 —— 弹一个「不支持」的红条只会挡住唯一可行的路；
 * - 面板是**受控的哑组件**：不自己取统计，保存成功后只调 `onImported()`，由父页重新取数。
 */

import { useEffect, useRef, useState } from "react";
import { AlertTriangle, CheckCircle2, FileSearch, FolderOpen, KeyRound, Loader2, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { exactTokens } from "@/components/usage/format";
import { useUsageWindowLabel } from "@/components/usage/view-param-bar";
import * as api from "@/lib/api";
import { useT } from "@/lib/i18n";
import type { UsageWindow } from "@/lib/usage-window";

/** 正在进行的动作（同一时刻只允许一个，避免并发改配置）。 */
type Busy = "pick" | "verify" | "save" | "clear" | "unlock" | null;

export function TraeClientImport({
  window: viewWindow,
  error = null,
  onImported,
  onRetry,
}: {
  /** 参数条上的窗口：只用于在卡头回显「现在看的是哪一段」，不参与取数。 */
  window: UsageWindow;
  /** 后端读失败时的判别式错误（未配置时传 `null`）。 */
  error?: api.TraeClientImportError | null;
  /** 保存 / 清除成功后的回调：让父页重新取数。 */
  onImported: () => void;
  /** 「重试」：父页重新读一次这本账（不给就不显示这个按钮）。 */
  onRetry?: () => void;
}) {
  const t = useT();
  const windowLabel = useUsageWindowLabel();
  const [path, setPath] = useState("");
  const [config, setConfig] = useState<api.TraeClientConfig | null>(null);
  const [probe, setProbe] = useState<api.TraeClientProbe | null>(null);
  const [busy, setBusy] = useState<Busy>(null);
  /**
   * 库口令。
   *
   * ⚠️ 只在内存里活到「解锁」那一刻，成功后**立即清空** —— 它不是配置，是一次性凭据，
   * 留在组件状态里等于给了它一个不该有的生命周期（截图、DevTools、内存快照都能读到）。
   */
  const [passphrase, setPassphrase] = useState("");
  /** 内置解密的失败（与「明文档读不出来」是两套 code，分开显示）。 */
  const [vaultError, setVaultError] = useState<api.TraeVaultError | null>(null);
  /** 解锁成功后的结论（含按哪一代兼容性解开的）。 */
  const [unlockNote, setUnlockNote] = useState<string | null>(null);
  /** 保存 / 清除本身的失败（与「库里读不出来」是两件事，分开显示）。 */
  const [failed, setFailed] = useState<string | null>(null);
  /** 本面板里**最后一次成功读通**的时刻（`importedAt`）。 */
  const [readAt, setReadAt] = useState<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  /* 挂载时预填已保存的路径：用户点进来看到的应该是「现在是哪份库」，不是空框。 */
  useEffect(() => {
    let disposed = false;
    api
      .getTraeClientDbConfig()
      .then((result) => {
        if (disposed) return;
        setConfig(result);
        setPath(result.path ?? "");
      })
      .catch(() => {
        /* 读不到配置只让「预填」缺席：输入框仍可手填，保存会重新建立配置。 */
        if (!disposed) setConfig(null);
      });
    return () => {
      disposed = true;
    };
  }, []);

  const typed = path.trim();
  const savedPath = config?.path ?? null;
  /* 只对**已保存的那条路径**判「文件不在了」：用户刚改了半截的新路径不该触发这个提示。 */
  const missingFile = config !== null && savedPath !== null && savedPath === typed && config.exists === false;
  const noPathYet = savedPath === null && typed === "";

  /** `TraeClientImportError` → 一对文案（`missing` / `io` 直接显示后端那句 `message`）。 */
  function errorCopy(source: api.TraeClientImportError): { title: string; hint: string | null } {
    switch (source.code) {
      case "not_plaintext":
        return {
          title: t("trae.stats.token.client.notPlaintext"),
          hint: t("trae.stats.token.client.notPlaintextHint"),
        };
      case "unrecognized":
        return {
          title: t("trae.stats.token.client.unrecognized"),
          hint: t("trae.stats.token.client.unrecognizedHint"),
        };
      case "no_usage":
        return {
          title: t("trae.stats.token.client.noUsage"),
          hint: t("trae.stats.token.client.noUsageHint"),
        };
      case "missing":
      case "io":
        return { title: source.message, hint: null };
    }
  }

  /**
   * 「选择文件」：桌面端走系统对话框；webui / 演示站没有这个能力 ⇒ **静默退回手填**。
   *
   * 动态 import 而不是顶部静态引入：浏览器构建里那条路径根本不存在，
   * 静态引入会让整页在 webui 下直接崩。
   */
  async function pick() {
    if (busy !== null) return;
    setBusy("pick");
    setFailed(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: false });
      if (typeof picked === "string" && picked.trim() !== "") {
        setPath(picked);
        setProbe(null);
      }
    } catch {
      inputRef.current?.focus();
    } finally {
      setBusy(null);
    }
  }

  /** 「校验」：只探测，**不改配置**（用户确认可读之后才点保存）。 */
  async function verify() {
    if (busy !== null || typed === "") return;
    setBusy("verify");
    setProbe(null);
    setFailed(null);
    try {
      const result = await api.probeTraeClientDb(typed);
      setProbe(result);
      if (result.ok) setReadAt(Date.now());
    } catch (e) {
      /* 命令本身失败（旧版本 / 演示站）按 `io` 归类：就是一次往返失败，不是「没配置」。 */
      setProbe({ ok: false, path: typed, tables: [], error: { code: "io", message: api.asError(e) } });
    } finally {
      setBusy(null);
    }
  }

  /** `TraeVaultError` → 一对文案（`io` / `missing` 直接用后端那句 `message`）。 */
  function vaultErrorCopy(source: api.TraeVaultError): { title: string; hint: string | null } {
    switch (source.code) {
      case "wrong_key":
        return {
          title: t("trae.stats.token.client.wrongKey"),
          hint: t("trae.stats.token.client.wrongKeyHint"),
        };
      case "already_plaintext":
        return {
          title: t("trae.stats.token.client.alreadyPlaintext"),
          hint: t("trae.stats.token.client.alreadyPlaintextHint"),
        };
      case "not_sqlite":
        return { title: t("trae.stats.token.client.notSqlite"), hint: null };
      case "missing":
      case "io":
        return { title: source.message, hint: null };
    }
  }

  /**
   * 「解锁并读取」：把**加密库**交给后端解密（`buddy-switch-vault` 的 `sqlcipher_export`），
   * 产物落到工具私有目录并自动设为数据源。
   *
   * 为什么要单列一个动作而不是让「校验」顺手做：解密会**写**文件（约等于源库大小），
   * 属于有副作用的操作，必须由用户显式点一下。
   */
  async function unlock() {
    if (busy !== null || typed === "" || passphrase === "") return;
    setBusy("unlock");
    setProbe(null);
    setFailed(null);
    setVaultError(null);
    setUnlockNote(null);
    try {
      const result = await api.unlockTraeClientDb(typed, passphrase);
      if (!result.ok) {
        setVaultError(result.error ?? { code: "io", message: t("trae.stats.token.client.loadFailed") });
        return;
      }
      /* 口令用完即弃：成功与否都不再持有它。 */
      setPassphrase("");
      setConfig(await api.getTraeClientDbConfig());
      if (result.probe) setProbe(result.probe);
      setReadAt(Date.now());
      setUnlockNote(
        result.mode === "plaintext"
          ? t("trae.stats.token.client.unlockedPlain")
          : t("trae.stats.token.client.unlocked", {
              compat: String(result.compatibility ?? 4),
            }),
      );
      onImported();
    } catch (e) {
      setVaultError({ code: "io", message: api.asError(e) });
    } finally {
      setBusy(null);
    }
  }

  /** 「保存并读取」：落配置 → 回读配置（拿权威的 `exists`）→ 通知父页重新取数。 */
  async function save() {
    if (busy !== null || typed === "") return;
    setBusy("save");
    setFailed(null);
    try {
      await api.saveTraeClientDbPath(typed);
      setConfig(await api.getTraeClientDbConfig());
      setReadAt(Date.now());
      onImported();
    } catch (e) {
      setFailed(api.asError(e));
    } finally {
      setBusy(null);
    }
  }

  /** 「清除」：路径置空 ⇒ 这本账回到「未配置」（统计页会显示「不可得」而不是 0）。 */
  async function clear() {
    if (busy !== null) return;
    setBusy("clear");
    setFailed(null);
    try {
      await api.saveTraeClientDbPath(null);
      setConfig({ path: null, exists: false });
      setPath("");
      setProbe(null);
      setReadAt(null);
      onImported();
    } catch (e) {
      setFailed(api.asError(e));
    } finally {
      setBusy(null);
    }
  }

  const readError = error ? errorCopy(error) : null;
  /* 校验失败时的同一套文案（`missing` / `io` 用后端原话）—— 提前算一次，避免在 JSX 里反复取。 */
  const probeError = probe && !probe.ok ? probe.error ?? null : null;
  const probeErrorCopy = probeError ? errorCopy(probeError) : null;

  return (
    <Card className="min-w-0">
      <CardHeader className="flex items-center gap-2.5 border-b px-4 py-3">
        <h3 className="text-[13.5px] font-semibold">{t("trae.stats.token.client.title")}</h3>
        <div className="flex-1" />
        <span className="text-[11.5px] text-muted-foreground">{windowLabel(viewWindow)}</span>
      </CardHeader>
      <CardContent className="space-y-3 px-4 py-3">
        <p className="text-[12.5px] leading-relaxed text-muted-foreground">
          {t("trae.stats.token.client.body")}
        </p>

        {/* 后端读失败：先说结论，再给原因（`not_plaintext` / `unrecognized` / `no_usage` 各自不同）。 */}
        {readError ? (
          <div className="space-y-1 rounded-[9px] border border-destructive/30 bg-destructive/5 px-3 py-2">
            <p className="flex items-center gap-1.5 text-[12.5px] font-medium text-destructive">
              <AlertTriangle className="size-3.5 shrink-0" aria-hidden="true" />
              {t("trae.stats.token.client.loadFailed")}
            </p>
            <p className="text-[12.5px] leading-relaxed break-all text-foreground">{readError.title}</p>
            {readError.hint ? (
              <p className="text-[11.5px] leading-relaxed text-muted-foreground">{readError.hint}</p>
            ) : null}
            {onRetry ? (
              <Button
                size="sm"
                variant="outline"
                className="mt-1"
                disabled={busy !== null}
                onClick={onRetry}
              >
                {busy !== null ? <Loader2 className="animate-spin" /> : <FileSearch />}
                {t("trae.stats.token.client.retry")}
              </Button>
            ) : null}
          </div>
        ) : null}

        <div className="space-y-1.5">
          <label
            className="block text-[11.5px] font-medium text-muted-foreground"
            htmlFor="trae-client-db-path"
          >
            {t("trae.stats.token.client.pathLabel")}
          </label>
          <div className="flex flex-wrap items-center gap-2">
            <Input
              id="trae-client-db-path"
              ref={inputRef}
              value={path}
              placeholder={t("trae.stats.token.client.pathPlaceholder")}
              spellCheck={false}
              className="min-w-0 flex-1 font-mono"
              onChange={(event) => {
                setPath(event.target.value);
                setProbe(null);
                setFailed(null);
              }}
            />
            <Button size="sm" variant="outline" disabled={busy !== null} onClick={() => void pick()}>
              {busy === "pick" ? <Loader2 className="animate-spin" /> : <FolderOpen />}
              {t("trae.stats.token.client.pick")}
            </Button>
          </div>
          {missingFile ? (
            <p className="flex items-center gap-1.5 text-[11.5px] text-warning">
              <AlertTriangle className="size-3.5 shrink-0" aria-hidden="true" />
              {t("trae.stats.token.client.missingHint")}
            </p>
          ) : null}
          {noPathYet ? (
            <p className="text-[11.5px] leading-relaxed text-muted-foreground">
              {t("trae.stats.token.client.missing")} · {t("trae.stats.token.client.missingHint")}
            </p>
          ) : null}
        </div>

        {/* 内置解密：口令由用户输入，只在内存里活到解锁那一刻。 */}
        <div className="space-y-1.5">
          <label
            className="block text-[11.5px] font-medium text-muted-foreground"
            htmlFor="trae-client-db-passphrase"
          >
            {t("trae.stats.token.client.passphraseLabel")}
          </label>
          <Input
            id="trae-client-db-passphrase"
            type="password"
            value={passphrase}
            placeholder={t("trae.stats.token.client.passphrasePlaceholder")}
            spellCheck={false}
            autoComplete="off"
            className="min-w-0 font-mono"
            onChange={(event) => {
              setPassphrase(event.target.value);
              setVaultError(null);
              setUnlockNote(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") void unlock();
            }}
          />
          <p className="text-[11.5px] leading-relaxed text-muted-foreground">
            {t("trae.stats.token.client.passphraseHint")}
          </p>
          <p className="text-[11.5px] leading-relaxed text-muted-foreground">
            {t("trae.stats.token.client.unlockHint")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            disabled={busy !== null || typed === "" || passphrase === ""}
            onClick={() => void unlock()}
          >
            {busy === "unlock" ? <Loader2 className="animate-spin" /> : <KeyRound />}
            {busy === "unlock"
              ? t("trae.stats.token.client.unlocking")
              : t("trae.stats.token.client.unlock")}
          </Button>
          <Button size="sm" variant="outline" disabled={busy !== null || typed === ""} onClick={() => void verify()}>
            {busy === "verify" ? <Loader2 className="animate-spin" /> : <FileSearch />}
            {t("trae.stats.token.client.verify")}
          </Button>
          <Button size="sm" disabled={busy !== null || typed === ""} onClick={() => void save()}>
            {busy === "save" ? <Loader2 className="animate-spin" /> : <CheckCircle2 />}
            {t("trae.stats.token.client.save")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={busy !== null || savedPath === null}
            onClick={() => void clear()}
          >
            {busy === "clear" ? <Loader2 className="animate-spin" /> : <Trash2 />}
            {t("trae.stats.token.client.clear")}
          </Button>
          {readAt !== null ? (
            <span className="text-[11.5px] text-muted-foreground">
              {t("trae.stats.token.client.importedAt", {
                time: new Date(readAt).toLocaleString("zh-CN", {
                  dateStyle: "medium",
                  timeStyle: "short",
                }),
              })}
            </span>
          ) : null}
        </div>

        {failed ? <p className="text-[11.5px] text-warning">{failed}</p> : null}

        {/* 解锁结果：成功给结论（含兼容代），失败按 vault 的 code 给结论 + 下一步。 */}
        {unlockNote ? (
          <p className="flex items-start gap-1.5 text-[12.5px] font-medium">
            <CheckCircle2
              className="mt-0.5 size-3.5 shrink-0 text-[color:var(--data-series-emerald)]"
              aria-hidden="true"
            />
            <span className="break-all">{unlockNote}</span>
          </p>
        ) : null}
        {vaultError ? (
          <div className="space-y-1 rounded-[9px] border border-destructive/30 bg-destructive/5 px-3 py-2">
            <p className="flex items-start gap-1.5 text-[12.5px] font-medium text-destructive">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
              <span className="break-all">{vaultErrorCopy(vaultError).title}</span>
            </p>
            {vaultErrorCopy(vaultError).hint ? (
              <p className="text-[11.5px] leading-relaxed text-muted-foreground">
                {vaultErrorCopy(vaultError).hint}
              </p>
            ) : null}
          </div>
        ) : null}

        {/* 校验结果：读得到给三个计数；读不到按 `code` 给结论 + 提示；表名一律回显（便于反馈适配）。 */}
        {probe ? (
          <div className="space-y-1.5 rounded-[9px] border px-3 py-2">
            {probe.ok ? (
              <p className="flex items-start gap-1.5 text-[12.5px] font-medium">
                <CheckCircle2
                  className="mt-0.5 size-3.5 shrink-0 text-[color:var(--data-series-emerald)]"
                  aria-hidden="true"
                />
                <span>
                  {t("trae.stats.token.client.probeOk", {
                    sessions: exactTokens(probe.counts?.sessions ?? 0),
                    messages: exactTokens(probe.counts?.messages ?? 0),
                    withUsage: exactTokens(probe.counts?.withUsage ?? 0),
                  })}
                </span>
              </p>
            ) : probeErrorCopy ? (
              <>
                <p className="flex items-start gap-1.5 text-[12.5px] font-medium text-warning">
                  <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
                  <span className="break-all">{probeErrorCopy.title}</span>
                </p>
                {probeErrorCopy.hint ? (
                  <p className="text-[11.5px] leading-relaxed text-muted-foreground">
                    {probeErrorCopy.hint}
                  </p>
                ) : null}
              </>
            ) : null}
            {/* `ok === false` 但后端仍给了计数（`no_usage`：「认出来了、但没有用量字段」）——
                这几个数是诊断的全部依据，不能因为结论是失败就不显示。
                ⚠️ `counts` 是**可选**的（`Unrecognized` / `NotPlaintext` 分支不返回）。 */}
            {!probe.ok && probe.counts ? (
              <p className="text-[12.5px] leading-relaxed tabular-nums text-muted-foreground">
                {t("trae.stats.token.client.probeOk", {
                  sessions: exactTokens(probe.counts.sessions),
                  messages: exactTokens(probe.counts.messages),
                  withUsage: exactTokens(probe.counts.withUsage),
                })}
              </p>
            ) : null}
            {probe.tables.length > 0 ? (
              <div className="space-y-0.5 border-t border-border/60 pt-1.5">
                <p className="text-[11px] font-medium text-muted-foreground">
                  {t("trae.stats.token.client.probeTables")}
                </p>
                <p className="font-mono text-[11px] leading-5 break-all text-muted-foreground">
                  {probe.tables.join(" · ")}
                </p>
              </div>
            ) : null}
          </div>
        ) : null}

        {/* 怎么导出：sqlcipher 命令是**多行**的，用 `whitespace-pre-wrap` 保住换行与缩进。 */}
        <details className="rounded-[9px] border px-3 py-2">
          <summary className="cursor-pointer text-[12.5px] font-medium">
            {t("trae.stats.token.client.helpTitle")}
          </summary>
          <p className="mt-2 font-mono text-[11.5px] leading-5 break-all whitespace-pre-wrap text-muted-foreground">
            {t("trae.stats.token.client.helpBody")}
          </p>
        </details>
      </CardContent>
    </Card>
  );
}
