/* 联动设计稿的**实测**（真跑一遍交互，不靠「看起来对」）。

## 这份稿子守的是什么

账号页「客户端限流」与网关页「限流探测」必须读**同一份**数据：

1. **同一份**：两侧点「实测」都改这两块；不存在两处各自维护一份。
2. **累积**：后端作业是单槽（新作业从空 `cells` 开始）⇒ 前端按 `(uid, model)` 合并，
   否则「探第二行就把第一行结论冲掉」。
3. **不篡改台账**：台账徽章照旧；实测结论**并列**显示，冲突时写明「实测可用，台账仍受限」。
4. **三档才算限流**：`no_credits`（402）必须显示成「积分不足 · 不是限流」。
5. **入口有代价**：卡头「实测 N 格」只测台账里出现过的组合；关联不到账号/模型的行**置灰**并说明原因。
6. **作业位单例**：探测中重复点击不再起第二个作业。

用法：`node docs/prototype/verify-limit-link.mjs`
产出：`logs/limit-link/*.png`；任一条失败 ⇒ 非零退出。
*/
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..", "..");
const FILE = join(HERE, "limit-link-proto.html");
const OUT = join(ROOT, "logs", "limit-link");
const PORT = Number(process.env.BS_LIMIT_LINK_PORT || 9587);

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
const profile = mkdtempSync(join(tmpdir(), "bs-limit-link-"));
const chrome = spawn(CHROME, [
  "--headless=new", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--disable-gpu",
  "--window-size=1440,1100", "--hide-scrollbars", "about:blank",
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
  if (msg.method === "Runtime.exceptionThrown") {
    runtimeErrors.push("exception: " + (msg.params?.exceptionDetails?.exception?.description
      || msg.params?.exceptionDetails?.text || "?"));
  }
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") {
    runtimeErrors.push("console.error: " + (msg.params.args || []).map((a) => a.value ?? a.description ?? "").join(" "));
  }
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
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(OUT, name), Buffer.from(r.data, "base64"));
  console.log(`       截图 → ${name}`);
};
/** 等某个表达式成真（默认 8s）。 */
const waitFor = async (expression, timeout = 8000) => {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    if (await evaluate(expression)) return true;
    await sleep(80);
  }
  return false;
};
const click = (selector) => evaluate(`{ const el = document.querySelector(${JSON.stringify(selector)});
  if (!el) throw new Error("找不到 " + ${JSON.stringify(selector)}); el.click(); }`);
const text = (selector) => evaluate(
  `((document.querySelector(${JSON.stringify(selector)})?.textContent || "").replace(/\\s+/g, " ").trim())`);
const verdictText = (uid, model) => text(`[data-verdict="${uid}|${model}"]`);
const rowText = (uid, model) => text(`[data-row="${uid}|${model}"]`);
const cellClass = (uid, model) => evaluate(`document.querySelector('[data-act="cell|${uid}|${model}"]')?.className || ""`);

try {
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Page.navigate", { url: "file:///" + FILE.replace(/\\/g, "/") });
  for (let i = 0; i < 60; i++) { if ((await evaluate("document.readyState")) === "complete") break; await sleep(200); }
  await sleep(400);

  /* ── ① 首屏：两块都在，台账与实测列都为空 ── */
  check("两块卡同屏（账号页台账卡 + 网关页探测卡）",
    await evaluate(`!!document.querySelector("#ledger-card") && !!document.querySelector("#probe-card")`));
  check("台账首屏 5 行（含 1 条 join 失败）",
    (await evaluate(`document.querySelectorAll("#ledger-rows .row").length`)) === 5,
    await evaluate(`"行数=" + document.querySelectorAll("#ledger-rows .row").length`));
  check("矩阵 3 账号 × 3 模型 = 9 格",
    (await evaluate(`document.querySelectorAll("#matrix .cell").length`)) === 9);
  check("首屏没有任何「实测 ·」结论（未探测 ≠ 可用）",
    await evaluate(`!document.querySelector("[data-verdict]")`));
  check("未实测的格是虚线（与「可用」视觉可区分）",
    await evaluate(`document.querySelector('[data-act="cell|u-jackey|deepseek-v4.1-flash"]').classList.contains("unknown")`));
  await shot("01-initial.png");

  /* ── ② 入口：卡头只测台账组合；join 失败行置灰并说明 ── */
  /* ★ 格数 = **去重后的 (uid, model) 组合数**：后端 `probe_start` 收 `cells` 显式清单
     （与 `uids × models` 的笛卡尔积互斥）⇒ 3 条台账 = 3 格，不白打另外 3 格。 */
  check("卡头按钮 = 台账里 (仍受限/未知) 的去重组合数（3，不是 3×2=6）",
    (await text("#probe-all span")) === "实测 3 格", await text("#probe-all span"));
  check("卡头按钮可点（有可实测条目）",
    (await evaluate(`document.querySelector("#probe-all").disabled`)) === false);
  check("join 失败行的「实测」按钮置灰（关联不到账号/模型）",
    (await evaluate(`document.querySelector('[data-act="probe-row||"]').disabled`)) === true);
  check("置灰按钮**说明了原因**（title 含「无法实测」）",
    /无法实测/.test(await evaluate(`document.querySelector('[data-act="probe-row||"]').title`)),
    await evaluate(`document.querySelector('[data-act="probe-row||"]').title`));
  check("卡头按钮的 title 写明代价、「只测台账组合」与「不回填」",
    /最小请求/.test(await evaluate(`document.querySelector("#probe-all").title`))
      && /只测台账里出现过的组合/.test(await evaluate(`document.querySelector("#probe-all").title`))
      && /不回填/.test(await evaluate(`document.querySelector("#probe-all").title`)));

  /* ── ③ 行内实测：两侧同时变 ── */
  await click('[data-act="probe-row|u-jackey|glm-5.3"]');
  check("探测中进度条出现（同一作业驱动账号页）",
    await evaluate(`document.querySelector("#ledger-progress").classList.contains("on")`));
  check("探测中该行按钮变「探测中」且不可再点",
    await evaluate(`document.querySelector('[data-act="probe-row|u-jackey|glm-5.3"]').disabled === true`)
      && /探测中/.test(await text('[data-act="probe-row|u-jackey|glm-5.3"]')));
  /* 单例：探测中重复点卡头不会起第二个作业 */
  const totalBefore = await evaluate(`state.job.total`);
  await evaluate(`document.querySelector("#probe-all").click()`);
  check("★ 作业位单例：探测中重复点击不起第二个作业",
    (await evaluate(`state.job.total`)) === totalBefore,
    `total ${totalBefore} → ${await evaluate(`state.job.total`)}`);

  await waitFor(`state.job.phase === "done"`);
  check("左栏该行出现「实测 · 模型限流」",
    /实测 · 模型限流/.test(await verdictText("u-jackey", "glm-5.3")), await verdictText("u-jackey", "glm-5.3"));
  check("右栏同一格**同时**变成模型限流（数据同步）",
    (await cellClass("u-jackey", "glm-5.3")).includes("rate_limited_model"), await cellClass("u-jackey", "glm-5.3"));
  check("网关卡标明「来源：账号页触发 · 未回填」",
    (await evaluate(`document.querySelector("#probe-origin").style.display`)) !== "none"
      && /未回填/.test(await text("#probe-origin")));
  check("网关卡底部说明**同一件事**（只查证不回填）",
    /不回填/.test(await text("#probe-foot")), await text("#probe-foot"));
  await shot("02-row-probed.png");

  /* ── ④ 累积：探第二行不冲掉第一行 ── */
  await click('[data-act="probe-row|u-qa|glm-5.3"]');
  await waitFor(`state.job.phase === "done"`);
  check("★ 跨作业累积：探第二行后第一行结论仍在",
    /实测 · 模型限流/.test(await verdictText("u-jackey", "glm-5.3")), await verdictText("u-jackey", "glm-5.3"));
  check("第二行出现「实测 · 可用」",
    /实测 · 可用/.test(await verdictText("u-qa", "glm-5.3")), await verdictText("u-qa", "glm-5.3"));

  /* ── ⑤ 冲突：并列显示，不篡改台账 ── */
  check("★ 冲突写明「实测可用，台账仍受限」",
    /台账仍受限/.test(await verdictText("u-qa", "glm-5.3")), await verdictText("u-qa", "glm-5.3"));
  check("★ 台账徽章**仍是「仍受限」**（UI 不替用户改结论）",
    /仍受限/.test(await rowText("u-qa", "glm-5.3")), await rowText("u-qa", "glm-5.3"));
  check("矩阵该格是「可用」（实测优先于被动底色）",
    (await cellClass("u-qa", "glm-5.3")).includes("ok"), await cellClass("u-qa", "glm-5.3"));

  /* ── ⑥ 402 不是限流 ── */
  await click('[data-act="probe-row|u-team|claude-sonnet-4.5"]');
  await waitFor(`state.job.phase === "done"`);
  check("402 显示成「实测 · 积分不足」",
    /实测 · 积分不足/.test(await verdictText("u-team", "claude-sonnet-4.5")), await verdictText("u-team", "claude-sonnet-4.5"));
  check("★ 并写明「不是限流」（否则用户去傻等解锁）",
    /不是限流/.test(await verdictText("u-team", "claude-sonnet-4.5")));
  check("积分不足的格**不计入**限流数（矩阵合计列 = 1）",
    (await evaluate(`document.querySelectorAll("#matrix .sum.hot")[0]?.textContent.trim()`)) === "1",
    await evaluate(`document.querySelectorAll("#matrix .sum.hot")[0]?.textContent.trim()`));

  /* ── ⑦ 从矩阵点格 → 详情 → 单格实测（台账外组合 ⇒ 实测独有） ── */
  await click('[data-act="cell|u-team|glm-5.3"]');
  check("点格出现详情（未实测时给「实测这一格」）",
    await evaluate(`document.querySelector("#cell-detail").classList.contains("on")`)
      && /实测这一格/.test(await text("#cell-detail")), await text("#cell-detail"));
  await click('[data-act="probe-cell|u-team|glm-5.3"]');
  await waitFor(`state.job.phase === "done"`);
  check("台账外组合被实测出来后**单列**在「实测独有」，不伪装成台账条目",
    (await evaluate(`document.querySelector("#ledger-extra").style.display`)) === "block"
      && /team-shared/.test(await text("#extra-rows")), await text("#extra-rows"));
  check("台账行数不变（实测不往台账里写记录）",
    (await evaluate(`document.querySelectorAll("#ledger-rows .row").length`)) === 5);
  await shot("03-conflict-and-extra.png");

  /* ── ⑧ 全部实测后，两侧计数一致（同一份数据） ── */
  const sel = "#ledger-rows .row";
  await click("#probe-all");
  check("卡头实测再次触发（job 重置、已探格仍保留）",
    await waitFor(`state.job.phase === "running" || state.job.phase === "done"`, 2000));
  await waitFor(`state.job.phase === "done"`, 12000);
  const matrixHot = await evaluate(`[...document.querySelectorAll("#matrix .cell")]
    .filter((el) => /rate_limited_model|rate_limited_account|model_blocked/.test(el.className)).length`);
  const ledgerVerdicts = await evaluate(`[...document.querySelectorAll("[data-verdict]")]
    .filter((el) => /实测 · (模型限流|账号限流|模型封禁)/.test(el.textContent)).length`);
  const extras = await evaluate(`document.querySelectorAll("#extra-rows .row").length`);
  check("★ 两侧一致：矩阵限流格数 = 台账行实测限流数 + 实测独有",
    matrixHot === ledgerVerdicts + extras, `矩阵 ${matrixHot} / 行 ${ledgerVerdicts} + 独有 ${extras}`);
  check("行数仍为 5（全程没有新增/删除台账条目）",
    (await evaluate(`document.querySelectorAll("${sel}").length`)) === 5);
  await shot("04-all-done.png");

  /* ── ⑨ 取消：保留已探到的格 ── */
  await evaluate(`{ state.job = idleJob(); renderLedger(); renderMatrix(); }`);
  await evaluate(`state.seen = {}; renderLedger(); renderMatrix();`);
  await click("#probe-all");
  await waitFor(`state.job.done >= 1`, 4000);
  await click('[data-act="probe-cancel"]');
  await sleep(200);
  check("取消后进度条收起", await evaluate(`!document.querySelector("#ledger-progress").classList.contains("on")`));
  check("取消后**保留已探到的格**（不白跑）",
    (await evaluate(`Object.keys(state.seen).length`)) >= 1,
    `已探 ${await evaluate(`Object.keys(state.seen).length`)} 格`);
  await shot("05-cancelled.png");

  check("运行期无 JS 异常 / console.error", runtimeErrors.length === 0, runtimeErrors.join(" | "));
} finally {
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 条通过`);
  if (failed.length) {
    console.log("失败：");
    for (const f of failed) console.log(" - " + f.name + (f.detail ? "  — " + f.detail : ""));
  }
  ws.close();
  chrome.kill();
  process.exit(failed.length ? 1 : 0);
}
