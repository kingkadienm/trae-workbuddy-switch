"use client";

import { useEffect, useMemo, useState } from "react";
import { Pencil } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Seg } from "@/components/ui/seg";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { useT } from "@/lib/i18n";
import type { KeyPolicy, KeyUsageSnapshot, QuotaMode, QuotaPeriod, QuotaUnit, ScopeMode } from "@/lib/api";
import { cn } from "@/lib/utils";

/* ══════════════════════════════════════════════════════════════════════════
   密钥权限共用构件（issue #76）—— **三个平台 + 统一网关共用同一份**。
   ══════════════════════════════════════════════════════════════════════════
   设计基准：`docs/prototype/buddy-switch-prototype.html` 的「网关 › 密钥」分区
   （v3：一览压成一行，明细进行内展开区；规范见
   `docs/specs/KEY-PERMISSION-UI-DESIGN.md` §3.0.1）。

   分三层，全部**受控**（数据由调用方给，本文件不碰 store / 不发请求）：
   ① 一览（表格列）：`KeyUsageCell` + `KeyScopeChip` —— 每行只有两行文字；
   ② 行内展开：`KeyRowDetail` —— 单位 / 周期 / 剩余 / 账号名册在这儿回收；
   ③ 抽屉：`KeyPermissionDrawer` —— 改策略的唯一入口（先落草稿，取消即丢弃）。
 */

/** 账号选择器的一行（调用方从账号库给；`group` 用于「按分组整组选取」）。 */
export interface KeyPermissionAccount {
  id: string;
  name: string;
  group?: string;
}

/* ─────────── ① 一览层 ─────────── */

/** 用量数字：中文优先「万」（`100万` 比 `1M` 好读），≥1 亿才用 M。 */
export function formatQuota(n: number): string {
  const v = Number.isFinite(n) ? Math.max(0, Math.round(n)) : 0;
  if (v >= 1e8) return `${(v / 1e6).toFixed(v % 1e6 === 0 ? 0 : 1)}M`;
  if (v >= 1e4) return `${(v / 1e4).toFixed(v % 1e4 === 0 ? 0 : 1)}万`;
  return String(v);
}

/**
 * 表格「用量」列 —— **一行**：`已用 / 上限` + 3px 迷你条。
 *
 * ★ 单位、周期、剩余**故意不在这一列**（v3 的核心取舍）：四层文字全塞一列会把
 * 行高撑到 4 行，与权限列叠在一起整行密不透风（真机截图返工）。
 * 那些是「想看才看」的信息 ⇒ 去 `KeyRowDetail`。
 */
export function KeyUsageCell({
  policy,
  usage,
  className,
}: {
  policy: KeyPolicy;
  usage: KeyUsageSnapshot | null;
  className?: string;
}) {
  const t = useT();
  if (policy.quotaMode !== "limited" || policy.quotaLimit <= 0) {
    /* 不限额时**仍显示已用次数**（当调用方给了 usage）——
       「这把 key 被用了多少次」与「有没有上限」是两件事：分给同事的 key 即使不限额，
       你也想知道它到底在不在被用。调用方没给 usage 时退回纯「不限」（另三条产品线
       只拉限额 Key 的用量，行为不变）。 */
    const total = usage?.total ?? 0;
    return (
      <span className={cn("text-sm text-muted-foreground", className)}>
        {usage && total > 0
          ? t("wbStats.keyPerm.unlimitedWithUsage", { n: String(total) })
          : t("wbStats.keyPerm.unlimited")}
      </span>
    );
  }
  const used = usage?.used ?? 0;
  const pct = Math.min(100, Math.round((used / policy.quotaLimit) * 100));
  const danger = used >= policy.quotaLimit;
  const warn = !danger && used >= policy.quotaLimit * 0.8;
  return (
    <span className={cn("flex items-center gap-1.5 whitespace-nowrap text-sm", className)}>
      <span className="tabular-nums">{formatQuota(used)}</span>
      <span className="text-muted-foreground">/ {formatQuota(policy.quotaLimit)}</span>
      {danger ? (
        <span className="text-[11px] text-destructive">{t("wbStats.keyPerm.exhausted")}</span>
      ) : null}
      <span
        className={cn(
          "h-[3px] min-w-[28px] flex-1 overflow-hidden rounded-full bg-muted",
        )}
        role="presentation"
      >
        <span
          className={cn(
            "block h-full rounded-full",
            danger ? "bg-destructive" : warn ? "bg-warning" : "bg-primary",
          )}
          style={{ width: `${pct}%` }}
        />
      </span>
    </span>
  );
}

/**
 * 表格「权限」列 —— **一枚 chip**（账号范围）。
 *
 * ★ 为什么不再有「额度」chip：上限值已在用量列以 `/ 1000万` 体现，
 * 再写一枚「额度 1000万/自然月」是同一信息的第二次展示，纯占宽度。
 */
export function KeyScopeChip({
  policy,
  onOpenScope,
  className,
}: {
  policy: KeyPolicy;
  onOpenScope: () => void;
  className?: string;
}) {
  const t = useT();
  const count = policy.scopeAccounts.length + policy.scopeGroups.length;
  const all = policy.scopeMode !== "selected";
  return (
    <button
      type="button"
      onClick={onOpenScope}
      title={t("wbStats.keyPerm.editScope")}
      className={cn(
        "inline-flex h-[22px] shrink-0 items-center gap-1.5 rounded-[7px] border px-2 text-[11px] whitespace-nowrap transition-colors",
        all
          ? "border-border bg-card text-muted-foreground hover:text-foreground"
          : count > 0
            ? "border-primary/40 bg-primary/10 text-primary"
            : "border-destructive/40 text-destructive",
        className,
      )}
    >
      <span className="size-[6px] rounded-full bg-current opacity-75" />
      {t("wbStats.keyPerm.accounts")}
      {all ? ` ${t("wbStats.keyPerm.allAccounts")}` : ` ${count}`}
    </button>
  );
}

/* ─────────── ② 行内展开层 ─────────── */

/**
 * 点行展开的明细区 —— 一览让出去的信息在这儿回收：完整配额口径 + 账号名册。
 *
 * 两行各带一个「编辑」快捷钮（直达抽屉对应段）—— 展开是「看」，编辑是「改」，
 * 两个动作分开但都在同一块地方，用户不用记住「改配额要去另一个页面」。
 */
export function KeyRowDetail({
  policy,
  usage,
  accountNames,
  onEditQuota,
  onEditScope,
}: {
  policy: KeyPolicy;
  usage: KeyUsageSnapshot | null;
  /** 白名单账号的显示名（按 policy.scopeAccounts 的顺序；分组单独一行）。 */
  accountNames: string[];
  onEditQuota: () => void;
  onEditScope: () => void;
}) {
  const t = useT();
  const quotaText =
    policy.quotaMode !== "limited" || policy.quotaLimit <= 0
      ? t("wbStats.keyPerm.unlimitedNote")
      : `${formatQuota(usage?.used ?? 0)} / ${formatQuota(policy.quotaLimit)} ${
          policy.quotaUnit === "request" ? t("wbStats.keyPerm.requests") : t("wbStats.keyPerm.tokens")
        } · ${t(`wbStats.keyPerm.period.${policy.quotaPeriod}` as "wbStats.keyPerm.period.none")} · ${t("wbStats.keyPerm.left")} ${formatQuota(
          Math.max(0, policy.quotaLimit - (usage?.used ?? 0)),
        )}`;

  return (
    <div className="space-y-1 border-t border-dashed border-border bg-card/40 px-3 py-2.5 text-[12px]">
      <div className="flex items-baseline gap-2.5">
        <span className="w-[76px] shrink-0 text-[11.5px] text-muted-foreground">
          {t("wbStats.keyPerm.quota")}
        </span>
        <span className="min-w-0 flex-1 leading-6">{quotaText}</span>
        <Button variant="ghost" size="xs" onClick={onEditQuota}>
          <Pencil />
          {t("wbStats.keyPerm.editQuota")}
        </Button>
      </div>
      <div className="flex items-baseline gap-2.5 border-t border-border/60 pt-1">
        <span className="w-[76px] shrink-0 text-[11.5px] text-muted-foreground">
          {t("wbStats.keyPerm.scope")}
        </span>
        <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1 leading-6">
          {policy.scopeMode !== "selected" ? (
            <span className="text-muted-foreground">
              {t("wbStats.keyPerm.allAccountsNote", { n: accountNames.length })}
            </span>
          ) : accountNames.length > 0 ? (
            accountNames.map((name) => (
              <span key={name} className="inline-flex h-[22px] items-center rounded-md bg-muted px-2 text-[11.5px]">
                {name}
              </span>
            ))
          ) : (
            <span className="text-[11.5px] text-destructive">
              {t("wbStats.keyPerm.emptyScopeWarning")}
            </span>
          )}
          {policy.scopeGroups.length > 0 ? (
            <span className="text-[11.5px] text-muted-foreground">
              {t("wbStats.keyPerm.groupsToo", { n: policy.scopeGroups.length })}
            </span>
          ) : null}
        </span>
        <Button variant="ghost" size="xs" onClick={onEditScope}>
          <Pencil />
          {t("wbStats.keyPerm.editScope")}
        </Button>
      </div>
    </div>
  );
}

/* ─────────── ③ 抽屉层 ─────────── */

const UNIT_OPTIONS: { value: QuotaUnit; labelKey: "wbStats.keyPerm.tokens" | "wbStats.keyPerm.requests" }[] = [
  { value: "token", labelKey: "wbStats.keyPerm.tokens" },
  { value: "request", labelKey: "wbStats.keyPerm.requests" },
];

const PERIOD_OPTIONS: { value: QuotaPeriod; labelKey: `wbStats.keyPerm.period.${QuotaPeriod}` }[] = [
  { value: "none", labelKey: "wbStats.keyPerm.period.none" },
  { value: "daily", labelKey: "wbStats.keyPerm.period.daily" },
  { value: "weekly", labelKey: "wbStats.keyPerm.period.weekly" },
  { value: "monthly", labelKey: "wbStats.keyPerm.period.monthly" },
];

/**
 * 权限抽屉 —— 改策略的**唯一入口**。
 *
 * 三条产品纪律（与原型逐字对应）：
 * 1. **先落草稿**：打开时从传入策略拷一份，「取消」直接丢弃 ⇒ 不会留下改了一半的策略；
 * 2. **「指定账号」一个都没选 ⇒ 保存禁用**（空白名单 = 永远选不到号 = 静默失败）；
 * 3. **上限低于已用量 ⇒ 允许保存但明确警告**「保存后立即超限」—— 有时就是要立刻
 *    掐掉这把 key，拦着不让存反而让人绕路；但后果必须说清。
 */
export function KeyPermissionDrawer({
  open,
  onOpenChange,
  keyName,
  policy,
  usage,
  accounts,
  saving,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  keyName: string;
  policy: KeyPolicy;
  usage: KeyUsageSnapshot | null;
  accounts: KeyPermissionAccount[];
  saving: boolean;
  onSave: (next: KeyPolicy) => void;
}) {
  const t = useT();
  const [draft, setDraft] = useState<KeyPolicy>(policy);
  const [query, setQuery] = useState("");
  /* ★ 草稿在抽屉**每次打开时**重新拷一份（不是每次 render）。
     判据用 `open` 而不是「policy 引用变了」：父组件每次 render 都会传新的对象
     字面量，用引用比较会在用户编辑中途把草稿冲回原值（改一个字就被重置）。 */
  useEffect(() => {
    if (open) {
      setDraft(policy);
      setQuery("");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 只认「打开」这一动作
  }, [open]);

  const groups = useMemo(() => {
    const seen: string[] = [];
    for (const account of accounts) {
      const group = account.group?.trim();
      if (group && !seen.includes(group)) seen.push(group);
    }
    return seen;
  }, [accounts]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return accounts;
    return accounts.filter((a) => a.name.toLowerCase().includes(q));
  }, [accounts, query]);

  const limited = draft.quotaMode === "limited";
  const used = usage?.used ?? 0;
  const overLimit = limited && draft.quotaLimit > 0 && used >= draft.quotaLimit;
  const selected = draft.scopeMode === "selected";
  const canSave =
    !(selected && draft.scopeAccounts.length === 0 && draft.scopeGroups.length === 0) &&
    !(limited && draft.quotaLimit <= 0);

  function patch(next: Partial<KeyPolicy>) {
    setDraft((prev) => ({ ...prev, ...next }));
  }

  function toggleAccount(id: string) {
    setDraft((prev) => {
      const set = new Set(prev.scopeAccounts);
      if (set.has(id)) set.delete(id);
      else set.add(id);
      return { ...prev, scopeAccounts: [...set] };
    });
  }

  function toggleGroup(group: string) {
    const members = accounts.filter((a) => a.group?.trim() === group).map((a) => a.id);
    setDraft((prev) => {
      const set = new Set(prev.scopeAccounts);
      const all = members.every((id) => set.has(id));
      for (const id of members) {
        if (all) set.delete(id);
        else set.add(id);
      }
      return { ...prev, scopeAccounts: [...set] };
    });
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" showCloseButton={false} className="w-full sm:max-w-md">
        <SheetHeader>
          <SheetTitle>
            {t("wbStats.keyPerm.title")} · {keyName}
          </SheetTitle>
          <SheetDescription>{t("wbStats.keyPerm.desc")}</SheetDescription>
        </SheetHeader>

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 pb-2">
          {/* ── 用量配额 ── */}
          <section className="space-y-2.5">
            <h3 className="text-[13px] font-semibold">{t("wbStats.keyPerm.quota")}</h3>
            <Seg
              ariaLabel={t("wbStats.keyPerm.quota")}
              value={draft.quotaMode}
              onChange={(value) => patch({ quotaMode: value as QuotaMode })}
              options={[
                { value: "unlimited", label: t("wbStats.keyPerm.unlimited") },
                { value: "limited", label: t("wbStats.keyPerm.limited") },
              ]}
            />
            {limited ? (
              <div className="space-y-2.5 rounded-lg border border-border bg-muted/30 p-3">
                <div className="flex items-center gap-2">
                  <span className="w-[56px] shrink-0 text-[11.5px] text-muted-foreground">
                    {t("wbStats.keyPerm.unit")}
                  </span>
                  <Seg
                    ariaLabel={t("wbStats.keyPerm.unit")}
                    value={draft.quotaUnit}
                    onChange={(value) => patch({ quotaUnit: value as QuotaUnit })}
                    options={UNIT_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) }))}
                  />
                </div>
                <p className="text-[11.5px] leading-5 text-muted-foreground">
                  {draft.quotaUnit === "request"
                    ? t("wbStats.keyPerm.unitRequestHint")
                    : t("wbStats.keyPerm.unitTokenHint")}
                </p>
                <div className="flex items-center gap-2">
                  <span className="w-[56px] shrink-0 text-[11.5px] text-muted-foreground">
                    {t("wbStats.keyPerm.limit")}
                  </span>
                  <Input
                    type="number"
                    min={0}
                    className="w-[120px] tabular-nums"
                    value={draft.quotaLimit || ""}
                    onChange={(event) => patch({ quotaLimit: Math.max(0, Number(event.target.value) || 0) })}
                    aria-label={t("wbStats.keyPerm.limit")}
                  />
                  <span className="text-[11.5px] text-muted-foreground">
                    {draft.quotaUnit === "request" ? t("wbStats.keyPerm.requests") : t("wbStats.keyPerm.tokens")}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="w-[56px] shrink-0 text-[11.5px] text-muted-foreground">
                    {t("wbStats.keyPerm.periodLabel")}
                  </span>
                  <Seg
                    ariaLabel={t("wbStats.keyPerm.periodLabel")}
                    value={draft.quotaPeriod}
                    onChange={(value) => patch({ quotaPeriod: value as QuotaPeriod })}
                    options={PERIOD_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) }))}
                  />
                </div>
                {draft.quotaLimit <= 0 ? (
                  <p className="text-[11.5px] text-warning">{t("wbStats.keyPerm.zeroLimit")}</p>
                ) : overLimit ? (
                  <p className="text-[11.5px] leading-5 text-warning">
                    {t("wbStats.keyPerm.overLimitWarning", {
                      used: formatQuota(used),
                      limit: formatQuota(draft.quotaLimit),
                    })}
                  </p>
                ) : (
                  <p className="text-[11.5px] leading-5 text-muted-foreground">
                    {t("wbStats.keyPerm.quotaNote", {
                      unit: draft.quotaUnit === "request" ? t("wbStats.keyPerm.requests") : t("wbStats.keyPerm.tokens"),
                      period: t(`wbStats.keyPerm.period.${draft.quotaPeriod}` as "wbStats.keyPerm.period.none"),
                    })}
                  </p>
                )}
              </div>
            ) : null}
          </section>

          {/* ── 可访问账号 ── */}
          <section className="space-y-2.5">
            <h3 className="text-[13px] font-semibold">{t("wbStats.keyPerm.scope")}</h3>
            <Seg
              ariaLabel={t("wbStats.keyPerm.scope")}
              value={draft.scopeMode}
              onChange={(value) => patch({ scopeMode: value as ScopeMode })}
              options={[
                { value: "all", label: t("wbStats.keyPerm.allAccounts") },
                { value: "selected", label: t("wbStats.keyPerm.selectedAccounts") },
              ]}
            />
            {selected ? (
              <div className="space-y-2 rounded-lg border border-border bg-muted/30 p-3">
                <div className="flex items-center gap-2">
                  <Input
                    className="h-8 min-w-0 flex-1 text-[12px]"
                    placeholder={t("wbStats.keyPerm.searchAccount")}
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                  />
                  <Button variant="outline" size="xs" onClick={() => patch({ scopeAccounts: accounts.map((a) => a.id) })}>
                    {t("wbStats.keyPerm.selectAll")}
                  </Button>
                  <Button variant="outline" size="xs" onClick={() => patch({ scopeAccounts: [] })}>
                    {t("wbStats.keyPerm.clear")}
                  </Button>
                </div>
                {groups.length > 1 ? (
                  <div className="flex flex-wrap gap-1.5">
                    {groups.map((group) => {
                      const members = accounts.filter((a) => a.group?.trim() === group);
                      const on = members.filter((a) => draft.scopeAccounts.includes(a.id)).length;
                      return (
                        <button
                          key={group}
                          type="button"
                          onClick={() => toggleGroup(group)}
                          className={cn(
                            "inline-flex h-6 items-center gap-1.5 rounded-md border px-2 text-[11.5px] transition-colors",
                            on === members.length
                              ? "border-primary/40 bg-primary/10 text-primary"
                              : "border-border text-muted-foreground hover:text-foreground",
                          )}
                        >
                          {on === members.length ? "✓ " : ""}
                          {group}
                          <span className="text-[10.5px] opacity-70">
                            {on}/{members.length}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                ) : null}
                <div className="max-h-[188px] overflow-y-auto rounded-md border border-border">
                  {filtered.map((account) => {
                    const on = draft.scopeAccounts.includes(account.id);
                    return (
                      <label
                        key={account.id}
                        className="flex cursor-pointer items-center gap-2 border-t border-border/60 px-2.5 py-1.5 first:border-t-0 hover:bg-accent/40"
                      >
                        <Checkbox checked={on} onCheckedChange={() => toggleAccount(account.id)} />
                        <span className="min-w-0 flex-1 truncate text-[12.5px]">{account.name}</span>
                        {account.group ? (
                          <span className="text-[11px] text-muted-foreground">{account.group}</span>
                        ) : null}
                      </label>
                    );
                  })}
                  {filtered.length === 0 ? (
                    <p className="px-2.5 py-2 text-[11.5px] text-muted-foreground">
                      {t("wbStats.keyPerm.noMatch")}
                    </p>
                  ) : null}
                </div>
                <p className="text-[11.5px] text-muted-foreground">
                  {t("wbStats.keyPerm.selectedCount", { n: draft.scopeAccounts.length })}
                </p>
                {draft.scopeAccounts.length === 0 && draft.scopeGroups.length === 0 ? (
                  <p className="text-[11.5px] text-warning">{t("wbStats.keyPerm.emptyScopeWarning")}</p>
                ) : null}
              </div>
            ) : null}
          </section>
        </div>

        {/* 底部：预览句 + 取消/保存。预览把「保存后会发生什么」一次说清。 */}
        <div className="flex items-center gap-2 border-t border-border px-5 py-3">
          <p className="min-w-0 flex-1 text-[11.5px] leading-5 text-muted-foreground">
            {t("wbStats.keyPerm.preview", {
              scope:
                draft.scopeMode !== "selected"
                  ? t("wbStats.keyPerm.allAccounts")
                  : `${draft.scopeAccounts.length + draft.scopeGroups.length}`,
              quota:
                !limited || draft.quotaLimit <= 0
                  ? t("wbStats.keyPerm.unlimited")
                  : `${formatQuota(draft.quotaLimit)}${
                      draft.quotaPeriod === "none"
                        ? ""
                        : `（${t(`wbStats.keyPerm.period.${draft.quotaPeriod}` as "wbStats.keyPerm.period.none")}）`
                    }`,
            })}
            {overLimit ? (
              <b className="text-destructive">{t("wbStats.keyPerm.previewOver")}</b>
            ) : null}
          </p>
          <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
            {t("wbStats.keyPerm.cancel")}
          </Button>
          <Button size="sm" disabled={!canSave || saving} onClick={() => onSave(draft)}>
            {saving ? t("wbStats.keyPerm.saving") : t("wbStats.keyPerm.save")}
          </Button>
        </div>
      </SheetContent>
    </Sheet>
  );
}
