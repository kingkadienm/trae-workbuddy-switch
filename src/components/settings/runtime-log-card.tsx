/**
 * 「运行日志」卡片（设置页「日志」分区）。
 *
 * ## 它展示的是**谁的**日志
 *
 * 本工具**自己**的运行日志（`~/.buddy-switch/logs/runtime-*.log`）—— 网关起不来、
 * 切号失败、后台任务报错、panic 都在这里。与另外两份日志严格区分：
 *
 * | 日志 | 载体 | 在哪里看 |
 * |:--|:--|:--|
 * | **本卡片** | `runtime-*.log`（纯文本行） | 设置 → 日志 |
 * | 网关请求日志 | `gateway_logs.json`（JSON、面向统计） | 网关页「请求日志」 |
 * | 上游客户端日志 | 各客户端自己的目录 | 不在本工具内 |
 *
 * 不合并它们的原因：用途不同（「发生了什么」vs「用了多少」），混在一起会让
 * 「按级别筛」这类操作对另一半数据失去意义。
 *
 * ## 交互约束（逐条对齐 Trae 侧的运行日志）
 *
 * - **筛选键进缓存键**：级别 / 日期 / 关键字任何一个变了都是另一份结果，
 *   漏在键外就会出现「换了日期、列表还是旧的」；
 * - **刷新按钮走 `useRefreshAction`**：`useCachedResource.loading` 只在「没有可用值」
 *   时为真，有旧值时的后台刷新它恒为 false ⇒ 绑它按钮永远不转（伪反馈）；
 * - **自动刷新加两道门控**：用户正在输入关键字时不刷（列表会抖回旧条件的结果）；
 *   应用不可见时不刷（2 秒一轮白打接口）；
 * - **清空必须二次确认**：删数据不是隐藏数据，且如实说明「不可恢复」；
 * - **空态分两种**：文件还不存在（新装）vs 当前筛选无匹配 —— 后者要引导「重置筛选」，
 *   否则用户会以为日志没写。
 */

import { useCallback, useEffect, useState } from "react";
import {
  ChevronDown,
  Copy,
  FolderOpen,
  Loader2,
  RefreshCw,
  Search,
  ShieldAlert,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { CardContent } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { SettingsGroup } from "@/components/settings-primitives";
import * as api from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { useCachedResource } from "@/lib/use-cached-resource";
import { useDocumentVisible } from "@/lib/use-document-visible";
import { useRefreshAction } from "@/lib/use-refresh-action";
import { useT } from "@/lib/i18n";
import type { RuntimeLogLevel, RuntimeLogsResponse } from "@/lib/types";
import type { TranslationKey } from "@/locales/zh";
import { cn } from "@/lib/utils";

/** 级别筛选的取值（`all` = 不过滤）。顺序即下拉顺序：**严重在前**。 */
const LEVEL_OPTIONS: { value: RuntimeLogLevel | "all"; labelKey: TranslationKey }[] = [
  { value: "all", labelKey: "settings.runtimeLog.levelAll" },
  { value: "error", labelKey: "settings.runtimeLog.levelError" },
  { value: "warn", labelKey: "settings.runtimeLog.levelWarn" },
  { value: "info", labelKey: "settings.runtimeLog.levelInfo" },
  { value: "debug", labelKey: "settings.runtimeLog.levelDebug" },
];

/** 级别的视觉标记（**颜色 + 文字**双编码：不靠颜色单独传意，色盲可用）。 */
const LEVEL_STYLE: Record<RuntimeLogLevel, { text: string; badge: string }> = {
  error: { text: "text-destructive", badge: "bg-destructive/12 text-destructive" },
  warn: { text: "text-amber-600 dark:text-amber-500", badge: "bg-amber-500/12 text-amber-600 dark:text-amber-500" },
  info: { text: "text-muted-foreground", badge: "bg-muted text-muted-foreground" },
  debug: { text: "text-muted-foreground/70", badge: "bg-muted/60 text-muted-foreground/80" },
};

const LEVEL_LABEL_KEY: Record<RuntimeLogLevel, TranslationKey> = {
  error: "settings.runtimeLog.levelError",
  warn: "settings.runtimeLog.levelWarn",
  info: "settings.runtimeLog.levelInfo",
  debug: "settings.runtimeLog.levelDebug",
};

/** 自动刷新间隔（毫秒）。取 2 秒：与 Trae 运行日志同节奏，看「刚才那条」不必手点。 */
const AUTO_REFRESH_MS = 2000;

/** 单次最多展示的条数（与后端默认 `limit` 一致）。 */
const DEFAULT_LIMIT = 500;

/** 运行日志卡片。 */
export function RuntimeLogCard() {
  const t = useT();

  const [level, setLevel] = useState<RuntimeLogLevel | "all">("all");
  const [date, setDate] = useState("all");
  const [keyword, setKeyword] = useState("");
  const [keywordDraft, setKeywordDraft] = useState("");
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);

  /**
   * 键里带上三个筛选条件：任何一个变了都是**另一份**结果。
   * `limit` 刻意不进键 —— 它只影响「截断到几条」，不属于查询语义。
   */
  const load = useCallback(
    () =>
      api.runtimeLogs({
        level: level === "all" ? undefined : level,
        date: date === "all" ? undefined : date,
        keyword: keyword || undefined,
        limit: DEFAULT_LIMIT,
      }),
    [level, date, keyword],
  );

  const { data, loading, error, refresh, revalidate } = useCachedResource<RuntimeLogsResponse>(
    `runtime-log:${level}:${date}:${keyword}`,
    load,
  );

  /* 刷新按钮的进行中态走本 hook 而不是 `loading`：有旧值时的后台刷新 `loading` 恒为 false。 */
  const { busy, run } = useRefreshAction();

  // 自动刷新只在「没有未提交的输入」且**应用可见**时跑。
  const dirty = keywordDraft !== keyword;
  const documentVisible = useDocumentVisible();
  useEffect(() => {
    if (!autoRefresh || dirty || !documentVisible) return;
    const timer = setInterval(() => void revalidate(), AUTO_REFRESH_MS);
    return () => clearInterval(timer);
  }, [autoRefresh, dirty, documentVisible, revalidate]);

  const entries = data?.entries ?? [];
  const counts = data?.counts;

  /** 未知级别（后端将来新增）回落到原文，不假装认识。 */
  const levelText = (value: RuntimeLogLevel): string => {
    const key: TranslationKey | undefined = LEVEL_LABEL_KEY[value];
    return key ? t(key) : value;
  };

  const resetFilters = () => {
    setLevel("all");
    setDate("all");
    setKeyword("");
    setKeywordDraft("");
  };

  const copyAll = async () => {
    if (entries.length === 0) return;
    await copyText(
      entries.map((entry) => `${entry.time} ${entry.level.toUpperCase()} [${entry.scope}] ${entry.message}`).join("\n"),
    );
  };

  async function runClear() {
    if (clearing) return;
    setClearing(true);
    try {
      const result = await api.clearRuntimeLogs();
      toast.success(t("settings.runtimeLog.cleared", { count: result.removed }));
      await refresh();
    } catch (e) {
      toast.error(t("settings.runtimeLog.clearFailed"), { description: api.asError(e) });
    } finally {
      setClearing(false);
      setClearOpen(false);
    }
  }

  async function runOpenDir() {
    try {
      await api.openRuntimeLogsDir();
    } catch (e) {
      toast.error(t("settings.runtimeLog.openDirFailed"), { description: api.asError(e) });
    }
  }

  const filtered = level !== "all" || date !== "all" || keyword !== "";
  const truncated = data?.truncated ?? false;

  return (
    <SettingsGroup id="settings-runtime-log" title={t("settings.runtimeLog.groupTitle")}>
      <CardContent className="space-y-0 p-0">
        {/* 说明条：先讲清「这是谁的日志」，再给操作 —— 用户最容易在这里误解成客户端日志。 */}
        <p className="border-b border-border/60 bg-muted/25 px-4 py-3 text-xs leading-5 text-muted-foreground">
          {t("settings.runtimeLog.intro", {
            days: String(data?.keepDays ?? 7),
            level: data?.level ?? "info",
          })}
        </p>

        <div className="p-4 sm:p-5">
          {/* ---- 筛选行 ---- */}
          <div className="flex flex-wrap items-center gap-2">
            <Select value={level} onValueChange={(value) => setLevel(value as RuntimeLogLevel | "all")}>
              <SelectTrigger size="sm" className="w-28">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LEVEL_OPTIONS.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {t(option.labelKey)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Select value={date} onValueChange={setDate}>
              <SelectTrigger size="sm" className="w-40">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("settings.runtimeLog.dateAll")}</SelectItem>
                {(data?.dates ?? []).map((day) => (
                  <SelectItem key={day} value={day}>
                    {day}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <div className="relative min-w-[180px] flex-1">
              <Search className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={keywordDraft}
                onChange={(event) => setKeywordDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") setKeyword(keywordDraft.trim());
                }}
                placeholder={t("settings.runtimeLog.searchPlaceholder")}
                className="h-8 pl-8"
              />
            </div>

            <Button variant="outline" size="sm" onClick={() => setKeyword(keywordDraft.trim())}>
              <Search />
              {t("settings.runtimeLog.search")}
            </Button>
            <Button variant="ghost" size="sm" onClick={resetFilters} disabled={!filtered && !keywordDraft}>
              <X />
              {t("settings.runtimeLog.reset")}
            </Button>
          </div>

          {/* ---- 工具行：自动刷新 / 计数 / 复制 / 打开目录 / 清空 ---- */}
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-border/60 pt-3">
            <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5">
                <Switch
                  checked={autoRefresh}
                  onCheckedChange={setAutoRefresh}
                  aria-label={t("settings.runtimeLog.autoRefreshAria")}
                />
                {t("settings.runtimeLog.autoRefresh")}
              </span>
              {counts && (
                <span className="flex flex-wrap items-center gap-2">
                  <span className="tabular-nums">
                    {t("settings.runtimeLog.countSummary", { total: String(data?.total ?? 0) })}
                  </span>
                  {(["error", "warn", "info", "debug"] as RuntimeLogLevel[]).map((value) =>
                    counts[value] > 0 ? (
                      <span
                        key={value}
                        className={cn("rounded px-1.5 py-0.5 text-[11px] font-medium", LEVEL_STYLE[value].badge)}
                      >
                        {levelText(value)} {counts[value]}
                      </span>
                    ) : null,
                  )}
                </span>
              )}
              {truncated && (
                <span className="text-amber-600 dark:text-amber-500">
                  {t("settings.runtimeLog.truncated", { limit: String(data?.limit ?? DEFAULT_LIMIT) })}
                </span>
              )}
            </div>

            <div className="flex flex-wrap items-center gap-1.5">
              <Button variant="ghost" size="sm" onClick={() => void run(refresh)} disabled={busy}>
                {busy ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                {t("settings.runtimeLog.refresh")}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => void copyAll()} disabled={entries.length === 0}>
                <Copy />
                {t("settings.runtimeLog.copy")}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => void runOpenDir()}>
                <FolderOpen />
                {t("settings.runtimeLog.openDir")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="text-destructive hover:text-destructive"
                onClick={() => setClearOpen(true)}
                disabled={clearing}
              >
                <Trash2 />
                {t("settings.runtimeLog.clear")}
              </Button>
            </div>
          </div>

          {/* ---- 列表 ---- */}
          <div className="mt-3">
            {loading ? (
              <div className="space-y-2">
                {Array.from({ length: 5 }).map((_, index) => (
                  <Skeleton key={index} className="h-9 w-full" />
                ))}
              </div>
            ) : error ? (
              <p className="py-8 text-center text-xs text-destructive">{error}</p>
            ) : entries.length === 0 ? (
              /* 空态分两种：有筛选 ⇒ 引导重置；无筛选 ⇒ 说明「还没产生日志」。 */
              <div className="py-10 text-center">
                <p className="text-[13px] font-medium">
                  {filtered ? t("settings.runtimeLog.emptyFilteredTitle") : t("settings.runtimeLog.emptyTitle")}
                </p>
                <p className="mx-auto mt-1.5 max-w-md text-xs leading-5 text-muted-foreground">
                  {filtered ? t("settings.runtimeLog.emptyFilteredBody") : t("settings.runtimeLog.emptyBody")}
                </p>
                {filtered && (
                  <Button variant="outline" size="sm" className="mt-3" onClick={resetFilters}>
                    {t("settings.runtimeLog.reset")}
                  </Button>
                )}
              </div>
            ) : (
              <div className="max-h-[520px] overflow-y-auto rounded-lg border border-border/60">
                <table className="w-full border-collapse text-[12px]">
                  <tbody>
                    {entries.map((entry, index) => (
                      <tr
                        key={`${entry.ts}-${index}`}
                        className="border-b border-border/40 align-top last:border-b-0 hover:bg-muted/40"
                      >
                        <td className="whitespace-nowrap px-3 py-2 font-mono text-[11px] text-muted-foreground">
                          {entry.time}
                        </td>
                        <td className="whitespace-nowrap px-1 py-2">
                          <span
                            className={cn(
                              "rounded px-1.5 py-0.5 text-[10.5px] font-semibold uppercase tracking-wide",
                              LEVEL_STYLE[entry.level]?.badge ?? "bg-muted text-muted-foreground",
                            )}
                          >
                            {levelText(entry.level)}
                          </span>
                        </td>
                        <td className="whitespace-nowrap px-2 py-2 font-mono text-[11px] text-muted-foreground">
                          {entry.scope ? `[${entry.scope}]` : ""}
                        </td>
                        <td className="w-full px-3 py-2 break-words font-mono text-[11.5px] leading-5">
                          {entry.message}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>

          {/* ---- 落盘位置与上限（排查时要能直接找到文件）---- */}
          {data?.logDir && (
            <Collapsible className="mt-3 rounded-lg border border-border/60 bg-muted/20 px-3 py-2">
              <CollapsibleTrigger className="flex w-full items-center gap-1.5 text-[11.5px] text-muted-foreground [&[data-state=open]>svg]:rotate-180">
                <ChevronDown className="size-3.5 transition-transform" />
                {t("settings.runtimeLog.details")}
              </CollapsibleTrigger>
              <CollapsibleContent>
                <dl className="mt-2 space-y-1 font-mono text-[11px] text-muted-foreground">
                  <div className="flex gap-2">
                    <dt className="shrink-0">{t("settings.runtimeLog.dirLabel")}</dt>
                    <dd className="min-w-0 break-all">{data.logDir}</dd>
                  </div>
                  <div className="flex gap-2">
                    <dt className="shrink-0">{t("settings.runtimeLog.limitLabel")}</dt>
                    <dd>
                      {t("settings.runtimeLog.limitValue", {
                        days: String(data.keepDays),
                        size: `${Math.round(data.maxPartBytes / 1024 / 1024)} MB`,
                        parts: String(data.maxPartsPerDay),
                      })}
                    </dd>
                  </div>
                </dl>
              </CollapsibleContent>
            </Collapsible>
          )}
        </div>
      </CardContent>

      {/* ---- 清空确认（删数据，必须二次确认）---- */}
      <Dialog open={clearOpen} onOpenChange={setClearOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <ShieldAlert className="size-4 text-destructive" />
              {t("settings.runtimeLog.clearConfirmTitle")}
            </DialogTitle>
            <DialogDescription>{t("settings.runtimeLog.clearConfirmBody")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setClearOpen(false)} disabled={clearing}>
              {t("settings.runtimeLog.cancel")}
            </Button>
            <Button variant="destructive" onClick={() => void runClear()} disabled={clearing}>
              {clearing ? <Loader2 className="animate-spin" /> : <Trash2 />}
              {t("settings.runtimeLog.clearConfirmAction")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsGroup>
  );
}
