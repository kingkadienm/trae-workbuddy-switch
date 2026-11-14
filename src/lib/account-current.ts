/**
 * 「某个 WorkBuddy 账号是不是**客户端当前登录的那个**」—— 纯判定，两处共用。
 *
 * ## 为什么要抽出来
 *
 * 账号页（`AccountsPage`）与聚合页（`AggregateAccountsPage`）都要回答这个问题，
 * 用来点亮卡片左侧的竖条与「当前登录」徽章。两处各写一份的后果不是「重复」这么轻：
 * 判据里有一条**非对称的兜底**（`current.uid` 可能存的是账号 id，也可能是 uid），
 * 漏掉它就会出现「账号页有徽章、聚合页没有」—— 同一件事两种结论，用户只会认为其中一处坏了。
 *
 * ## 判据
 *
 * 依次比 `uid`（两种可能形态）与 `email`。任一命中即当前 —— 因为后端 `AppStatus.current`
 * 是**客户端认证文件**读出来的，它的主键口径与账号库的主键并不总是同一个字段
 * （WB 客户端存 `uid`，账号库的 `id` 在部分历史数据里就是 `uid`）。
 *
 * ⚠️ 没有 `current`（未登录 / 读不到）⇒ `false`，**不猜**。
 */

import type { AccountMeta, AppStatus } from "@/lib/types";

export function isWorkbuddyCurrent(
  account: AccountMeta,
  current: AppStatus["current"] | undefined,
): boolean {
  if (!current) return false;
  return Boolean(
    (current.uid && (account.uid === current.uid || account.id === current.uid)) ||
      (current.email && account.email === current.email),
  );
}
