/**
 * Trae 程序位装配（issue #50 · 聚合页复用）。
 *
 * ## 为什么抽出来
 *
 * `TraeAccountsPage` 的 `programsFor(account)` 原本是页面内的闭包函数，依赖页面私有的
 * `variant` / `variantStatuses` / `logins` 三个变量。聚合页要渲染同一批程序位胶囊，
 * 若把这段**再抄一份**，两处就会各自演化 —— 最容易漂的是「`current` 的判定」：
 * 登录态是**客户端级**的，比的是 `userId`（身份）而不是 `name`（展示名），
 * 后者改名即失配。抄错的后果是「账号页说这不是当前账号、聚合页说它是」。
 *
 * ⇒ 抽成**纯函数**，两边共用一份（参数化传入三个依赖）。
 *
 * ## 三条判据（与 Trae 账号页逐字一致）
 *
 * ① **只取当前区域**的程序位（用户 2026-10-10 的明确要求：国内版页面上不要挂着国际版的）。
 * ② **绝不按 `program` 去重**：`TraeProgramStatus.program` 不含区域，去重会把
 *    `(cn, trae_work)` 与 `(global, trae_work)` 合并、丢掉「该写哪个客户端」。
 * ③ `current` 比 `userId`；`logins` 读不到时是 `null`，**不能**与空 `userId` 相互匹配。
 */

import type { TraeProgram } from "@/components/trae-account-card";
import type { TraeAccount, TraeRegionId, TraeVariantId, TraeVariantStatus } from "@/lib/trae-types";
import type { TraeVariantLogins } from "@/lib/trae-variant-status";

export function traeProgramsFor(
  account: TraeAccount,
  variant: TraeVariantId,
  statuses: readonly TraeVariantStatus[],
  logins: TraeVariantLogins,
): TraeProgram[] {
  return statuses
    .filter((status) => status.variant === variant)
    .flatMap((status) =>
      status.programs.map((program) => ({
        region: status.variant as TraeRegionId,
        variant: program.variant,
        program: program.program,
        label: program.label,
        installed: program.installed,
        /* 「装了」不等于「能用」：客户端从未启动过时它的 userData 目录不存在 ⇒
           切换永远不可能成功。取**写侧**目录的存在性（与后端保存 / 恢复同源）。 */
        hasDataDir: program.writeDataDirExists,
        current: Boolean(account.userId) && logins[program.variant]?.userId === account.userId,
      })),
    );
}
