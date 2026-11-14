/**
 * 聚合账号面板的**可见性筛选偏好**（issue #50 · R4 / D3.1）。
 *
 * ## 为什么是 `localStorage` 而不是 `app_prefs.json`
 *
 * 判据是**谁需要读它**：只有渲染层读（聚合页决定显示哪些卡），宿主（Rust）一辈子不看。
 * 留在 `localStorage` 就是同步可得、零 IPC；写进后端反而要为一枚纯前端偏好加一对命令
 * + 四处接线。这与 `update-preference.ts` / `shortcut-preference.ts` 同一条边界
 * （见 `types.ts` 里「纯渲染层偏好不在这里」那段）。
 *
 * ## 为什么只存「账号键数组」，不存「维度」
 *
 * 「只看 `trae-*`」与「只看这几个号」在语义上是**同一件事**：一个账号集合。
 * 若另记一个作用域维度，两种筛选叠加时就必须回答「以谁为准」——
 * 那个问题没有正确答案，只有一堆特例。所以只有一种表示：账号键数组。
 *
 * ## 为什么必须规范化（选满 ⇒ 删键）
 *
 * 账号键表示下，筛选生效期间**新增的账号不在键数组里**，因此默认不可见。
 * 如果用户把 32 个号全勾上、我们照实存 32 个键，那么第 33 个账号一入库就
 * **永远看不见**，而界面上还写着「显示全部」—— 那是谎报。
 * ⇒ 「选满」必须归一成「无筛选」（删键），这样全量态下新增账号自动可见。
 *
 * ## 读写是纯函数
 *
 * 接收 `storage` 入参（可传桩），因此能被门禁直接跑真代码断言 ——
 * 前端没有单测框架（同 `update-preference.ts:23-26` 的既有理由）。
 */

import { useCallback, useSyncExternalStore } from "react";

/**
 * `localStorage` 键。`v1` 前缀沿用既有惯例（语义若变可换键而不破坏旧值）。
 *
 * ⚠️ 与 `src/lib/scope.ts:124` / `update-preference.ts:31` / `shortcut-preference.ts:23` 同款。
 */
export const AGGREGATE_VISIBLE_KEY = "buddy-switch:v1:aggregate-visible";

/**
 * 读取偏好。返回**原始键数组**（不做有效性过滤）。
 *
 * 为什么不在这里过滤失效键：读取时机（应用启动）账号列表还没到，此时无法判断
 * 哪个键还有效。过滤必须等数据到位，由 `aggregate-accounts.ts::pruneVisibleKeys`
 * 在页面层做 —— 那里既能拿到全集，又是个纯函数（可被门禁断言）。
 *
 * 任何异常（键不存在 / JSON 损坏 / 存储被禁用）⇒ `null`（= 无筛选，全量可见）。
 * **绝不因偏好损坏而白屏** —— 偏好坏了是小事，页面打不开是大事。
 */
export function readAggregateVisible(storage: Pick<Storage, "getItem"> | undefined): string[] | null {
  try {
    const raw = storage?.getItem(AGGREGATE_VISIBLE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return null;
    const keys = parsed.filter((item): item is string => typeof item === "string" && item.includes(":"));
    return keys.length ? keys : null;
  } catch {
    return null;
  }
}

/**
 * 写入偏好。`null` / 空数组 ⇒ **删键**（等于「无筛选」）。
 *
 * 写失败静默：本次会话内仍按内存状态工作，只是记不住。
 */
export function writeAggregateVisible(
  storage: Pick<Storage, "setItem" | "removeItem"> | undefined,
  keys: readonly string[] | null,
): void {
  try {
    if (!keys || keys.length === 0) {
      storage?.removeItem(AGGREGATE_VISIBLE_KEY);
      return;
    }
    storage?.setItem(AGGREGATE_VISIBLE_KEY, JSON.stringify([...keys].sort()));
  } catch {
    /* 忽略：存储不可用时只是记不住 */
  }
}

/** 取可用的 `localStorage`（不可用返回 `undefined`）。 */
function safeStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

/**
 * 模块内缓存 —— `useSyncExternalStore` 的快照必须在两次渲染间**引用稳定**，
 * 否则每次 `getSnapshot()` 返回新数组都会触发无限重渲染。
 */
let current: string[] | null | undefined;
const listeners = new Set<() => void>();

function snapshot(): string[] | null {
  if (current === undefined) current = readAggregateVisible(safeStorage());
  return current ?? null;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** 设置偏好：持久化 + 更新缓存 + 通知订阅者。 */
export function setAggregateVisible(keys: readonly string[] | null): void {
  writeAggregateVisible(safeStorage(), keys);
  current = keys && keys.length ? [...keys] : null;
  for (const listener of listeners) listener();
}

/**
 * 订阅偏好。返回 `[raw, set]`。
 *
 * `getServerSnapshot` 固定 `null`：本应用是纯客户端，此分支不会被用到，
 * 但给出确定值可避免水合不一致（与 `useAutoCheckUpdate` 同）。
 */
export function useAggregateVisible(): [string[] | null, (keys: readonly string[] | null) => void] {
  const raw = useSyncExternalStore(subscribe, snapshot, () => null);
  const set = useCallback((keys: readonly string[] | null) => setAggregateVisible(keys), []);
  return [raw, set];
}

/**
 * 跨窗口 / 跨标签页同步（webui 可能开多个标签）。
 *
 * 桌面端是单窗口，但 webui 不是 —— 两个标签一个改了筛选、另一个还按旧值渲染，
 * 用户会以为「筛选没生效」。`storage` 事件是浏览器为这件事准备的唯一机制。
 *
 * @returns 取消订阅函数（`useEffect` 里直接 return）
 */
export function subscribeAggregateVisibleStorage(): () => void {
  if (typeof window === "undefined") return () => undefined;
  const onStorage = (event: StorageEvent) => {
    if (event.key !== AGGREGATE_VISIBLE_KEY) return;
    /* 别的标签改了盘 ⇒ 丢掉本进程缓存，让下一次 `snapshot()` 重读。
       ⚠️ 不能直接解析 `event.newValue`：那条路径绕过了 `readAggregateVisible`
       的容错，损坏的 JSON 会在这里抛出去。 */
    current = undefined;
    for (const listener of listeners) listener();
  };
  window.addEventListener("storage", onStorage);
  return () => window.removeEventListener("storage", onStorage);
}
