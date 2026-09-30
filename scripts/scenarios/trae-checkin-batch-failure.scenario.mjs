// 批量签到「跑完」不等于「成功」：全部失败时必须报失败（2026-09-29）。
//
// ## 背景
//
// 与切换账号那条是**同一个毛病**：动作外壳原先只认「Promise 有没有 reject」，
// 而后端批量签到的失败是**正常返回的报告**（`{"total":2,"failed":2}`）⇒
// 界面照样弹绿勾「签到并刷新积分完成」。WorkBuddy 侧早就按失败数分流
// （`AccountsPage.onRefreshCredits`：全失败 → `toast.error`），Trae 侧漏了。
//
// ## 证伪方式（本场景必须能红）
//
// 把 `checkinAll` 的判定改回「一律 `level: "success"`」（等价于旧行为）⇒
// 第 2、3 条断言变红。
//
// ## 前置：需要「两个账号都会签到失败」的现场
//
// 用**副本** home 构造（不要动真库）：
//   ① 清 `checkin_ledger.json` + `credits_daily.json` 当日项 + `credits_history.json`
//      当日项 —— 否则 `checkedToday` 为真、账号被「跳过今日已签到」滤掉，`total` 为 0；
//   ② 把 `checkin_accounts.json` 里 JWT 的**签名尾部**改坏（payload 保持可解析、
//      `exp` 未过期）⇒ 账号仍进队列，但上游必然拒绝；
//   ③ 跑前清空 `account_cooldowns.json` —— 失败会写冷却，第二次跑就会被跳过。
//
// 跑法见 `docs/perf-audit-2026-09-24.md` 记录的三步（后端内嵌 dist + cdp-drive）。

const CHECKIN_ALL = `document.querySelector('button[aria-label="签到并刷新全部账号积分"]')`;

export default async function (ctx) {
  await ctx.waitFor("账号卡片渲染完成", `document.querySelectorAll("article").length >= 2`, 30000);

  const clickable = await ctx.evaluate(`(() => { const b = ${CHECKIN_ALL}; return !!b && !b.disabled; })()`);
  ctx.check("工具栏的「签到并刷新全部账号积分」可点击", clickable === true);

  await ctx.press(CHECKIN_ALL);
  await ctx.waitFor("出现签到结果提示", `document.querySelectorAll("[data-sonner-toast]").length > 0`, 60000);

  // 断 `data-type` 而不是文案：标题与描述在 `textContent` 里是连在一起的，
  // 只断「含『失败』」会被描述里的「N 个失败」蒙混过去（实测确实混过去了）。
  const toasts = JSON.parse(
    await ctx.evaluate(
      `JSON.stringify([...document.querySelectorAll("[data-sonner-toast]")].map((el) => ({ type: el.getAttribute("data-type"), text: el.textContent })))`,
    ),
  );
  const joined = toasts.map((item) => `${item.type}: ${item.text}`).join(" ｜ ");

  // ★ 核心：全部失败时提示类型必须是 `error`，且**不能**同时冒出 success。
  ctx.check("全部失败 ⇒ 提示类型为 error", toasts.some((item) => item.type === "error"), joined);
  ctx.check("全部失败 ⇒ 没有 success 类型的提示", !toasts.some((item) => item.type === "success"), joined);
  // 描述里必须给出计数，否则用户只知道「失败」而不知道几个。
  ctx.check("描述里带上失败账号数", toasts.some((item) => /\d+\s*个失败/.test(item.text)), joined);

  await ctx.screenshot("trae-checkin-all-failed.png");
}
