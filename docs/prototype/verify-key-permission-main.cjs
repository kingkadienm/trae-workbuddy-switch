/* 密钥权限（issue #76）在**主原型**（`buddy-switch-prototype.html`）里的行为验收。
 *
 * 立场与其他 verify-*.cjs 一致：**不信任自述** —— 直接开页面、点真实 DOM、读真实文本。
 * 断言的是「用户能看见的东西」：表头、用量条、chip 文案、抽屉内容、保存后的行内变化。
 *
 * 用法：
 *   NODE_PATH=<managed workspace>/node_modules node docs/prototype/verify-key-permission-main.cjs
 *   ... --mutate=nosaveguard|noscope   注入变异，**期望对应断言变红**。
 */
const path = require("path");
const fs = require("fs");
const os = require("os");
const { JSDOM } = require("jsdom");

const SRC = __dirname;
const FILE = "buddy-switch-prototype.html";
const MUT = (process.argv.find((a) => a.startsWith("--mutate")) || "").replace("--mutate", "").replace(/^=/, "");
const results = [];
let dom = null;
let doc = null;

/* 每个变异声明**它该打红哪几条断言**（`expectRed`）：
 * 期望变红却仍绿 ⇒ 失败；**全部**断言都红 ⇒ 也失败（变异把页面打瘫了，
 * 测的就不是目标行为）。 */
const MUTATIONS = {
  nosaveguard: {
    desc: "去掉「指定账号 0 个时禁止保存」的守卫",
    from: 'const canSave = !(d.scope.mode === "selected" && d.scope.accounts.length === 0) && !(d.quota.mode === "limited" && d.quota.limit <= 0);',
    to: 'const canSave = true; /* MUTATED: 去掉守卫 */',
    expectRed: ["清空后保存被禁用"],
  },
  noscope: {
    desc: "不渲染账号勾选列表（选号入口消失）",
    from: "function keyAcctListHtml() {",
    to: 'function keyAcctListHtml() { return ""; /* MUTATED */',
    expectRed: ["切到「指定账号」", "勾选一个账号"],
  },
  nochip: {
    desc: "权限列不渲染 chip（账号范围无从看起，也失去编辑入口）",
    from: '  return p.scope.mode === "all"\n    ? `<button class="kchip" data-act="key-perm|${k.id}|scope"><i class="dot"></i>账号 全部</button>`\n    : `<button class="kchip ${n ? "on" : "danger"}" data-act="key-perm|${k.id}|scope"><i class="dot"></i>账号 ${n} 个</button>`;',
    to: '  return ""; /* MUTATED: 权限列不渲染 chip */',
    expectRed: ["权限列只一枚 chip"],
  },
};

function check(name, fn) {
  try {
    fn();
    results.push({ name, ok: true });
  } catch (error) {
    results.push({ name, ok: false, error: error.message });
  }
}
function assert(cond, message) { if (!cond) throw new Error(message); }
function text(selector) {
  const el = doc.querySelector(selector);
  return el ? el.textContent.replace(/\s+/g, " ").trim() : "";
}
function click(selector) {
  const el = doc.querySelector(selector);
  assert(el, `找不到元素：${selector}`);
  el.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
}
function clickEl(el) {
  assert(el, "元素不存在");
  el.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
}
function clickAll(selector) {
  doc.querySelectorAll(selector).forEach((el) =>
    el.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })));
}
function setInput(selector, value) {
  const el = doc.querySelector(selector);
  assert(el, `找不到输入框：${selector}`);
  el.value = String(value);
  el.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
}

function load() {
  let html = fs.readFileSync(path.join(SRC, FILE), "utf8");
  if (MUT) {
    const m = MUTATIONS[MUT];
    assert(m, `未知变异：${MUT}`);
    assert(html.includes(m.from), `变异锚点没命中：${m.from.slice(0, 60)}`);
    html = html.replace(m.from, m.to);
  }
  dom = new JSDOM(html, { runScripts: "dangerously", pretendToBeVisual: true, url: "http://localhost/" });
  doc = dom.window.document;
  /* 走真实导航：左栏「网关」→ 子导航「密钥」。 */
  click('[data-act="nav|gateway"]');
  click('[data-act="gw-sub|keys"]');
}

/* ★ 必须在**断言之前**装载页面：断言是立即执行的（`check(...)` 收到函数就调用）。 */
load();

/* ───────────────────────── 断言 ───────────────────────── */

check("密钥表多了「用量」与「权限」两列，且表头没多出横向滚动", () => {
  const heads = [...doc.querySelectorAll("#content table thead th")].map((th) => th.textContent.trim());
  assert(heads.includes("用量"), `表头应有「用量」，实际 ${JSON.stringify(heads)}`);
  assert(heads.includes("权限"), `表头应有「权限」，实际 ${JSON.stringify(heads)}`);
  assert(heads.length === 7, `表头列数应仍是 7（把「最后使用」并进名称次行），实际 ${heads.length}`);
});

check("名称次行带了「创建于」与「最后使用」（原来那一列被并进来了）", () => {
  const cell = doc.querySelector("#content table tbody tr td");
  const sub = cell.querySelector(".tiny");
  assert(sub, "名称次行缺失");
  assert(sub.textContent.includes("创建于") && sub.textContent.includes("最后使用"),
    `次行应同时有创建于与最后使用：${sub.textContent}`);
});

check("用量列压成一行：已用/上限 + 3px 迷你条（四层文字不再塞一列）", () => {
  const rows = [...doc.querySelectorAll("#content table tbody tr")];
  const cell = rows[0].querySelector(".kusage");
  assert(cell, "第一行应有 .kusage 一览单元");
  const bar = cell.querySelector(".kbar");
  assert(bar, "一览单元里应有迷你进度条");
  const fill = bar.querySelector("i");
  assert(fill && fill.style.width === "62%", `进度条应为 62%，实际 ${fill && fill.style.width}`);
  /* ★ 一览只承载「已用/上限」：单位、周期、剩余**不在这一列**（它们进行内展开区）。
     断言写法是「列内不得出现」—— 否则「重新设计」会被悄悄改回去。 */
  assert(cell.textContent.includes("620万"), `应显示已用 620万：${cell.textContent}`);
  assert(cell.textContent.includes("1000万"), `应显示上限 1000万：${cell.textContent}`);
  assert(!cell.textContent.includes("自然月"), "周期不该出现在一览列（应进行内展开区）");
  assert(!cell.textContent.includes("按 token 计"), "单位不该出现在一览列");
});

check("「不限」的密钥不画进度条，但保留累计请求数", () => {
  const rows = [...doc.querySelectorAll("#content table tbody tr")];
  const unlimited = rows.find((tr) => tr.querySelector(".kusage") && tr.textContent.includes("不限"));
  assert(unlimited, "应有「不限」的行");
  assert(!unlimited.querySelector(".kbar"), "不限额不应有进度条");
  assert(unlimited.textContent.includes("累计"), "不限额行仍应显示累计请求数");
});

check("用尽与接近上限分别给出状态徽标", () => {
  const body = doc.querySelector("#content table tbody").textContent;
  assert(body.includes("已用尽"), "缺「已用尽」徽标");
  assert(body.includes("接近上限"), "缺「接近上限」徽标");
});

check("权限列只一枚 chip（账号范围）—— 额度已由用量列的上限体现，不重复占宽", () => {
  const row = doc.querySelector("#content table tbody tr");
  const chips = [...row.querySelectorAll(".kchip")];
  assert(chips.length === 1, `权限列应只有 1 枚 chip，实际 ${chips.length}`);
  assert(chips[0].dataset.act.startsWith("key-perm|"), "chip 必须可点（带 data-act=key-perm|…）");
  assert(chips[0].textContent.includes("账号"), `chip 应是账号范围：${chips[0].textContent}`);
});

check("点行展开明细：单位/周期/剩余都在展开区里（一览让出去的信息在这儿回收）", () => {
  const row = doc.querySelector("#content table tbody tr");
  row.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  const detail = doc.querySelector(".krow-detail");
  assert(detail, "点行应展开明细区");
  const body = detail.textContent;
  assert(body.includes("用量配额"), "展开区应有「用量配额」行");
  assert(body.includes("可访问账号"), "展开区应有「可访问账号」行");
  assert(body.includes("自然月"), `展开区应写明周期：${body}`);
  assert(body.includes("剩余"), `展开区应写明剩余：${body}`);
  assert(body.includes("编辑配额") && body.includes("编辑范围"), "两行各应带编辑快捷钮");
  /* 再点一次同一行 ⇒ 收起。 */
  doc.querySelector("#content table tbody tr").dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  assert(!doc.querySelector(".krow-detail"), "再点一次应收起");
});

check("点操作列的按钮不展开行（点「吊销」不该顺手展开明细）", () => {
  const revoke = doc.querySelector('#content table tbody tr [data-act^="revoke|"]');
  assert(revoke, "应有吊销按钮");
  revoke.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
  assert(!doc.querySelector(".krow-detail"), "点操作按钮不该展开明细");
  assert(doc.querySelector('#content table tbody tr [data-act^="revoke|"]'), "页面结构应保持");
});

check("点 chip 打开权限抽屉：两个分区都在", () => {
  click('[data-act^="key-perm|"]');
  const drawer = doc.querySelector("#drawer");
  assert(drawer.classList.contains("open"), "抽屉未打开");
  assert(text("#drawer-title").includes("权限设置"), `标题应是「权限设置 · …」，实际 ${text("#drawer-title")}`);
  const body = text("#drawer-body");
  assert(body.includes("用量配额"), "抽屉缺「用量配额」分区");
  assert(body.includes("可访问账号"), "抽屉缺「可访问账号」分区");
});

check("抽屉里能切「限额」，并出现单位 / 上限 / 周期三个控件", () => {
  click('[data-act="key-perm-q|limited"]');
  const body = doc.querySelector("#drawer-body");
  assert(body.textContent.includes("计量单位"), "缺计量单位");
  assert(body.querySelector("#key-limit"), "缺上限输入框");
  assert(body.querySelector("#key-limit-mul"), "缺单位下拉");
  assert(body.textContent.includes("重置周期"), "缺重置周期");
  const periods = [...body.querySelectorAll('[data-act^="key-perm-period|"]')].map((b) => b.dataset.act);
  assert(periods.length === 4, `周期应有 4 档，实际 ${periods.length}`);
});

check("计量单位切「请求数」⇒ 预览与说明都改成请求数", () => {
  click('[data-act="key-perm-unit|request"]');
  assert(text("#drawer-body").includes("业务代理"), "单位说明应解释何时用请求数");
  assert(text("#key-policy-preview").includes("请求数"), `预览应写请求数：${text("#key-policy-preview")}`);
});

check("重置周期切「自然月」⇒ 预览跟着变", () => {
  click('[data-act="key-perm-period|monthly"]');
  assert(text("#key-policy-preview").includes("自然月"), `预览应写自然月：${text("#key-policy-preview")}`);
});

check("切到「指定账号」⇒ 出现账号列表与分组 chip", () => {
  click('[data-act="key-perm-scope|selected"]');
  const body = doc.querySelector("#drawer-body");
  assert(body.querySelectorAll("#key-acct-list .kacct").length > 0, "缺账号勾选列表");
  assert(body.querySelectorAll('[data-act^="key-perm-group|"]').length > 0, "缺分组 chip");
});

check("勾选一个账号 ⇒ 已选计数与预览同步", () => {
  const first = doc.querySelector('#key-acct-list [data-act^="key-perm-acct|"]');
  const id = first.dataset.act.split("|")[1];
  click('#key-acct-list [data-act="key-perm-acct|' + id + '"]');
  assert(text("#drawer-body").includes("已选 1"), `已选计数未更新：${text("#drawer-body")}`);
  assert(text("#key-policy-preview").includes("1 个指定账号"), `预览未同步：${text("#key-policy-preview")}`);
});

check("分组 chip 能整组选取（组内成员全部进白名单）", () => {
  /* ★ 先清空：让本用例**自包含**（上一条用例勾过一个人，
     靠「前置状态」断言会让这条用例随用例顺序漂）。 */
  click('[data-act="key-perm-acct|none"]');
  const before = text("#drawer-body");
  assert(before.includes("还没有选择账号"), "前置：清空后应一个都没选");
  /* ★ 清空会**重画整个抽屉** ⇒ chip 必须在清空**之后**再取（旧的已脱离文档，
     点它不会有任何反应 —— 这正是本用例一开始失败的原因）。 */
  const chip = doc.querySelector('[data-act^="key-perm-group|"]');
  const group = decodeURIComponent(chip.dataset.act.split("|")[1]);
  clickEl(chip);
  const after = text("#drawer-body");
  assert(!after.includes("还没有选择账号"), "整组选取后不该是空选");
  const groupRow = [...doc.querySelectorAll('[data-act^="key-perm-group|"]')]
    .find((b) => decodeURIComponent(b.dataset.act.split("|")[1]) === group);
  assert(groupRow.classList.contains("on"), "整组选取后该分组 chip 应进入选中态");
  /* chip 上的计数形如「2/2」：全选时分子必须等于分母（不是「1/2」）。 */
  const m = groupRow.textContent.match(/(\d+)\/(\d+)/);
  assert(m, `组 chip 上应有「已选/组内总数」：${groupRow.textContent}`);
  assert(m[1] === m[2], `整组选取后计数应满（${m[1]}/${m[2]}）`);
  /* 已选人数也应等于组内人数。 */
  const picked = text("#drawer-body").match(/已选\s*(\d+)\s*个/);
  assert(picked && Number(picked[1]) === Number(m[2]),
    `已选人数应等于组内人数：${text("#drawer-body")}`);
});

check("清空后保存被禁用，并给出原因", () => {
  click('[data-act="key-perm-acct|none"]');
  const save = doc.querySelector('[data-act="key-perm-save"]');
  assert(save.disabled, "一个都没选时保存必须禁用");
  assert(text("#drawer-body").includes("至少要选一个账号"), "缺「至少要选一个账号」的提示");
});

check("保存后抽屉关闭、表格里的 chip 与用量同步更新", () => {
  /* ★ 靶子**按名称定位**（不用 last-child）：新增钥匙会让末行漂移，
     而上一条「取消」用例已经在改第一行的草稿了 —— 用「外包同学」这把互不干扰。 */
  const target = () => [...doc.querySelectorAll("#content table tbody tr")]
    .find((tr) => tr.textContent.includes("外包同学"));
  const before = target();
  assert(before, "应有「外包同学」这把密钥");
  clickEl(before.querySelector('[data-act^="key-perm|"]'));
  click('[data-act="key-perm-scope|selected"]');
  click('[data-act="key-perm-acct|all"]');
  click('[data-act="key-perm-save"]');
  assert(!doc.querySelector("#drawer").classList.contains("open"), "保存后抽屉应关闭");
  const row = target();
  assert(row.textContent.includes("账号 8 个"), `行内账号 chip 应更新为 8 个：${row.textContent}`);
  assert(row.textContent.includes("1000万"), `行内额度 chip 应写 1000万：${row.textContent}`);
});

check("取消 ⇒ 草稿丢弃，表格不变", () => {
  const row = doc.querySelector("#content table tbody tr");
  const before = row.textContent;
  click('[data-act^="key-perm|"]');
  click('[data-act="key-perm-q|unlimited"]');
  click('[data-act="key-perm-cancel"]');
  assert(!doc.querySelector("#drawer").classList.contains("open"), "取消后抽屉应关闭");
  const after = doc.querySelector("#content table tbody tr").textContent;
  assert(after === before, `取消后不应改表格：\n前：${before}\n后：${after}`);
});

/* ───────────────────────── 变异模式 ───────────────────────── */

if (MUT) {
  const spec = MUTATIONS[MUT];
  const red = results.filter((r) => !r.ok).map((r) => r.name);
  console.log(`\n▶ 变异模式：${MUT} —— ${spec.desc}`);
  console.log(`  期望打红：${spec.expectRed.join(" / ")}\n`);
  for (const r of results) console.log(`  ${r.ok ? "· 仍绿" : "✓ 变红"}  ${r.name}`);
  const missed = spec.expectRed.filter((name) => !red.some((r) => r.includes(name)));
  if (missed.length) {
    console.log(`\n❌ 期望变红却仍然绿：${missed.join(" / ")} —— 断言打不红`);
    process.exit(1);
  }
  if (results.every((r) => !r.ok)) {
    console.log("\n❌ 全部断言都红了 —— 变异把页面打瘫了，测的不是目标行为");
    process.exit(1);
  }
  console.log("\n✅ 变异被目标断言打红 —— 断言有效");
  process.exit(0);
}

console.log("");
let failed = 0;
for (const r of results) {
  if (!r.ok) failed += 1;
  console.log(`${r.ok ? "✓" : "✗"} ${r.name}`);
  if (!r.ok) console.log(`    ${r.error}`);
}
console.log(`\n${results.length - failed} passed, ${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
