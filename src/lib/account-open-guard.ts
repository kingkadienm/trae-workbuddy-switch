/**
 * 「点这一下算不算『点账号本身』」—— 整卡 / 整行打开详情前的**唯一判据**。
 *
 * ## 为什么必须抽成共用纯函数（2026-10-22 用户报障）
 *
 * 报障原文：*「账号卡片的更多操作中，每次点击上面的任何一个操作，侧拉抽屉都被打开」*。
 *
 * 根因不是漏写了某个按钮的守卫，而是**守卫的判据选错了维度**：
 *
 * - 菜单内容由 Radix `Portal` 渲染到 `<body>` 下（`ui/dropdown-menu.tsx`），
 *   **DOM 上不在卡片里**，但 **React 树里仍在卡片里** ⇒ 冒泡照样回到 `<article onClick>`；
 * - 菜单项是 `Primitive.div[role="menuitem"]`（见 `@radix-ui/react-menu`），
 *   **不是 `<button>`** ⇒ 原先那串 `closest("button, a, input, …")` 一条也命中不了；
 * - 于是「点刷新 / 签到 / 导出 / 删除」全部被当成「点卡片」⇒ 详情抽屉跟着弹出来。
 *
 * 顺着原判据继续打补丁（再往选择器里塞 `[role='menuitem']`）是错的：
 * `select` / `popover` / `dialog` / 命令面板都是同一套 Portal 手法，
 * 每加一个控件就要再补一次，且**漏了不会报错**，只会静默打开抽屉。
 *
 * ## 判据
 *
 * 不猜「点到了哪个控件」，而是先问「这一点**在不在**这张卡的 DOM 里」：
 *
 * 1. 不在 `currentTarget` 子树内 ⇒ 是 Portal 里的弹层内容，一律不算点卡片；
 * 2. 在子树内，但落在交互控件上（按钮 / 链接 / 输入 / 勾选 / 开关 / 显式豁免）⇒ 不算点卡片；
 * 3. 其余 ⇒ 算点卡片，打开详情。
 *
 * 第 1 条顺带覆盖了「拖选文字后在卡片外松手」「弹层内部滚动」等零散路径 ——
 * 它们的 `target` 都不在卡片里。
 *
 * ## 与 `onKeyDown` 的分工
 *
 * 键盘路径（Enter / Space）不经过 PointEvent 守卫，仍由各卡片自己的
 * `e.target !== e.currentTarget` 判断拦住（内部控件自己会消费按键）。
 */

/**
 * 卡片内部**不打开详情**的控件。
 *
 * ⚠️ `[role='button']` 不是冗余：`DemoAction`（演示模式）把真实按钮包在
 * `span[role="button"]` 里并禁用原生按钮 —— 那一层 span 才是事件的 target，
 * 而它**不是** `<button>`。只列标签名就会漏掉它（2026-10-22 实测：demo 模式下
 * 点 Qoder 卡片的「更多」仍会打开抽屉）。
 *
 * ⚠️ `[data-no-account-open]` 是给「长得不像控件、但点了也不该开详情」的装饰区留的逃生口；
 * 新增例外优先用它，**不要**往这个选择器里加业务类名。
 */
export const ACCOUNT_OPEN_IGNORE_SELECTOR =
  "button, a, input, label, select, textarea, [role='checkbox'], [role='switch'], [role='menuitem'], [role='button'], [data-no-account-open]";

/** 点击事件里本判据需要的最小形状（不绑 React 类型，便于单测与复用）。 */
export interface AccountOpenClickLike {
  readonly currentTarget: Element | null;
  readonly target: EventTarget | null;
}

/**
 * 这一次点击是否应当打开账号详情抽屉。
 *
 * @param event 点击事件（或等价的 `{ currentTarget, target }`）
 * @param extraIgnoreSelector 调用方额外的豁免标记（如表格的动作格 `[data-row-actions]`）。
 *   该标记只作用于**调用方自己的**那片区域，因此不进 {@link ACCOUNT_OPEN_IGNORE_SELECTOR}。
 * @returns `true` = 点的是卡片本体，可以开详情；`false` = 点的是卡内控件或 Portal 弹层，**不要**开。
 */
export function shouldOpenAccountDetail(
  event: AccountOpenClickLike,
  extraIgnoreSelector?: string,
): boolean {
  const { currentTarget, target } = event;
  if (!currentTarget || !(target instanceof Element)) return false;
  /* ★ 第 1 条是这次报障的根治：Portal 内容在 DOM 上不属于卡片，
     但 React 事件仍会冒泡回来 —— 「不在子树内」是唯一稳定的判据。 */
  if (!currentTarget.contains(target)) return false;
  if (target.closest(ACCOUNT_OPEN_IGNORE_SELECTOR)) return false;
  if (extraIgnoreSelector && target.closest(extraIgnoreSelector)) return false;
  return true;
}
