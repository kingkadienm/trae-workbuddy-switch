/**
 * 签到历史的**日期口径**（两平台共用）。
 *
 * ## 为什么单独一个模块
 *
 * 「近 7 天」的抽屉区块两个平台都有（WorkBuddy 走 `getCheckinLogs()`、
 * Trae 走 `get_trae_logs({ kind: "checkin" })`），**日期键的口径必须一致** ——
 * 差一天就会把「昨天签的」显示成「未签到」。
 *
 * ⚠️ 必须用**本地**日期，不能用 `toISOString().slice(0,10)`：后者是 UTC，
 * 在东八区会把 08:00 之前的记录算到前一天。
 */

/** 本地日期键（`YYYY-MM-DD`），与后端签到日志 / 积分统计事件的 `date` 同口径。 */
export function localDateKey(ts: number): string {
  const date = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 近 7 天本地日期（含今天，**倒序**）。 */
export function lastSevenDays(): string[] {
  const days: string[] = [];
  const base = new Date();
  for (let i = 0; i < 7; i += 1) {
    days.push(localDateKey(new Date(base.getFullYear(), base.getMonth(), base.getDate() - i).getTime()));
  }
  return days;
}
