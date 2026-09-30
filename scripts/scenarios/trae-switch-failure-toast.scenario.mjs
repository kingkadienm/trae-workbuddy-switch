// Trae「切换账号」失败时必须**响亮失败**，不能报「完成」（2026-09-29 用户报障）。
//
// ## 报障原文
//
// 「TRAEWORK 和 traecode 切换成功，但是程序没有被打开，标记状态也没有变更」。
// 真机复现：目标账号在该程序位上**没有登录态快照**，后端 `switch_account` 在预检查
// 就 return，返回 `success:false` +「账号 … 的登录态快照不存在…」—— 客户端当然没启动、
// 当前账号标记当然没变。而界面弹的是绿勾「切换TraeCode账号完成」。
//
// ## 为什么这条只能在这里守
//
// 前端**没有单测框架**（见 `scripts/check-store-selectors.cjs` 的说明），而这条缺陷
// 的性质是「两条 toast 同时弹出、用户只看见成功那条」—— 静态扫描扫不出来。
// 唯一可信的证据是真的把浏览器驱动起来点一遍。
//
// ## 证伪方式（本场景必须能红）
//
// 把 `src/lib/api.ts` 的 `traeSwitchAccount` 改回「直接 return outcome（失败不抛错）」，
// 则 `run()` 会照旧无条件弹「{label}完成」⇒ 第 3 条断言变红。
//
// ## 前置（本场景不是自包含的）
//
// 需要一个**真的会失败**的切换：mock 后端或真实后端都行，只要
// `POST /api/trae/switch` 回 `{"success": false, ...}`。
//
// ⚠️ 这里刻意选「Jackey 在 TraeWork 上」这一格，而**不是** TraeCode：
// TraeCode 在本机没有客户端数据目录，它的按钮已被禁用（那是
// `trae-program-availability.scenario.mjs` 负责的另一条契约），点不动。
// 而「有数据目录、但这个账号没存过快照」是**仍然可达**的失败形态 ——
// 正是这条路径才验证得了「失败要响亮」。

export default async function (ctx) {
  await ctx.waitFor("Trae 账号页渲染出账号卡片", `document.querySelectorAll("article").length >= 2`, 30000);

  // Jackey 卡片上的 TraeWork 切换按钮（`trae.comp.card.switch.aria` = 「切换到 {label}」）。
  const target = `(() => {
    const card = [...document.querySelectorAll("article")].find((el) => el.textContent.includes("Jackey"));
    if (!card) return null;
    return card.querySelector('button[aria-label="切换到 TraeWork"]');
  })()`;

  ctx.check("找到 Jackey 卡片上的 TraeWork 切换按钮", (await ctx.evaluate(`!!${target}`)) === true);

  const clickable = await ctx.evaluate(`(() => { const b = ${target}; return !!b && !b.disabled; })()`);
  ctx.check("该按钮可点击（不是禁用态）", clickable === true);

  await ctx.press(target);
  await ctx.waitFor("出现结果提示", `document.querySelectorAll("[data-sonner-toast]").length > 0`, 20000);

  const toasts = JSON.parse(
    await ctx.evaluate(
      `JSON.stringify([...document.querySelectorAll("[data-sonner-toast]")].map((el) => el.textContent))`,
    ),
  );
  const joined = toasts.join(" ｜ ");

  ctx.check("弹出的是失败提示", toasts.some((text) => text.includes("失败")), joined);
  // ★ 核心断言：绿勾「…完成」**不得**出现。改回「失败也 resolve」即变红。
  ctx.check(
    "没有出现「切换…完成」的成功提示",
    !toasts.some((text) => text.includes("完成")),
    joined,
  );
  // 失败提示必须带上原因，否则用户仍不知道下一步做什么。
  ctx.check(
    "失败提示带上了后端给出的原因",
    toasts.some((text) => text.includes("快照") || text.includes("无法保存")),
    joined,
  );

  await ctx.screenshot("trae-switch-failure.png");
}
