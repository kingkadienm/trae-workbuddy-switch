/**
 * 侧栏折叠态（只显示图标）—— **纯渲染层偏好 ⇒ localStorage**。
 *
 * ## 为什么不进 core `app_prefs.json`
 *
 * 持久化按**读者**选层（见 `.workbuddy-ai/memory/MEMORY.md`）：
 * `close_to_tray` 这类宿主也读的偏好必须进 core，否则托盘菜单与窗体行为会不一致；
 * 而侧栏折叠**只有前端渲染层读**，没有第二个消费者 ⇒ 放 localStorage 即可，
 * 进 core 反而要多拖一条命令契约（`app_prefs_*`）出来维护。
 *
 * ## 键名带 `buddy-switch.` 前缀
 *
 * 与 `theme.ts` 同一命名空间：同一应用写进 localStorage 的键不该混用两套前缀，
 * 否则清理 / 迁移时得先猜哪个是哪个。
 */
const SIDEBAR_STORAGE_KEY = "buddy-switch.sidebarCollapsed";

export function getSidebarCollapsed(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_STORAGE_KEY) === "1";
  } catch {
    // 存储不可用（隐私模式 / 配额满）时按展开处理：折叠是「锦上添花」，
    // 不该因为读不到偏好就把侧栏藏起来 —— 那会让用户找不到导航。
    return false;
  }
}

export function setSidebarCollapsed(collapsed: boolean): void {
  try {
    localStorage.setItem(SIDEBAR_STORAGE_KEY, collapsed ? "1" : "0");
  } catch {
    // 存储不可用时**仍然切换**：这一帧的界面状态由调用方持有，
    // 写不进去只意味着「下次启动回到展开」，不是「这次不折叠」。
  }
}
