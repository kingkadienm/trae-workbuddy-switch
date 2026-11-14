/* 限流探测原型的**实测**（真跑一遍交互，不靠「看起来对」）。
 *
 * ## 为什么用 Chrome/CDP 而不是 jsdom
 *
 * 本仓其余原型门禁走 jsdom，但 jsdom 装在**隔离工作区**、靠 `NODE_PATH` 找；本机当前不可用。
 * Chrome 本机就有 ⇒ 改走 CDP，反而更接近「用户真的在点」。
 *
 * ## 被测的形态（对齐 `MODEL-LIMIT-PROBE-PLAN.md` §5）
 *
 *   **模型分区里的一张 `.card#probe-card`**（不是模态弹层）——
 *   折叠（一行）⇄ 展开，展开后在卡内切「选择 / 进行中 / 结果」三态。
 *   六条语义必须被钉住，否则原型会演示一个假交互：
 *     ① ★ **不是弹层**：卡片不在 `.overlay` 里（第一版做成弹层被否）；
 *     ② 只有「模型限流／账号限流／模型封禁」算**限流**，402 积分不足分列；
 *     ③ **默认不替用户选模型**（空选 + 按钮禁用 + 明确提示），账号默认全测；
 *     ④ **回填只认本次实测的格**（不得把被动底色再抄一遍）；
 *     ⑤ **`ok` 要清掉该模型已有的冷却**；冷却**按模型记**（徽标点名模型）；
 *     ⑥ 换作用域后探测结果必须清空。
 *
 * 用法：node docs/redesign/verify-model-probe.mjs
 * 产出：logs/model-probe/*.png + 断言输出；任一条失败 ⇒ 非零退出。
 */
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..", "..");
const FILE = join(HERE, "buddy-switch-prototype.html");
const OUT = join(ROOT, "logs", "model-probe");
const PORT = Number(process.env.BS_PROBE_PORT || 9566);

/* Chrome 位置按平台兜底 —— 换机器时只需改这里（或设 BS_CHROME）。 */
const CHROME =
  process.env.BS_CHROME ||
  [
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
    "/usr/bin/google-chrome",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  ].find((p) => existsSync(p));

if (!CHROME) {
  console.error("找不到 Chrome：设 `BS_CHROME` 指向可执行文件后重试。");
  process.exit(2);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok, detail });
  console.log(`${ok ? "  ok  " : " FAIL "} ${name}${detail ? "  — " + detail : ""}`);
};

mkdirSync(OUT, { recursive: true });
const profile = mkdtempSync(join(tmpdir(), "bs-probe-"));
const chrome = spawn(CHROME, [
  "--headless=new", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--disable-gpu",
  "--window-size=1440,1000", "--hide-scrollbars", "about:blank",
], { stdio: "ignore" });

async function targets() {
  for (let i = 0; i < 60; i++) {
    try { return await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json(); } catch { await sleep(250); }
  }
  throw new Error("Chrome CDP 未就绪");
}

const page = (await targets()).find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));

let seq = 0;
const pending = new Map();
const runtimeErrors = [];
ws.onmessage = (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); return; }
  if (msg.method === "Runtime.exceptionThrown") runtimeErrors.push("exception: " + (msg.params?.exceptionDetails?.exception?.description || msg.params?.exceptionDetails?.text || "?"));
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") runtimeErrors.push("console.error: " + (msg.params.args || []).map((a) => a.value ?? a.description ?? "").join(" "));
};
const send = (method, params = {}) => new Promise((res, rej) => {
  const id = ++seq;
  pending.set(id, (m) => (m.error ? rej(new Error(method + ": " + m.error.message)) : res(m.result)));
  ws.send(JSON.stringify({ id, method, params }));
});

const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error("evaluate 抛错: " + (r.exceptionDetails.exception?.description || r.exceptionDetails.text));
  return r.result.value;
};
const shot = async (name) => {
  /* 先清掉右下角的 toast 堆（会盖住模型表右半边 —— 与本次交互无关，只是挡截图）。 */
  await evaluate(`{ const t = document.querySelector("#toasts"); if (t) t.innerHTML = ""; }`);
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(OUT, name), Buffer.from(r.data, "base64"));
  console.log(`       截图 → ${name}`);
};
const ctxt = () => evaluate(`document.querySelector("#probe-card").textContent`);
/* 直接设好选中集再点「开始探测」，并等它跑完。 */
const runProbe = async (accounts, models) => {
  await evaluate(`{ state.probe.expanded = true; state.probe.phase = "setup"; state.probe.accountsSel = new Set(${JSON.stringify(accounts)}); state.probe.modelsSel = new Set(${JSON.stringify(models)}); renderProbeCard(); }`);
  await sleep(120);
  await evaluate(`document.querySelector('#probe-card [data-act="probe-start"]').click()`);
  for (let i = 0; i < 150; i++) { if (await evaluate(`state.probe.phase === "done"`)) break; await sleep(120); }
};

try {
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Page.navigate", { url: "file:///" + FILE.replace(/\\/g, "/") });
  for (let i = 0; i < 60; i++) { if ((await evaluate("document.readyState")) === "complete") break; await sleep(200); }
  await sleep(500);
  await evaluate(`{ const h = document.querySelector("#hint-close"); if (h) h.click(); }`);
  /* 关掉右下角的「会话状态」浮窗 —— 它会盖住卡片底部（与本次交互无关，只是挡截图）。 */
  await evaluate(`{ state.companionOpen = false; renderCompanion(); }`);

  /* ── 入口：卡片就在模型分区里，不在弹层里 ── */
  await evaluate(`{ state.dest = "gateway"; state.scope = "wb-cn"; state.gwSub = "models"; renderAll(); }`);
  await sleep(300);
  check("模型分区里有 `#probe-card`", await evaluate(`!!document.querySelector("#probe-card")`));
  check("★ 卡片**不在**任何 `.overlay` 里（不是弹层 —— 第一版形态被否）",
    await evaluate(`!document.querySelector("#probe-card").closest(".overlay") && !document.querySelector("#probe-overlay")`));
  check("默认**折叠**（只有卡头，没有 padded body）",
    await evaluate(`!!document.querySelector("#probe-card .card-head") && !document.querySelector("#probe-card .card-pad")`));
  /* 回归：卡头的「N 模型可测」必须与复选框网格**同源**（曾出现「卡头 5 个、网格 4 行」，
     因为跨区域同名模型被算了两次）。 */
  check("卡头「N 模型可测」= 网格行数（同源，不重不漏）",
    await evaluate(`(() => { const t = document.querySelector("#probe-card .sub").textContent; const m = t.match(/(\\d+) 模型可测/); return !!m && Number(m[1]) === probeAxisModels(state.scope).length; })()`),
    await evaluate(`document.querySelector("#probe-card .sub").textContent`));
  await shot("01-collapsed.png");

  await evaluate(`document.querySelector('#probe-card [data-act="probe-toggle"]').click()`);
  await sleep(200);
  check("点「展开」后出现 setup 内容", await evaluate(`!!document.querySelector("#probe-card .card-pad")`));

  /* ── ③ 默认不替用户选模型 ── */
  check("★ 默认**不选**任何模型", await evaluate(`state.probe.modelsSel.size === 0`), await evaluate(`"已选=" + state.probe.modelsSel.size`));
  check("默认**全选**账号（要测的就是每个账号）", await evaluate(`state.probe.accountsSel.size === state.probe.accounts.length && state.probe.accounts.length > 0`));
  check("未选模型时「开始探测」禁用", await evaluate(`document.querySelector('#probe-card [data-act="probe-start"]').disabled === true`));
  check("未选模型时给出明确提示", /请先选择要测试的模型/.test(await ctxt()));
  check("步骤编号（1 选模型 / 2 选账号）", /要测哪些模型/.test(await ctxt()) && /在这些账号上测/.test(await ctxt()));
  check("回填文案说明「可用 ⇒ 清掉」", /清掉/.test(await ctxt()) && /置/.test(await ctxt()));
  /* 回归：描述文案里的行内 `<b>` 不能被 `display:block` 打成独立行。 */
  check("回填描述的行内强调没被 block 化（不换行错位）",
    await evaluate(`[...document.querySelectorAll("#probe-card .padopt .t > span b")].length > 0 && [...document.querySelectorAll("#probe-card .padopt .t > span b")].every((el) => getComputedStyle(el).display === "inline")`));
  check("选择控件是**复选框行**（`.pick`），不是 chip 云",
    await evaluate(`document.querySelectorAll("#probe-card .pick").length > 0 && !document.querySelector("#probe-card .chip")`));
  await shot("02-setup-empty.png");

  /* ── 选模型 → 按钮可用、格数正确 ── */
  await evaluate(`document.querySelectorAll('#probe-card [data-act^="probe-model|"]')[0].click()`);
  await sleep(120);
  check("点一行可**选中**（默认不再是全选）", await evaluate(`state.probe.modelsSel.size === 1`), await evaluate(`"已选=" + state.probe.modelsSel.size`));
  check("选中的行有 on 态（有可视反馈）", await evaluate(`document.querySelectorAll('#probe-card .pick.on[data-act^="probe-model|"]').length === 1`));
  await evaluate(`document.querySelectorAll('#probe-card [data-act^="probe-model|"]')[1].click()`);
  await sleep(120);
  const nModels = await evaluate(`state.probe.modelsSel.size`);
  const nAccounts = await evaluate(`state.probe.accountsSel.size`);
  check("选了模型后按钮可用", await evaluate(`document.querySelector('#probe-card [data-act="probe-start"]').disabled === false`));
  check("格数 = 账号 × 已选模型", await evaluate(`new RegExp("= <b>" + (${nAccounts} * ${nModels}) + "</b> 格").test(document.querySelector("#probe-card").innerHTML)`), `expect=${nAccounts * nModels}`);
  await shot("03-setup-selected.png");

  /* ── running + 取消 ── */
  await evaluate(`document.querySelector('#probe-card [data-act="probe-start"]').click()`);
  await sleep(600);
  check("进入 running 态并有进度条", await evaluate(`!!document.querySelector("#probe-card .pbar")`));
  check("进度在推进（done > 0）", await evaluate(`state.probe.done > 0`), await evaluate(`"done=" + state.probe.done + "/" + state.probe.total`));
  check("统计是**一行**（不是一屏 tile）", await evaluate(`!!document.querySelector("#probe-card .pstats") && !document.querySelector("#probe-card .tiles")`));
  await shot("04-running.png");

  await evaluate(`{ const b = document.querySelector('#probe-card [data-act="probe-cancel"]'); if (b) b.click(); }`);
  await sleep(200);
  check("取消后保留部分结果（done 态）", await evaluate(`state.probe.phase === "done" && state.probe.done > 0`));
  check("取消后矩阵仍渲染", await evaluate(`document.querySelectorAll("#probe-card .mx-cell").length > 0`));
  await shot("05-cancelled-matrix.png");

  /* ── ⑤ `ok` 要清冷却 + ④ 被动底色不得被回填 ── */
  await evaluate(`{ for (const r of GATEWAY["wb-cn"].pool) delete r.modelCooling; }`);
  /* 钉死一格为 `ok`，并给它**预先存在的**冷却 —— 探测后必须被清掉。 */
  await evaluate(`{ PROBE_OVERRIDES["wb-cn"]["jackey|gemini-2.5-pro"] = "ok"; GATEWAY["wb-cn"].pool.find((x) => x.account === "jackey").modelCooling = ["gemini-2.5-pro"]; }`);
  await runProbe(["jackey"], ["gemini-2.5-pro"]);
  const adOk = await evaluate(`state.probe.adopted`);
  check("★ 探到 `ok` ⇒ 清掉该模型已有的冷却", adOk && adOk.cleared === 1 && adOk.added === 0, `adopted=${JSON.stringify(adOk)}`);
  check("清空后该账号不再有冷却标记", await evaluate(`!GATEWAY["wb-cn"].pool.find((x) => x.account === "jackey").modelCooling`));
  check("★ 被动底色不得被回填（qa-bot 的日志 429 没被抄进池）", await evaluate(`!(GATEWAY["wb-cn"].pool.find((x) => x.account === "qa-bot") || {}).modelCooling`));

  /* ── 回填「置」：命中数 = 本次实测里「模型限流/封禁」的格数 ── */
  await evaluate(`{ for (const r of GATEWAY["wb-cn"].pool) delete r.modelCooling; }`);
  const accs = await evaluate(`probePlan("wb-cn").accounts.slice(0, 2)`);
  const mods = await evaluate(`probePlan("wb-cn").models.slice(0, 3)`);
  const expectAdd = await evaluate(`{ const ov = PROBE_OVERRIDES["wb-cn"] || {}, plan = probePlan("wb-cn"); let n = 0; for (const a of ${JSON.stringify(accs)}) for (const m of ${JSON.stringify(mods)}) { const st = ov[a + "|" + m] || ov[a + "|*"] || plan.cells[a][m]; if (["rate_limited_model", "model_blocked"].includes(st)) n++; } n; }`);
  await runProbe(accs, mods);
  const adAdd = await evaluate(`state.probe.adopted`);
  check("回填写入数 = 本次实测「模型限流/封禁」格数", adAdd && adAdd.added === expectAdd, `adopted=${JSON.stringify(adAdd)} expect=${expectAdd}`);
  check("矩阵有「限流数」汇总列", await evaluate(`document.querySelectorAll("#probe-card .mx-sum").length > 0`));
  check("图例色块真的有底色（不是空标签）",
    await evaluate(`[...document.querySelectorAll("#probe-card .legend .lg i")].every((el) => getComputedStyle(el).backgroundColor !== "rgba(0, 0, 0, 0)")`));
  check("结论区把「本次实测」与「被动已知」分开报（不混计）", /本次实测命中/.test(await ctxt()));
  await shot("06-matrix-done.png");

  /* ── ★ 探测结果必须**折回模型列表**（模型表加一列状态） ── */
  check("模型表有「限流（探测）」列",
    await evaluate(`[...document.querySelectorAll(".tbl thead th")].some((th) => th.textContent.includes("限流（探测）"))`));
  const cellTxt = await evaluate(`(() => { const row = [...document.querySelectorAll(".tbl tbody tr")].find((r) => /全部可用|限流|被动/.test(r.textContent)); return row ? row.cells[4].textContent.trim() : ""; })()`);
  check("探测过的模型在表里显示状态（全部可用 / N/M 限流 / 被动 N）", /全部可用|\d+\/\d+ 限流|被动 \d+/.test(cellTxt), `cell=${cellTxt}`);
  check("表里的状态是可点的（能下钻到矩阵）", await evaluate(`!!document.querySelector(".tbl .badge.link")`));

  await evaluate(`document.querySelector(".tbl .badge.link").click()`);
  await sleep(300);
  check("★ 点表里的徽章 → 卡片展开、且**只留这一个模型**（下钻）",
    await evaluate(`state.probe.expanded === true && state.probe.modelsSel.size === 1`),
    await evaluate(`"modelsSel=" + [...state.probe.modelsSel].join(",")`));
  const focused = await evaluate(`[...state.probe.modelsSel][0]`);
  check("下钻后矩阵只列出该模型",
    await evaluate(`[...document.querySelectorAll("#probe-card .mx-colhead")].every((th) => th.getAttribute("title") === ${JSON.stringify(focused)})`),
    `focused=${focused}`);
  check("★ 下钻后「共 N 格」随当前选择收缩（不留上次的总数）",
    await evaluate(`/共 ${2} 格/.test(document.querySelector("#probe-card").textContent)`),
    await ctxt().then((t) => (t.match(/共 \d+ 格/) || [""])[0]));
  await shot("07-model-list-status.png");

  /* ── 冷却**按模型**记录且可见 ── */
  await evaluate(`{ state.gwSub = "pool"; renderContent(); }`);
  await sleep(200);
  const badge = await evaluate(`(document.querySelector(".tbl .badge.warn") || {}).textContent || ""`);
  check("账号池出现冷却徽标（回填可见）", !!badge, badge);
  check("★ 徽标点名**具体模型**（不是只有一个数字）",
    await evaluate(`[...document.querySelectorAll(".tbl .badge.warn")].some((el) => el.textContent.trim().length > 3 && /[A-Za-z]/.test(el.textContent))`));
  check("账号池说明段解释了「按模型记」", /按模型记/.test(await evaluate(`document.querySelector(".split-main").textContent`)));
  await shot("08-pool-badge.png");

  /* ── ⑥ 换作用域必须清空 ── */
  await evaluate(`switchScope("qoder-cn")`);
  await sleep(600);
  check("★ 切作用域后探测结果被清空",
    await evaluate(`Object.keys(state.probe.cells).length === 0 && state.probe.resultAt === null`));
  await evaluate(`{ state.dest = "gateway"; state.gwSub = "models"; renderAll(); }`);
  await sleep(300);
  check("切作用域后卡片回到折叠", await evaluate(`state.probe.expanded === false`));
  await evaluate(`document.querySelector('#probe-card [data-act="probe-toggle"]').click()`);
  await sleep(200);
  check("Qoder 作用域同样可探测（三平台同形）",
    await evaluate(`state.probe.phase === "setup" && state.probe.models.length > 0`),
    await evaluate(`"models=" + state.probe.models.join(",")`));
  check("Qoder 下同样默认不选模型", await evaluate(`state.probe.modelsSel.size === 0`));
  await shot("09-qoder-setup.png");

  /* ── 亮色主题 ── */
  await evaluate(`{ state.scope = "wb-cn"; state.dest = "gateway"; state.gwSub = "models"; renderAll(); }`);
  await evaluate(`document.documentElement.classList.remove("dark")`);
  await sleep(250);
  await shot("10-light-collapsed.png");

  check("无 JS 运行时异常", runtimeErrors.length === 0, runtimeErrors.slice(0, 3).join(" | "));
} finally {
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 通过${failed.length ? "（失败：" + failed.map((f) => f.name).join(" / ") + "）" : ""}`);
  try { ws.close(); } catch { /* 忽略 */ }
  chrome.kill();
}
process.exit(results.filter((r) => !r.ok).length ? 1 : 0);
