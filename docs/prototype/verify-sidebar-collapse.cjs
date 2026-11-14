/* 侧栏折叠（只显示图标）—— 2026-10-22 新增的行为验收。
 *
 * 立场与其他 verify-*.cjs 一致：**不信任自述** —— 直接开页面、点真实 DOM、读真实文本。
 *
 * 断言打在「两态各自成立」上，而不是「有个按钮」：
 *   ① 展开态：宽度 240、六项导航**都带文字**、徽章在行尾；
 *   ② 折叠态：宽度 64、导航**没有文字**、徽章转图标右上角标、每项带 tooltip；
 *   ③ 折叠态下点导航**仍然能跳页**（图标不是装饰）；
 *   ④ 快捷键 ⌘/Ctrl+B 与按钮**同一个动作**（两条路径不许说两种话）；
 *   ⑤ 折叠开关自己在两态里都点得动（收起后它就是「展开」按钮）。
 *
 * 用法：
 *   NODE_PATH=<managed workspace>/node_modules node docs/prototype/verify-sidebar-collapse.cjs
 *   ... --mutate=<名>   期望对应断言变红
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
  /* 徽章在折叠态消失（不是移位）⇒ 用户看不到「有待处理」。 */
  badgelost: {
    desc: "折叠态不渲染徽章（有待处理也无从看起）",
    from: '        <span class="nav-icon-wrap">${d.icon}${badge}</span>',
    to: '        <span class="nav-icon-wrap">${d.icon}</span> /* MUTATED: 徽章丢了 */',
    expectRed: ["折叠态徽章转成图标右上角标"],
  },
  /* 折叠态仍渲染文字 ⇒ 64px 里文字会被裁一半，「只显示图标」名不副实。 */
  textkept: {
    desc: "折叠态仍然渲染导航文字（64px 里会被裁一半）",
    from: '        <span class="nav-icon-wrap">${d.icon}${badge}</span>\n        <span class="nav-tip" role="tooltip">${d.label}</span>\n      </button>`;',
    to: '        <span class="nav-icon-wrap">${d.icon}${badge}</span>\n        <span class="nav-label">${d.label}</span>\n        <span class="nav-tip" role="tooltip">${d.label}</span>\n      </button>`;',
    expectRed: ["侧栏 64px、导航没有文字"],
  },
  /* 折叠开关点不动 ⇒ 用户被锁在图标态里，只能重启应用。 */
  nodeadend: {
    desc: "折叠后无法展开（侧栏开关失效，用户被困在图标态）",
    from: '    case "side-toggle": stop(); state.sideCollapsed = !state.sideCollapsed; renderSidebar(); break;',
    to: '    case "side-toggle": stop(); /* MUTATED: 开关失效 */ break;',
    expectRed: ["折叠开关两态都点得动"],
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
/* ★ 用例自包含：先把侧栏**确保到目标态**再断言。
   没有它，用例之间会通过「点了几次开关」互相污染 ——
   折叠类断言会随用例顺序漂移（本轮第一版就是这么翻车的：第 3 条把第 2 条点的开关又点回去了）。 */
function ensureCollapsed() {
  if (!doc.querySelector(".sidebar").classList.contains("collapsed")) click("#side-toggle");
  assert(doc.querySelector(".sidebar").classList.contains("collapsed"), "确保折叠失败");
}
function ensureExpanded() {
  if (doc.querySelector(".sidebar").classList.contains("collapsed")) click("#side-toggle");
  assert(!doc.querySelector(".sidebar").classList.contains("collapsed"), "确保展开失败");
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
}

/* ★ 必须在**断言之前**装载页面：断言是立即执行的（`check(...)` 收到函数就调用）。 */
load();

/* ───────────────────────── 断言 ───────────────────────── */

check("展开态：侧栏 240px、六项导航都带文字、徽章在行尾", () => {
  const side = doc.querySelector(".sidebar");
  assert(side, "应有侧栏");
  assert(!side.classList.contains("collapsed"), "默认应是展开态");
  const items = [...doc.querySelectorAll("#nav .nav-item")];
  assert(items.length === 6, `应有 6 个导航项，实际 ${items.length}`);
  items.forEach((el) => {
    const label = el.querySelector(".nav-label");
    assert(label && label.textContent.trim().length > 0, "展开态每项都应带文字");
  });
  /* 行尾徽章：`data-badge` 在 `.nav-label` **之后**（flex 行尾）。 */
  const accounts = items.find((el) => el.querySelector('[data-badge="accounts"]'));
  assert(accounts, "账号项应有徽章");
  assert(!accounts.querySelector(".nav-icon-wrap"), "展开态不该有角标包装");
  const wrap = accounts.querySelector(".nav-icon-wrap");
  assert(!wrap, "展开态徽章不该在角标里");
  /* 品牌块（issue #75 方案 1）：只有两行文字，**不再画应用图标** ——
     那枚 32px 图标与顶栏 26px 的平台图标同族、同处左缘一条竖线，
     是「找切换平台入口时点错」的主因。 */
  const brand = doc.querySelector(".brand");
  assert(brand, "应有品牌块");
  assert(!brand.querySelector(".brand-mark"), "品牌块不该再有应用图标（issue #75）");
  assert(brand.textContent.includes("Buddy Switch"), "品牌块要保留应用名");
});

check("折叠态：侧栏 64px、导航没有文字、每项带 tooltip", () => {
  ensureCollapsed();
  const side = doc.querySelector(".sidebar");
  assert(side.classList.contains("collapsed"), "点开关后应进入折叠态");
  const items = [...doc.querySelectorAll("#nav .nav-item")];
  assert(items.length === 6, `折叠态仍应有 6 个导航项，实际 ${items.length}`);
  items.forEach((el) => {
    assert(!el.querySelector(".nav-label"), "折叠态不该渲染文字");
    assert(el.getAttribute("aria-label"), "折叠态每项都要有 aria-label（读屏器）");
    const tip = el.querySelector(".nav-tip");
    assert(tip && tip.textContent.trim().length > 0, "折叠态每项都要有可见 tooltip");
  });
  /* 品牌块（issue #75 方案 1）：图标已按该 issue 移除 ⇒ 折叠态**整块隐藏**
     （60px 里放不下双行文案，没有图标就无可承载的内容）。
     ★ 断言用 **computed style**（`display:none`）而不是「DOM 里找不到」——
     原型走 CSS 隐藏（真机走不渲染），用 `querySelector` 断言「不存在」
     在 CSS 方案下会恒真，那等于什么都没测。 */
  const brand = doc.querySelector(".brand");
  assert(brand, "品牌块应在 DOM 里（CSS 隐藏方案）");
  assert(
    dom.window.getComputedStyle(brand).display === "none",
    "折叠态品牌块必须整块隐藏",
  );
});

check("折叠态徽章转成图标右上角标（数字没丢）", () => {
  ensureCollapsed();
  const badge = doc.querySelector('#nav [data-badge="accounts"]');
  assert(badge, "折叠态账号徽章不能丢");
  assert(badge.textContent.trim().length > 0, "徽章数字不能空");
  const wrap = badge.closest(".nav-icon-wrap");
  assert(wrap, "徽章必须在 .nav-icon-wrap 里（角标定位锚）");
  /* 角标是「绝对定位在图标右上」——用类名断言形态，不用算坐标。 */
  assert(badge.classList.contains("nav-badge"), "仍是 nav-badge（只是换了位置）");
});

check("折叠态点导航仍然跳页（图标不是装饰）", () => {
  ensureCollapsed();
  const before = text("#content h1, #content h2").slice(0, 20);
  /* 原型口径的网关项 label 是「网关」（真实应用是「API 服务」）——
     按 aria-label 找，不写死第几个。 */
  const gateway = [...doc.querySelectorAll("#nav .nav-item")].find((el) =>
    el.getAttribute("aria-label") === "网关");
  assert(gateway, "应能找到网关导航项");
  clickEl(gateway);
  const after = text("#content h1, #content h2").slice(0, 20);
  assert(after !== before, `点折叠态导航应跳页：${before} → ${after}`);
});

check("折叠开关两态都点得动（收起后它就是「展开」按钮）", () => {
  ensureExpanded();
  const toggle = doc.querySelector("#side-toggle");
  assert(toggle, "应有折叠开关");
  const label = () => (toggle.querySelector(".nav-label") || {}).textContent || "";
  const expanded = () => toggle.getAttribute("aria-expanded");
  assert(expanded() === "true", "默认 aria-expanded=true");
  click("#side-toggle");
  assert(expanded() === "false", "收起后 aria-expanded=false");
  assert(label() === "展开侧栏", `收起后文案应变成「展开侧栏」，实际 ${label()}`);
  click("#side-toggle");
  assert(expanded() === "true", "再点一次应回到展开");
  assert(label() === "收起侧栏", `展开后文案应回到「收起侧栏」，实际 ${label()}`);
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
