import type { ExtClientId } from "@/lib/types";

/**
 * P14 S4a · **程序位 → 插件会话仓客户端**的映射。
 *
 * ## 为什么需要这张表，以及为什么它有「查不到」
 *
 * 会话仓的目录形状是 `<数据根>/CodeBuddyExtension/Data/<uid>/<客户端段>/<uid>/history`
 * —— 客户端段只有 **`VSCode`** 与 **`CodeBuddyIDE`** 两个值。
 * 而账号页的程序位比这多（还有 CLI、JetBrains）⇒ **必然有程序位没有对应会话仓**。
 *
 * ⚠️ **`jetbrains_codebuddy_plugin` 刻意不在这张表里**：
 * JetBrains 插件的数据仓**没有** `VSCode` / `CodeBuddyIDE` 这两段中的任何一段
 * （本仓的模块头只认这两个客户端段）。给它硬塞一个映射，等于**去读一棵不属于它的树** ——
 * 那比「没有这个功能」坏得多。
 *
 * ⇒ 查不到就返回 `null`，调用方据此**不渲染入口**（而不是渲染一个点了会读错目录的按钮）。
 */
const PROGRAM_TO_EXT_CLIENT: Readonly<Record<string, ExtClientId>> = {
  /** CodeBuddy IDE（国内版桌面客户端）。 */
  codebuddy_cn_ide: "codebuddy-ide",
  /** VS Code CodeBuddy 插件（只读探测槽）。 */
  vscode_cn_inject: "vscode",
  /** VS Code CodeBuddy 扩展（可切换槽）。与上面**同一个数据仓**。 */
  vscode_codebuddy_plugin: "vscode",
};

/** 该程序位对应的插件会话仓客户端；**没有对应仓 ⇒ `null`**（调用方据此不渲染入口）。 */
export function extClientForProgram(programId: string): ExtClientId | null {
  return PROGRAM_TO_EXT_CLIENT[programId] ?? null;
}

/** 客户端显示名（抽屉标题用；与 `ExtClientId` 一一对应）。 */
export const EXT_CLIENT_LABEL_KEY: Readonly<Record<ExtClientId, string>> = {
  vscode: "sessions.ext.client.vscode",
  "codebuddy-ide": "sessions.ext.client.codebuddyIde",
};
