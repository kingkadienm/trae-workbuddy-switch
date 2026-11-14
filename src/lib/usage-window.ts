/**
 * 用量页「视图参数」的**时间窗口** —— 整页唯一的一份定义。
 *
 * ## 为什么把 `UsageRange = 7 | 14 | 30` 换成一个联合类型
 *
 * 参数条原来只有三档 `[7, 14, 30]`，`range` 是个**天数**，所有窗口函数都在
 * 用算术推日期（`now - (range - 1 - offset) * DAY_MS`）。加「今天」与
 * 「自定义时间」后，天数**不再能表达用户选的东西** ——「今天」碰巧是 1 天，
 * 但「9/20 – 9/22」是**锚在日期上**的一段区间，它随「今天」移动而移动。
 * 若继续用天数硬拟合，会出现「9/20–9/22 选了 3 天，明天看到的是 9/23–9/25」——
 * 正是原型里刻意避开的那个谎报（`docs/prototype/demo-app.js` 的 `rangeWindow` 注释）。
 *
 * ⇒ 窗口的**唯一真值是一对日期键**；「近 N 天」只是**生成这对键的一种方式**。
 *
 * ## 为什么窗口有 90 天上限
 *
 * 后端 `token_stats` 的 `days` 只认 `7 / 30 / 90`（其余值 ⇒ `None` = 全量扫），
 * 90 天是它承认的**最宽档**。自定义若放开到任意历史，逐日点会无上界增长，
 * 且「日均」这类分母会失去意义。故上限取 90，与后端最宽档**同口径**。
 *
 * ## 本模块**不导入 React**（只有 type 导入，转译后消失）
 *
 * 与 `usage-charts.ts` 同规矩：纯函数可被门禁直接求值做运行时校验。
 */

import type { TranslationKey } from "@/locales/zh";

/** 预设档位（天）。与参数条 `.seg` 的前四枚一一对应（「今天」另算）。 */
export type UsagePresetDays = 7 | 14 | 30;

/** 窗口覆盖的最多日历日 —— 与后端 `token_stats` 承认的最宽档（90）同口径。 */
export const USAGE_WINDOW_MAX_DAYS = 90;

/**
 * 时间窗口。`kind` 是**模式**：只有 `preset` 才有 `days`。
 *
 * `custom` 的 `from` / `to` 是**闭区间**的本地日期键（`YYYY-MM-DD`，与后端逐日桶同口径）。
 */
export type UsageWindow =
  | { kind: "today" }
  | { kind: "preset"; days: UsagePresetDays }
  | { kind: "custom"; from: string; to: string };

/** 默认窗口 = 近 30 天（等价于改造前的 `useState<UsageRange>(30)`）。 */
export const DEFAULT_USAGE_WINDOW: UsageWindow = { kind: "preset", days: 30 };

/** `.seg` 的档位键。**字符串**，可直接当 `Seg` 的 `value`（它的泛型允许 `string`）。 */
export type UsageWindowMode = "today" | "7" | "14" | "30" | "custom";

/** 档位顺序 —— 参数条按它渲染，顺序与原型逐值一致。 */
export const USAGE_WINDOW_MODES: readonly UsageWindowMode[] = ["today", "7", "14", "30", "custom"];

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * 本地日期键 `YYYY-MM-DD`（与后端逐日桶同口径，不走 `toISOString` —— 那会偏时区）。
 *
 * ★ 本模块是 `dateKey` 的**定义处**（原先在 `usage-charts.ts`）：
 * 窗口的边界与「窗口内有哪几天」是同一件事，两者分居两个文件必然在跨时区 / 跨月时分叉。
 */
export function dateKey(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 日期键 → 本地零点。⚠️ 不用 `new Date(key)`：那按 **UTC** 解析，会偏一天。 */
function parseKey(key: string): Date {
  return new Date(`${key}T00:00:00`);
}

/** 合法的日期键（`YYYY-MM-DD` 且可解析）。非法 ⇒ `null`（**fail-closed**，不猜）。 */
export function parseDateKey(value: unknown): string | null {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
  const date = parseKey(value);
  return Number.isNaN(date.getTime()) ? null : dateKey(date);
}

/** 今天（本地）的日期键。 */
export function todayKey(now: number = Date.now()): string {
  return dateKey(new Date(now));
}

/** 自定义窗口**可选的最早日期**（今天往前 `USAGE_WINDOW_MAX_DAYS - 1` 天）。 */
export function usageWindowEarliestKey(now: number = Date.now()): string {
  return dateKey(new Date(now - (USAGE_WINDOW_MAX_DAYS - 1) * DAY_MS));
}

/**
 * 把自定义日期**收敛到合法区间**：非法 ⇒ 退回今天；越界 ⇒ 夹到边界；倒序 ⇒ 交换。
 *
 * ★ 与原型 `rangeWindow` 的 custom 分支同一条取舍：**不信任入参**。
 * 日期来自 `<input type="date">` 与 URL，两处都可能给出越界值。
 */
function clampCustom(
  from: unknown,
  to: unknown,
  now: number,
): { from: string; to: string } {
  const earliest = usageWindowEarliestKey(now);
  const latest = todayKey(now);
  const lo = parseDateKey(from) ?? latest;
  const hi = parseDateKey(to) ?? latest;
  const [left, right] = lo <= hi ? [lo, hi] : [hi, lo];
  return {
    from: left < earliest ? earliest : left,
    to: right > latest ? latest : right,
  };
}

/** 窗口的**闭区间**日期键。 */
export function usageWindowBounds(
  window: UsageWindow,
  now: number = Date.now(),
): { from: string; to: string } {
  if (window.kind === "today") {
    const key = todayKey(now);
    return { from: key, to: key };
  }
  if (window.kind === "custom") return clampCustom(window.from, window.to, now);
  return {
    from: dateKey(new Date(now - (window.days - 1) * DAY_MS)),
    to: todayKey(now),
  };
}

/**
 * 窗口内的**全部日期键**（升序，闭区间）—— 「窗口里有哪几天」的**唯一实现**。
 *
 * ★ 逐日 `setDate(+1)` 而不是 `diff / DAY_MS`：夏令时那天不是 24 小时，
 * 相除会得到小数而被 `Math.round` 掩掉，跨月时表现为「少一天 / 多一天」。
 * 循环上限取 `USAGE_WINDOW_MAX_DAYS`，异常入参也不会变成死循环。
 */
export function usageWindowKeys(window: UsageWindow, now: number = Date.now()): string[] {
  const { from, to } = usageWindowBounds(window, now);
  const keys: string[] = [];
  const cursor = parseKey(from);
  const end = parseKey(to).getTime();
  while (cursor.getTime() <= end && keys.length < USAGE_WINDOW_MAX_DAYS) {
    keys.push(dateKey(cursor));
    cursor.setDate(cursor.getDate() + 1);
  }
  return keys.length > 0 ? keys : [todayKey(now)];
}

/** 窗口覆盖的**日历天数**（≥ 1）—— 「日均」这类分母必须用它，不是「有数据的天数」。 */
export function usageWindowDays(window: UsageWindow, now: number = Date.now()): number {
  return usageWindowKeys(window, now).length;
}

/**
 * 从今天往回数多少天才能**覆盖**该窗口 —— Trae 侧取数用（它的统计接口按
 * 「最近 N 天」拉日志，`days <= 0` 表示不限）。
 *
 * ★ 为什么不直接用 `usageWindowDays`：窗口可能是**锚在过去**的一段
 * （如 9/1–9/5），拉 5 天只会拉到最后 5 天，那段历史**根本不在返回里**。
 */
export function usageWindowSpanDays(window: UsageWindow, now: number = Date.now()): number {
  const { from } = usageWindowBounds(window, now);
  const today = parseKey(todayKey(now)).getTime();
  const cursor = parseKey(from);
  let span = 1;
  while (cursor.getTime() < today && span < USAGE_WINDOW_MAX_DAYS) {
    cursor.setDate(cursor.getDate() + 1);
    span += 1;
  }
  return span;
}

/** 窗口当前对应哪一个 `.seg` 档位。 */
export function usageWindowMode(window: UsageWindow): UsageWindowMode {
  if (window.kind === "today") return "today";
  if (window.kind === "custom") return "custom";
  return String(window.days) as UsageWindowMode;
}

/**
 * 档位 → 窗口。
 *
 * ⚠️ `custom` **不是「一按就生效」的档位**（它要开弹层选日期）⇒ 原样返回 `previous`。
 * 参数条在 `Seg` 的 `onChange` 里拦下 `custom` 去开弹层，不会走到这里的 `custom` 分支；
 * 这个分支存在只是为了让映射**全覆盖**（不留 `undefined` 语义）。
 */
export function usageWindowFromMode(
  mode: UsageWindowMode,
  previous: UsageWindow,
): UsageWindow {
  if (mode === "today") return { kind: "today" };
  if (mode === "custom") return previous;
  return { kind: "preset", days: Number(mode) as UsagePresetDays };
}

/** 固定档位的词条键；`custom` 返回 `null`（它没有固定文案，走 `usageCustomRangeLabel`）。 */
export function usageWindowKey(window: UsageWindow): TranslationKey | null {
  if (window.kind === "today") return "wbStats.usage.range.today";
  if (window.kind === "custom") return null;
  return `wbStats.usage.range.${window.days}` as TranslationKey;
}

/** `M/D`（跨年时带年份）—— 自定义区间的轴标签。 */
function shortKey(key: string, withYear: boolean): string {
  const [year, month, day] = key.split("-");
  const md = `${Number(month)}/${Number(day)}`;
  return withYear ? `${year}/${md}` : md;
}

/**
 * **单个**日期键的短文案（`7/18`）。
 *
 * ⚠️ 与 `usageCustomRangeLabel` 分开：**边界提示**是「最早可选 – 最晚可选」两个**独立日期**，
 * 拿区间文案去渲染单个日期会得到 `7/18 – 7/18`，再套进 `{from} – {to}` 就变成
 * `7/18 – 7/18 – 10/15`（实测踩到）。
 */
export function shortDateLabel(key: string, now: number = Date.now()): string {
  const safe = parseDateKey(key) ?? todayKey(now);
  return shortKey(safe, safe.slice(0, 4) !== todayKey(now).slice(0, 4));
}

/**
 * 自定义区间的文案：`9/20 – 9/25`。
 *
 * ★ 跨年时才带年份：窗口上限 90 天，绝大多数情况同一年，带年份只是噪声；
 * 但跨年时**必须**带，否则「12/28 – 1/5」会让人以为是反的。
 */
export function usageCustomRangeLabel(
  window: Extract<UsageWindow, { kind: "custom" }>,
  now: number = Date.now(),
): string {
  const { from, to } = clampCustom(window.from, window.to, now);
  const crossYear = from.slice(0, 4) !== to.slice(0, 4);
  return `${shortKey(from, crossYear)} – ${shortKey(to, crossYear)}`;
}
