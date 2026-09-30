// Trae 卡片上「程序切换 / 保存登录态」的**可用性必须如实反映能不能做**（2026-09-29）。
//
// ## 背景
//
// 用户报障「切换提示成功但程序没打开、标记也没变」。根因是两件事叠在一起：
// ① 目标账号在那个程序位上没有登录态快照（后端预检查就 return）；
// ② 界面给了一枚**看着能点、点了必然失败**的按钮 —— 它只按「客户端装没装」判定，
//    而 `D:\Programs\Trae CN\Trae CN.exe` 装着、`%APPDATA%\Trae CN` 却不存在
//    （客户端**从未启动过**）⇒ 快照既存不出也恢复不进，这个程序位上的切换是死路。
//
// 本场景把「按钮该不该可点」这件事钉住，并顺带钉住保存请求的**目标程序位**。
//
// ## 证伪方式
//
// - 把 `programsFor` 的 `hasDataDir` 从卡片的 `disabled` 里去掉 ⇒ 第 1 条变红；
// - 把页面的 `onSaveLogin` 改回传页面所属的**区域**（`variant`）⇒ 第 5 条变红
//   （本机 JackDev 登录在 TraeWork，旧实现发的区域标识是 `cn`，不是 `trae_work`）。
//
// ## 前置
//
// 需要真机形态：某个程序位「装了但从未启动过」（无 userData 目录）。
// 本机 CN TraeCode 就是这一格。

const CARD = (name) =>
  `[...document.querySelectorAll("article")].find((el) => el.textContent.includes(${JSON.stringify(name)}))`;

const switchButton = (name, label) =>
  `(() => { const card = ${CARD(name)}; return card ? card.querySelector('button[aria-label="切换到 ${label}"]') : null; })()`;

const menuTrigger = (name) =>
  `(() => { const card = ${CARD(name)}; return card ? card.querySelector('button[aria-label="管理账号 ${name}"]') : null; })()`;

const SAVE_MENU_ITEM = `[...document.querySelectorAll('[role="menuitem"]')].find((el) => el.textContent.trim() === "保存登录态") ?? null`;

export default async function (ctx) {
  await ctx.waitFor("账号卡片渲染完成", `document.querySelectorAll("article").length >= 2`, 30000);

  // ① 装了但从未启动过的程序位（CN TraeCode）必须**禁用** —— 否则就是给假承诺。
  const codeDisabled = await ctx.evaluate(`(() => { const b = ${switchButton("Jackey", "TraeCode")}; return !!b && b.disabled; })()`);
  ctx.check("TraeCode（装了但从未启动过）的切换按钮为禁用态", codeDisabled === true);

  // ② 对照：有数据目录的程序位（CN TraeWork）必须**仍可用** —— 否则等于把功能关掉了。
  const workDisabled = await ctx.evaluate(`(() => { const b = ${switchButton("JackDev", "TraeWork")}; return !!b && b.disabled; })()`);
  ctx.check("TraeWork（有数据目录）的切换按钮仍可用", workDisabled === false);

  // ③ 保存登录态：该账号必须**此刻登录在某个程序上**。Jackey 谁也没登录 ⇒ 禁用。
  await ctx.press(menuTrigger("Jackey"));
  await ctx.waitFor("Jackey 的菜单展开", `!!${SAVE_MENU_ITEM}`);
  const jackeySaveDisabled = await ctx.evaluate(`(() => { const el = ${SAVE_MENU_ITEM}; return !!el && (el.getAttribute("aria-disabled") === "true" || el.hasAttribute("data-disabled")); })()`);
  ctx.check("Jackey（未登录任何客户端）的「保存登录态」为禁用态", jackeySaveDisabled === true);
  await ctx.press(menuTrigger("Jackey"));
  await ctx.sleep(300);

  // ④ 保存请求必须打向「**该账号当前登录的程序位**」，而不是页面所属的区域。
  //    本机 JackDev 登录在 TraeWork ⇒ 请求体里应是 `trae_work`；
  //    旧实现传的是区域标识（`cn`），两者在服务端解析结果相同，但**请求体不同**，
  //    所以这条断言对旧实现是**可证伪**的。
  await ctx.evaluate(`(() => {
    window.__saves = [];
    const original = window.fetch;
    window.fetch = (input, init) => {
      try {
        const url = typeof input === "string" ? input : input.url;
        if (url.includes("/api/trae/login/save") && init && init.body) window.__saves.push(JSON.parse(init.body));
      } catch {}
      return original(input, init);
    };
    return true;
  })()`);

  await ctx.press(menuTrigger("JackDev"));
  await ctx.waitFor("JackDev 的菜单展开", `!!${SAVE_MENU_ITEM}`);
  await ctx.press(SAVE_MENU_ITEM);
  await ctx.waitFor("保存请求已发出", `window.__saves.length > 0`, 20000);

  const saves = JSON.parse(await ctx.evaluate(`JSON.stringify(window.__saves)`));
  ctx.check(
    "保存请求打向 trae_work（该账号当前登录的程序位）",
    saves.length > 0 && saves.every((body) => body.variant === "trae_work"),
    JSON.stringify(saves),
  );

  await ctx.waitFor("出现保存结果提示", `document.querySelectorAll("[data-sonner-toast]").length > 0`, 20000);
  const toasts = JSON.parse(
    await ctx.evaluate(`JSON.stringify([...document.querySelectorAll("[data-sonner-toast]")].map((el) => el.textContent))`),
  );
  ctx.check("保存成功（提示含「完成」而不是「失败」）", toasts.some((text) => text.includes("完成")), toasts.join(" ｜ "));

  await ctx.screenshot("trae-program-availability.png");
}
