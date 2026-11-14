"use client";

import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Plus, Tag } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * 账号分组选择器（**可新建**）。
 *
 * ## 为什么不是「一个文本框」也不是「一个 `<select>`」
 *
 * 分组在后端是**自由文本**（`AccountMeta.groupName`），所以它既不是固定枚举、
 * 也不该让用户每次手打一遍：
 *
 * - 纯文本框 ⇒ 用户看不见「已经有哪些组」，同一个组会被打成「客户A / 客户 A / 客户a」三份；
 * - 纯 `<select>` ⇒ 只能选已有项，**建不出新组**。
 *
 * 所以是「既有分组列表 + 就地新建」：点开先看到这作用域里已有的组（含「未分组」），
 * 最后一枚是「新建分组」—— 选中后在同一个面板里变成输入框，不再开第二层浮层。
 *
 * ## ⚠️ 为什么是**行内展开**，而不是 Popover / DropdownMenu
 *
 * 这个控件住在**抽屉（Radix `Sheet` = modal Dialog）里**，两条硬约束：
 *
 * 1. `DropdownMenu` 会吃掉字符键（当作 typeahead 跳项）、Esc 直接关菜单 ⇒ 组名敲不进去；
 * 2. `Popover` 是 Portal 到 `body` 的，落在 Dialog 的 FocusScope **之外**：
 *    Dialog 的焦点锁会立刻把焦点抢回抽屉 —— 表现是**输入框拿不到焦点、value 永远是空**
 *    （2026-10-18 实测：Playwright 里 `fill` 后立刻被 React 重置为空，键盘输入也进不去）；
 *    另外 Dialog 打开时给 `body` 设的 `pointer-events: none` 也会让浮层「看得见点不动」。
 *
 * 行内展开没有 Portal ⇒ 焦点、层级、点击都天然正确。面板在文档流里把下方内容推开，
 * 而不是盖住它们 —— 抽屉本来就窄，浮层盖住的往往正是用户想对照的那几行。
 */
export interface AccountGroupPickerProps {
  /** 当前分组；空串 = 未分组。 */
  value: string;
  /** 本作用域里已有的分组（调用方去重、排序）。不含「未分组」。 */
  groups?: string[];
  /** 由「更多 → 编辑分组」进入时自动展开。 */
  autoOpen?: boolean;
  disabled?: boolean;
  /** 账号显示名（用于 aria-label）。 */
  accountName?: string;
  /**
   * 提交分组。返回 `false` = 保存失败 —— 面板保持打开，让用户重试，
   * 而不是「关掉了但其实没存上」。
   */
  onCommit: (next: string) => Promise<boolean>;
}

export function AccountGroupPicker({
  value,
  groups = [],
  autoOpen = false,
  disabled = false,
  accountName,
  onCommit,
}: AccountGroupPickerProps) {
  const t = useT();
  const [open, setOpen] = useState(autoOpen && !disabled);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const current = value.trim();
  /** 当前值不在已有列表里（例如别的作用域设过）⇒ 也要列出来，否则勾选态无处可显示。 */
  const options = (() => {
    const list = groups.filter(Boolean);
    return current && !list.includes(current) ? [current, ...list] : list;
  })();
  const trimmed = draft.trim();
  const duplicated = trimmed !== "" && options.includes(trimmed);

  useEffect(() => {
    if (!open) {
      setCreating(false);
      setDraft("");
    }
  }, [open]);

  useEffect(() => {
    if (!creating) return undefined;
    const id = window.setTimeout(() => inputRef.current?.focus(), 20);
    return () => window.clearTimeout(id);
  }, [creating]);

  /* 点面板外收起：行内面板不会自己消失，用户点到别处就该认为「我不改了」。 */
  useEffect(() => {
    if (!open) return undefined;
    function onPointerDown(event: PointerEvent) {
      const root = rootRef.current;
      if (root && event.target instanceof Node && !root.contains(event.target)) setOpen(false);
    }
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  async function commit(next: string) {
    if (saving || disabled) return;
    if (next.trim() === current) {
      setOpen(false);
      return;
    }
    setSaving(true);
    const ok = await onCommit(next.trim());
    setSaving(false);
    if (ok) {
      setOpen(false);
      setCreating(false);
      setDraft("");
    }
  }

  return (
    <div ref={rootRef} className="min-w-0" data-slot="account-group-picker">
      <button
        type="button"
        disabled={disabled}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={t("shared.drawer.group.picker.aria", { name: accountName ?? "" })}
        onClick={() => setOpen((prev) => !prev)}
        className={cn(
          "-mx-1 flex min-w-0 max-w-full items-center gap-1.5 rounded-md px-1 text-left text-xs leading-5",
          "transition-colors hover:bg-foreground/[0.04] focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none",
          current ? "text-foreground/80" : "text-muted-foreground/70 hover:text-muted-foreground",
          disabled && "cursor-not-allowed opacity-60",
        )}
      >
        {current ? (
          <Tag className="size-3.5 shrink-0 stroke-[1.75] text-muted-foreground" aria-hidden="true" />
        ) : (
          <Plus className="size-3 shrink-0" aria-hidden="true" />
        )}
        <span className="min-w-0 flex-1 truncate">
          {current || t("shared.drawer.group.picker.unassigned")}
        </span>
        <ChevronDown
          className={cn("size-3 shrink-0 opacity-60 transition-transform", open && "rotate-180")}
          aria-hidden="true"
        />
      </button>

      {open && (
        <div
          role="listbox"
          aria-label={t("shared.drawer.group.picker.aria", { name: accountName ?? "" })}
          className="mt-1.5 rounded-lg border border-border bg-popover p-1 shadow-sm"
        >
          <div className="max-h-[184px] overflow-y-auto">
            <button
              type="button"
              role="option"
              aria-selected={!current}
              onClick={() => void commit("")}
              className={cn(
                "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors",
                "hover:bg-accent focus-visible:bg-accent focus-visible:outline-none",
                !current && "font-medium",
              )}
            >
              <span className="grid size-3.5 shrink-0 place-items-center">
                {!current ? <Check className="size-3.5 text-primary" /> : null}
              </span>
              <span className="min-w-0 flex-1 truncate text-muted-foreground">
                {t("shared.drawer.group.picker.unassigned")}
              </span>
            </button>
            {options.map((group) => {
              const selected = group === current;
              return (
                <button
                  key={group}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  onClick={() => void commit(group)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors",
                    "hover:bg-accent focus-visible:bg-accent focus-visible:outline-none",
                    selected && "font-medium",
                  )}
                >
                  <span className="grid size-3.5 shrink-0 place-items-center">
                    {selected ? <Check className="size-3.5 text-primary" /> : null}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{group}</span>
                </button>
              );
            })}
          </div>

          <div className="my-1 h-px bg-border" aria-hidden="true" />

          {creating ? (
            <div className="space-y-1.5 px-1 pb-1">
              <Input
                ref={inputRef}
                value={draft}
                disabled={saving}
                maxLength={20}
                spellCheck={false}
                autoComplete="off"
                placeholder={t("shared.drawer.group.picker.newPlaceholder")}
                aria-label={t("shared.drawer.group.picker.newPlaceholder")}
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    if (trimmed) void commit(trimmed);
                  } else if (event.key === "Escape") {
                    event.preventDefault();
                    event.stopPropagation();
                    setCreating(false);
                    setDraft("");
                  }
                }}
                className="h-7 text-xs"
              />
              {duplicated ? (
                <p className="text-[10.5px] leading-snug text-muted-foreground">
                  {t("shared.drawer.group.picker.exists")}
                </p>
              ) : null}
              <div className="flex items-center gap-1.5">
                <Button
                  type="button"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  disabled={!trimmed || saving}
                  onClick={() => void commit(trimmed)}
                >
                  {t("shared.drawer.group.picker.confirm")}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  disabled={saving}
                  onClick={() => {
                    setCreating(false);
                    setDraft("");
                  }}
                >
                  {t("shared.drawer.group.picker.cancel")}
                </Button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => {
                setCreating(true);
                setDraft("");
              }}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:bg-accent focus-visible:outline-none"
            >
              <span className="grid size-3.5 shrink-0 place-items-center">
                <Plus className="size-3.5" />
              </span>
              <span className="min-w-0 flex-1 truncate">{t("shared.drawer.group.picker.create")}</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
