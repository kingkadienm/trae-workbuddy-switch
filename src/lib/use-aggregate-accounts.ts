/**
 * 聚合账号面板的**取数层**（issue #50 · D4）。
 *
 * ## 为什么合成**一把键**，而不是 6 个 `useCachedResource`
 *
 * 6 个独立资源会出现 6 个 `loading` / 6 个 `error`，页面就得自己拼「哪些还在转、
 * 哪些挂了」—— 那正是要避免的分叉。合成一个快照后，**部分失败**是快照里的一个字段
 * （`failed: Scope[]`），页面照着渲染「这一组挂了、其余照常」即可。
 *
 * ## 为什么是 `Promise.allSettled`
 *
 * 一条作用域取数失败（客户端没装、凭据读不到、命令超时）**不能拖垮整页**。
 * 这是 issue 的验收红线：任一作用域失败 ⇒ 该组不渲染卡片、其余照常、页面不白屏。
 * `Promise.all` 会在第一个失败时丢掉全部结果，正是要避免的行为。
 *
 * ## 三条命令的形态不同（这是本模块存在的第二个理由）
 *
 * | 平台 | 命令 | 吃 |
 * |:--|:--|:--|
 * | WorkBuddy | `accounts_list` | `region` |
 * | Trae | `get_trae_accounts` | 账号库变体 id |
 * | Qoder | `get_qoder_accounts` | `scope` |
 *
 * 映射集中在 `AGGREGATE_SOURCES`（纯函数层），这里只负责照着跑 —— 重试、演示数据
 * 都读同一份映射，不会出现「重试时用了错的命令」。
 */

import { useEffect, useMemo } from "react";

import * as api from "@/lib/api";
import {
  AGGREGATE_SOURCES,
  makeAggregateAccount,
  type AggregateAccount,
} from "@/lib/aggregate-accounts";
import type { Scope } from "@/lib/scope";
import { useCachedResource } from "@/lib/use-cached-resource";
import { useDocumentVisible } from "@/lib/use-document-visible";
import { useResourceStore, type ResourceEntry } from "@/stores/resources";

/** 聚合快照。`failed` 里的作用域**不渲染卡片**（由页面给「加载失败 + 重试」占位）。 */
export interface AggregateSnapshot {
  accounts: AggregateAccount[];
  failed: Scope[];
  /** 取数完成时刻（epoch 毫秒）。页面用它算「刚刚 / N 分钟前」。 */
  at: number;
}

/** 资源键。全局唯一：聚合页只有一个实例，不需要按作用域分键。 */
export const AGGREGATE_RESOURCE_KEY = "aggregate:accounts";

/**
 * 快照新鲜窗口：60 秒。
 *
 * 进出聚合页（或从聚合页点进某个作用域再回来）时，窗口内**连请求都不发**。
 * 6 个作用域 = 6 次后端调用，来回切几次就是几十次 —— 那是纯浪费。
 * 手动刷新走 `refresh()`（`force`），不受这个窗口限制。
 */
export const AGGREGATE_FRESH_MS = 60_000;

/** 单个作用域的取数（照 `AGGREGATE_SOURCES` 的映射分派）。 */
async function loadOne(source: (typeof AGGREGATE_SOURCES)[number]): Promise<AggregateAccount[]> {
  if (source.platform === "workbuddy") {
    const { accounts } = await api.getAccounts(source.region);
    return accounts.map((account) => makeAggregateAccount(source.scope, "workbuddy", account));
  }
  if (source.platform === "trae") {
    const overview = await api.getTraeAccounts(source.variant);
    return overview.accounts.map((account) => makeAggregateAccount(source.scope, "trae", account));
  }
  const view = await api.getQoderAccounts(source.scope);
  return view.accounts.map((account) => makeAggregateAccount(source.scope, "qoder", account));
}

/**
 * 取全部 6 个作用域。**永不抛**（单条失败进 `failed`）。
 *
 * 导出是为了让门禁能在 node 里直接跑它（纯异步函数，不依赖 React）。
 */
export async function loadAggregateSnapshot(): Promise<AggregateSnapshot> {
  const results = await Promise.allSettled(AGGREGATE_SOURCES.map((source) => loadOne(source)));
  const accounts: AggregateAccount[] = [];
  const failed: Scope[] = [];
  results.forEach((result, index) => {
    if (result.status === "fulfilled") {
      accounts.push(...result.value);
    } else {
      failed.push(AGGREGATE_SOURCES[index].scope);
    }
  });
  return { accounts, failed, at: Date.now() };
}

/** 聚合页的取数结果。 */
export interface AggregateAccountsState {
  accounts: AggregateAccount[];
  failed: Set<Scope>;
  /** 首屏骨架判据：**没有可用快照**时才为 true（有旧值时的后台刷新不闪骨架）。 */
  loading: boolean;
  /** 有请求在飞（含后台刷新）—— 刷新按钮的转圈绑它。 */
  pending: boolean;
  error: string | null;
  /** 强制重取（刷新按钮 / 单组重试）。 */
  reload: () => Promise<void>;
}

/**
 * 聚合账号总数（**跨挂载存活的共享事实**）。
 *
 * ## 为什么顶栏与聚合页必须读**同一个**数
 *
 * 顶栏作用域菜单里的「聚合」入口写「N 个账号」，聚合页自己的工具栏也写「N 个账号」——
 * 两个 N 必须是同一个数，否则用户点进去发现对不上，第一反应是「这应用有 bug」。
 *
 * ⚠️ 曾出现过「顶栏 13 / 聚合页 29」：顶栏数的是 `useScopeStatus`（WB 侧只读 store 的
 * **当前区域切片**，没进过国际版账号页时国际版是 0），聚合页数的是自己拉的 6 个作用域。
 * **两个不同的数据源** ⇒ 必然对不上。
 *
 * ⇒ 顶栏改读本 hook（**同一个资源键** ⇒ 同一份快照、同一个 in-flight 请求）。
 *
 * @param enabled 是否真的去取数。
 *   顶栏**只在作用域菜单打开时**才取：菜单外的那个数字没人看，而 6 个作用域的账号
 *   列表是 6 次后端调用 —— 每次启动都拉一遍不划算。
 *   `false` 时**不发起请求**（`useCachedResource` 传 `null` 键 = 不加载）。
 */
export function useAggregateAccountTotal(enabled: boolean): {
  /** 账号总数；`null` = 还没取到（**不是 0**）。 */
  total: number | null;
  /** 取数失败时的文案（`null` = 没有错误）。 */
  error: string | null;
} {
  /* ⚠️ **直接读 store**，不走 `useCachedResource`：那个 hook 在 `key === null` 时
     连缓存都读不到（`entries[key]` 直接 undefined），于是「菜单外显示已取到的总数」
     这件事做不到。而这里恰恰需要它：进过聚合页后，顶栏按钮上的「N 个账号」
     必须与聚合页一致，不能因为菜单没开就变回「未知」。 */
  const entry = useResourceStore((s) => s.entries[AGGREGATE_RESOURCE_KEY]) as
    | ResourceEntry<AggregateSnapshot>
    | undefined;
  const load = useResourceStore((s) => s.load);

  /* 菜单打开时按需加载（菜单外那个数字没人看，而 6 个作用域 = 6 次后端调用）。 */
  useEffect(() => {
    if (!enabled) return;
    void load(AGGREGATE_RESOURCE_KEY, loadAggregateSnapshot, { freshMs: AGGREGATE_FRESH_MS });
  }, [enabled, load]);

  const value = entry?.value;
  return {
    total: value ? value.accounts.length : null,
    error: entry?.error == null ? null : api.asError(entry.error),
  };
}

export function useAggregateAccounts(): AggregateAccountsState {
  const resource = useCachedResource<AggregateSnapshot>(
    AGGREGATE_RESOURCE_KEY,
    loadAggregateSnapshot,
    { freshMs: AGGREGATE_FRESH_MS },
  );
  const visible = useDocumentVisible();
  const reload = resource.refresh;
  const revalidate = resource.revalidate;

  const failed = useMemo(() => new Set<Scope>(resource.data?.failed ?? []), [resource.data]);

  /* 回到窗口时**再校验一次**（不强发）：60 秒窗口内已有值就连请求都不发。
     用 `revalidate` 而不是 `refresh` —— 后者会无视窗口、每切一次窗口发 6 个请求。 */
  const hasData = resource.data !== undefined;
  useEffect(() => {
    if (visible && hasData) void revalidate();
  }, [visible, hasData, revalidate]);

  return {
    accounts: resource.data?.accounts ?? [],
    failed,
    loading: resource.loading,
    pending: resource.pending,
    error: resource.error,
    reload,
  };
}

/** 单组重试：只重拉一个作用域，成功后并回快照（失败仍留在 `failed` 里）。 */
export async function reloadOneScope(scope: Scope): Promise<AggregateAccount[] | null> {
  const source = AGGREGATE_SOURCES.find((item) => item.scope === scope);
  if (!source) return null;
  try {
    return await loadOne(source);
  } catch {
    return null;
  }
}
