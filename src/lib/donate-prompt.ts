/**
 * 打赏引导的「第 N 次启动」计数与去重。
 *
 * ## 为什么放 `localStorage`
 *
 * 它只影响**这台设备上**的引导节奏：宿主（Rust）不读、托盘不用、也不需要跨设备同步。
 * 按 `app_prefs.rs` 模块头定的界线（「谁需要读它」），这类纯渲染层偏好就该留在这里，
 * 与 `first-run.ts` / `update-preference.ts` 同款。
 *
 * ## 三条硬约束
 *
 * 1. **每次启动只计一次**：React StrictMode 会把 effect 跑两遍，用模块级 `countedThisLoad`
 *    守住 —— 否则用户开两次应用就以为到了第三次。
 * 2. **「记已提示」与「该弹」在同一刻判定**（`consumeDonatePrompt`）：先弹后记，中途崩溃
 *    就会每次启动都弹一次。
 * 3. **写不了就当作不需要弹**（隐私模式 / 禁用 storage）：宁可少弹一次，也不要变成一个
 *    每次启动都关不掉的弹窗。
 */

const LAUNCH_KEY = "buddy-switch:launch-count";
const PROMPTED_KEY = "buddy-switch:donate-prompted";

/** 第几次启动时自动弹打赏引导。改这个数字 = 改产品行为，不是调参。 */
export const DONATE_PROMPT_AT_LAUNCH = 3;

let countedThisLoad = false;

function safeStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

function readCount(storage: Storage): number {
  const raw = storage.getItem(LAUNCH_KEY);
  const value = raw ? Number.parseInt(raw, 10) : 0;
  return Number.isFinite(value) && value > 0 ? value : 0;
}

/**
 * 记录一次启动，返回累计启动次数。
 *
 * 同一次页面加载内重复调用只返回当前值（不重复加一）—— App 在 StrictMode 下会挂载两次。
 */
export function noteLaunch(): number {
  const storage = safeStorage();
  if (!storage) return 0;
  const current = readCount(storage);
  if (countedThisLoad) return current;
  countedThisLoad = true;
  const next = current + 1;
  try {
    storage.setItem(LAUNCH_KEY, String(next));
  } catch {
    return 0;
  }
  return next;
}

/**
 * 是否该自动弹打赏引导：**到了阈值且从未弹过**才返回 `true`，并当场记为已提示。
 *
 * 调用方负责先决定「这次要不要弹」（例如风险确认还没通过就跳过），
 * **只有真要弹的时候才调用本函数** —— 否则一口气把「已提示」用掉了。
 */
export function consumeDonatePrompt(): boolean {
  const storage = safeStorage();
  if (!storage) return false;
  if (readCount(storage) < DONATE_PROMPT_AT_LAUNCH) return false;
  try {
    if (storage.getItem(PROMPTED_KEY) === "1") return false;
    storage.setItem(PROMPTED_KEY, "1");
  } catch {
    return false;
  }
  return true;
}
