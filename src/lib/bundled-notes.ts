/**
 * 打包内置的版本说明（离线兜底）。
 *
 * ## 它是什么、不是什么
 *
 * 文件由 `npm run notes:json` 生成（`scripts/gen-release-notes.mjs --json`），内容是
 * **本应用这一版**的说明。它的用途只有一个：**网络取不到新版本说明时，也能说清
 * 「你正在跑的这一版是什么」** —— 而不是假装它就是新版本的说明。
 *
 * 因此 `resolveNotesFor` 只在两种情况下采用它：
 *   ① 后端抓到的 `update.notes` 版本号与它一致（说明就是这一版）；
 *   ② 用户点升级时发现「已经是最新版」——这时「本版改了什么」正是用户要的答案。
 *
 * 版本号不一致（应用落后、而网络又挂了）⇒ **返回 `null`**，由弹窗显示「这次发布没有
 * 附带说明」的真话。拿旧版本的说明冒充新版本，比什么都不显示更坏。
 *
 * 为什么不做成后端 `include_str!`：文件要随前端一起被打包才有意义（`vite` 已经把它
 * 编进产物），塞进 Rust 反而多一处「改了前端文件要重编 Rust」的耦合。
 */

import bundledRaw from "@/data/release-notes.json";
import { normalizeReleaseNotes } from "@/lib/release-notes";
import type { ReleaseNotes, UpdateInfo } from "@/lib/types";

/** 归一化后的内置说明（模块级缓存：归一化只做一次）。 */
let cached: ReleaseNotes | null | undefined;

export function bundledReleaseNotes(): ReleaseNotes | null {
  if (cached === undefined) cached = normalizeReleaseNotes(bundledRaw);
  return cached;
}

/**
 * 弹窗最终要渲染的说明。
 *
 * 优先级：后端抓到的这份（版本对得上才用） → 内置的（版本对得上才用） → `null`。
 * 「版本对得上」的判据：等于 `update.latest`，或等于 `update.current`（本版说明）。
 */
export function resolveNotesFor(update: UpdateInfo | null): ReleaseNotes | null {
  const remote = update?.notes ?? null;
  if (remote) return remote;

  const bundled = bundledReleaseNotes();
  if (!bundled) return null;
  if (bundled.version === update?.latest) return bundled;
  if (bundled.version === update?.current) return bundled;
  return null;
}
