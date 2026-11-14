/**
 * 用量页「视图参数」的**兼容读**（只读，**不回写**）。
 *
 * ## 背景
 *
 * 改造前 Token 统计页与积分统计页**各自**记一份「统计范围」偏好
 * （`buddy-switch:token-stats:region` / `buddy-switch:credit-stats:region`），
 * 值域是 `cn | global | all`。其中 `cn` / `global` 早就由 `useScope` 接管
 * （URL 优先 + localStorage 记忆），**只有 `all` 还有意义** —— 它是**视图参数**
 * （合并两个区域看），不是作用域。
 *
 * 现在参数条收敛到 `UsagePage` 一处、合并态只有一份，于是需要把「老用户存过 `all`」
 * 这件事读出来当初始值：**否则升级后他的合并态被静默重置为「仅本作用域」**，
 * 看起来像「数据少了一半」。
 *
 * ## 为什么不回写
 *
 * 回写会与 `useScope` 打架（两个真值）。门禁
 * `★ 统计页不再回写旧的「各页独立」偏好键` 就是钉这条。
 *
 * ## 为什么两个键都读
 *
 * 两个键是**历史遗留的两份**：只在积分页选过合并的用户、只在 Token 页选过的用户
 * 都存在。任一个为 `all` 即视为 `true`（保守取「用户想看全部」）。
 */

/** 两个历史键（Token 页 / 积分页各一份）。 */
const LEGACY_COMBINE_KEYS = [
  "buddy-switch:token-stats:region",
  "buddy-switch:credit-stats:region",
] as const;

/** 读合并态初始值；storage 不可用 / 无记录 ⇒ `false`（与原型默认「仅本作用域」一致）。 */
export function readPreferredCombineAll(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return LEGACY_COMBINE_KEYS.some((key) => window.localStorage.getItem(key) === "all");
  } catch {
    return false;
  }
}
