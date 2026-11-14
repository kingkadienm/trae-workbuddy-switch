/* 聚合账号面板（issue #50）验收套件 —— 与 verify-demo / verify-behaviors 同一套写法。
 *
 * 立场：**不信任实现者的自述**。全部断言直接打开页面读真实 DOM / 真实数据对象。
 *
 * 用法：
 *   export NODE_PATH="<managed node workspace>/node_modules"
 *   node docs/prototype/verify-aggregate.cjs
 *   node docs/prototype/verify-aggregate.cjs --mutate=aggselect|aggpersist|aggsort|aggscope
 *   （变异模式下**全绿即为失败** —— 说明断言在空转。）
 *
 * 为什么从 `index.html` 加载而不是单文件：模块化版是源码，单文件是产物；
 * 套件跑源码才能抓到手改产物的分叉。外部 `<script src>` 在这里被内联，
 * 只为拿到一个**非 opaque origin**（`http://localhost/`）—— 否则 jsdom 的
 * `localStorage` 会抛 SecurityError，持久化那几条断言就没法真跑。
 */
const path = require("path");
const fs = require("fs");
const { JSDOM } = require("jsdom");

const BASE = __dirname;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];

const MUT = (process.argv.find((a) => a.startsWith("--mutate")) || "").replace("--mutate", "").replace(/^=/, "");
const MUTATE = Boolean(MUT);

const MUTATIONS = {
  aggselect: {
    desc: "把「勾选」当成「筛选」（两个选择集混成一个）",
    file: "demo-app.js",
    apply: (src) => {
      const from = "state.selected.has(a1) ? state.selected.delete(a1) : state.selected.add(a1);";
      const to = "state.selected.has(a1) ? state.selected.delete(a1) : state.selected.add(a1); if (state.agg) state.aggVisible = new Set(state.selected); /* MUTATED */";
      if (!src.includes(from)) throw new Error("未匹配到 acct-select 行");
      return src.replace(from, to);
    },
  },
  aggpersist: {
    desc: "写盘不规范化（选满也存成一份「筛选」）",
    file: "demo-app.js",
    apply: (src) => {
      const from = "if (!set || !set.size || set.size === all.size) localStorage.removeItem(AGG_PREF_KEY);";
      /* 只改条件、不动 if/else 结构：选满时落到 else 分支，把「全部键」存成一份筛选。 */
      const to = "if (!set || !set.size) localStorage.removeItem(AGG_PREF_KEY); /* MUTATED: 选满不规范化 */";
      if (!src.includes(from)) throw new Error("未匹配到 writeAggVisible 的删键行");
      return src.replace(from, to);
    },
  },
  aggsort: {
    desc: "平铺视图不排序（只是打散分组，等于没有新信息）",
    file: "demo-app.js",
    apply: (src) => {
      const from = "const sorted = shown.slice().sort((x, y) => aggUrgency(x.a) - aggUrgency(y.a)";
      const to = "const sorted = shown.slice().sort((x, y) => 0 /* MUTATED */";
      if (!src.includes(from)) throw new Error("未匹配到平铺排序行");
      return src.replace(from, to);
    },
  },
  aggscope: {
    desc: "卡片用裸 id 作 ref（跨作用域串号）",
    file: "demo-app.js",
    apply: (src) => {
      const from = "const ref = o.scope ? refOf(o.scope, a.id) : a.id;";
      const to = "const ref = a.id; /* MUTATED: 丢掉作用域 */";
      if (!src.includes(from)) throw new Error("未匹配到 accountCard 的 ref 行");
      return src.replace(from, to);
    },
  },
};

function readSrc(file) {
  let src = fs.readFileSync(path.join(BASE, file), "utf8");
  if (MUTATE && MUTATIONS[MUT] && MUTATIONS[MUT].file === file) src = MUTATIONS[MUT].apply(src);
  return src;
}

function buildHtml() {
  let html = fs.readFileSync(path.join(BASE, "index.html"), "utf8");
  const inline = (tag, file) => {
    html = html.replace(tag, `<script>\n${readSrc(file)}\n</script>`);
  };
  inline('<script src="demo-data.js"></script>', "demo-data.js");
  inline('<script src="demo-app.js"></script>', "demo-app.js");
  return html;
}

function check(name, fn) {
  try {
    const r = fn();
    results.push({ name, ok: r !== false, detail: r === true || r === undefined ? "" : String(r) });
  } catch (e) {
    results.push({ name, ok: false, detail: e.message });
  }
}

(async () => {
  if (MUTATE && !MUTATIONS[MUT]) {
    console.error("未知变异：" + MUT + "（可选 " + Object.keys(MUTATIONS).join(" / ") + "）");
    process.exit(2);
  }

  const dom = new JSDOM(buildHtml(), {
    url: "http://localhost/",
    runScripts: "dangerously",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  const doc = window.document;
  const $ = (s) => doc.querySelector(s);
  const $$ = (s) => [...doc.querySelectorAll(s)];
  const click = (s) => { const n = typeof s === "string" ? $(s) : s; if (!n) throw new Error("找不到元素: " + s); n.click(); };
  const ev = (code) => window.eval(code);
  const txt = (s) => { const n = $(s); return n ? n.textContent.replace(/\s+/g, " ").trim() : ""; };

  await new Promise((r) => window.addEventListener("load", r));
  await sleep(80);
  ev("localStorage.clear()");

  /* ══════════ 落位：聚合是「视图模式」，不是第 7 个作用域 ══════════ */
  check("★ 作用域菜单第一项是「聚合」，且与 6 个作用域用分隔线隔开", () => {
    click("#scope-trigger");
    const items = $$("#scope-menu .scope-item");
    if (!items.length) throw new Error("菜单空");
    if (!items[0].classList.contains("agg-entry")) throw new Error("第一项不是聚合");
    if (!$("#scope-menu .sep")) throw new Error("没有分隔线（会被读成第 7 个作用域）");
    return "第一项 + 分隔线";
  });

  check("★ 聚合**不在** SCOPES 里（不污染封闭字面量类型与快捷键序号）", () => {
    const ids = ev("Object.keys(SCOPES)");
    if (ids.length !== 6) throw new Error("SCOPES 长度是 " + ids.length);
    if (ids.includes("aggregate")) throw new Error("aggregate 被塞进了 SCOPES");
    return "6 个作用域，无 aggregate";
  });

  check("★ 6 个作用域项各自仍带 ⌘1–⌘6（聚合项不占序号）", () => {
    const keys = $$("#scope-menu .scope-item:not(.agg-entry)").map((b) => b.textContent.replace(/\s+/g, " ").trim());
    for (let i = 1; i <= 6; i += 1) if (!keys.some((k) => k.includes("⌘" + i))) throw new Error("缺少 ⌘" + i);
    const agg = $$("#scope-menu .agg-entry")[0].textContent;
    if (/⌘\d/.test(agg)) throw new Error("聚合项不该有快捷键序号（它不是作用域）");
    return "⌘1–⌘6 完整";
  });

  /* ══════════ 进入聚合：独立界面，没有侧栏 ══════════ */
  click('[data-act="scope|aggregate"]');
  await sleep(60);

  check("★ 进入聚合后**没有左侧导航栏**（CSS 规则 + body 标记双证）", () => {
    /* jsdom 不做布局、也不加载 `<link>` 外部样式 ⇒ 这里断言「规则存在 + 标记已加」，
       而不是 `getComputedStyle` 的 display（那会恒为 block，永远绿不了也永远红不对）。 */
    if (!doc.body.classList.contains("agg-mode")) throw new Error("body 上没有 agg-mode");
    const css = fs.readFileSync(path.join(BASE, "demo.css"), "utf8");
    if (!/body\.agg-mode\s+\.sidebar\s*\{\s*display:\s*none/.test(css)) throw new Error("demo.css 里没有隐藏侧栏的规则");
    return "body.agg-mode + css 规则";
  });

  check("★ 聚合页不是「套壳的普通页面」：没有 page-head（标题交给顶栏）", () => {
    if ($(".page-head")) throw new Error("出现了 page-head");
    if (!$(".agg-bar")) throw new Error("没有吸顶工具栏");
    return "只有 agg-bar + 卡片区";
  });

  check("★ 顶栏仍是唯一公共外壳，且显示「聚合 · 全部账号」+ 全量汇总", () => {
    const name = txt("#scope-name");
    if (!name.includes("聚合")) throw new Error("顶栏没切到聚合：" + name);
    const sub = txt("#scope-sub");
    const total = ev("Object.keys(SCOPES).reduce((n,id)=>n+SCOPES[id].accounts.length,0)");
    if (!sub.includes(String(total))) throw new Error(`副标题没汇总总数（期望 ${total}）：` + sub);
    return name + " / " + sub;
  });

  /* ══════════ R1 一屏看全 ══════════ */
  check("★ 渲染 6 个作用域的全部账号（卡片数 = 账号总数）", () => {
    const total = ev("Object.keys(SCOPES).reduce((n,id)=>n+SCOPES[id].accounts.length,0)");
    const cards = $$(".acct").length;
    if (cards !== total) throw new Error(`卡片 ${cards} ≠ 账号 ${total}`);
    return cards + " 张卡";
  });

  check("★ 每张卡都是账号页那张卡（四块结构齐全，不是聚合页专用卡）", () => {
    const cards = $$(".acct");
    for (const c of cards) {
      for (const cls of [".acct-top", ".acct-chips", ".acct-metrics", ".acct-actions"]) {
        if (!c.querySelector(cls)) throw new Error("卡片缺 " + cls);
      }
      /* 卡内可点区：勾选 / 程序位 / 更多 —— 与账号页同一份模板，聚合页不许加控件。 */
      const acts = new Set([...c.querySelectorAll("[data-act]")].map((n) => n.dataset.act.split("|")[0]));
      for (const a of acts) if (!["acct-select", "acct-prog", "acct-more", "acct-open"].includes(a)) throw new Error("卡片里出现了新动作：" + a);
    }
    return cards.length + " 张卡结构一致";
  });

  check("★ 分组视图的组标题承担「这是哪个渠道」，卡上因此不加 chip", () => {
    const heads = $$(".agg-group-head").length;
    if (heads < 1) throw new Error("没有组标题");
    if ($$(".acct-scope-line").length) throw new Error("分组视图里卡片出现了 scope-line（与组标题重复）");
    return heads + " 个组";
  });

  /* ══════════ R4 两个选择集互不写入 ══════════ */
  check("★★ 勾选卡片**不改变可见性**（批量选择 ≠ 筛选）", () => {
    const before = $$(".acct").length;
    const checks = $$(".acct .check");
    checks[0].click();
    checks[1].click();
    const after = $$(".acct").length;
    if (after !== before) throw new Error(`勾了 2 张后卡片数从 ${before} 变成 ${after}`);
    if (ev("state.aggVisible") !== null) throw new Error("勾选写进了 aggVisible");
    return "卡片数不变 · aggVisible 仍为 null";
  });

  check("★ 批量条写明「跨 N 个作用域」（否则不知道批量删除会动几个渠道）", () => {
    const bar = $(".batch-bar");
    if (!bar) throw new Error("没有批量条");
    if (!bar.textContent.includes("跨")) throw new Error("批量条没说跨几个作用域：" + bar.textContent);
    return bar.textContent.replace(/\s+/g, " ").trim().slice(0, 30);
  });

  click('[data-act="batch-clear"]');
  await sleep(40);

  /* ══════════ 筛选：应用后生效 + 持久化 ══════════ */
  check("★★ 筛选面板**应用后才生效**（勾了不点应用，列表不变）", () => {
    const before = $$(".acct").length;
    click("#agg-filter-btn");
    click('[data-act="agg-fp-toggle|trae-cn"]');   // 勾上 Trae 国内版
    const mid = $$(".acct").length;
    if (mid !== before) throw new Error("勾选时列表就变了（live 生效，用户在面板后面看不见）");
    click('[data-act="agg-fp-apply"]');
    const after = $$(".acct").length;
    if (after === before) throw new Error("点应用后列表没变");
    return `勾选时 ${before} → 应用后 ${after}`;
  });

  check("★★ 筛选持久化到 localStorage（键 = buddy-switch:v1:aggregate-visible）", () => {
    const raw = ev("localStorage.getItem('buddy-switch:v1:aggregate-visible')");
    if (!raw) throw new Error("没写盘");
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr) || !arr.length) throw new Error("存的不是账号键数组：" + raw);
    if (!arr.every((k) => k.includes(":"))) throw new Error("键里没有作用域前缀：" + raw);
    return arr.length + " 个键";
  });

  check("★ 筛选生效时工具栏有**常驻**提示 + 「显示全部」出口（不是一次性 toast）", () => {
    const c = $("#agg-count");
    if (!c) throw new Error("没有计数");
    if (!c.classList.contains("filtered")) throw new Error("计数没有进入 filtered 态");
    if (!$('[data-act="agg-clear-filter"]')) throw new Error("没有「显示全部」出口");
    return txt("#agg-count").slice(0, 30);
  });

  check("★★ 选满 == 无筛选（规范化成删键，否则新增账号会看不见）", () => {
    click("#agg-filter-btn");
    click('[data-act="agg-fp-all"]');              // 全选
    click('[data-act="agg-fp-apply"]');
    const raw = ev("localStorage.getItem('buddy-switch:v1:aggregate-visible')");
    if (raw !== null) throw new Error("选满后仍存着键：" + raw);
    if (ev("state.aggVisible") !== null) throw new Error("aggVisible 没回到 null");
    const total = ev("Object.keys(SCOPES).reduce((n,id)=>n+SCOPES[id].accounts.length,0)");
    if ($$(".acct").length !== total) throw new Error("选满后没有恢复全量");
    return "键已删除 · 回到全量";
  });

  check("★ 规范化是**双保险**：写盘函数自己也拒绝存「全部键」", () => {
    /* UI 层（agg-fp-apply）和存储层（writeAggVisible）各做一次规范化 ——
       任一层被后人删掉，另一层仍然兜住。这里单测存储层，避免只测 UI 层时
       存储层失效却看不出来（变异 aggpersist 就靠这条打红）。 */
    ev("localStorage.clear(); writeAggVisible(new Set(aggAllRefs()))");
    if (ev("localStorage.getItem('buddy-switch:v1:aggregate-visible')") !== null) {
      throw new Error("writeAggVisible 把「全部键」存进了 localStorage");
    }
    ev("writeAggVisible(new Set([...aggAllRefs()].slice(0, 3)))");
    if (ev("localStorage.getItem('buddy-switch:v1:aggregate-visible')") === null) {
      throw new Error("真子集反而没存（规范化写反了）");
    }
    ev("localStorage.clear()");
    return "全部键 ⇒ 删 · 子集 ⇒ 存";
  });

  check("★ 偏好损坏 / 存储不可用时回退全量（绝不白屏）", () => {
    ev("localStorage.setItem('buddy-switch:v1:aggregate-visible', '{oops')");
    const v = ev("readAggVisible()");
    if (v !== null) throw new Error("损坏 JSON 没有回退 null");
    ev("localStorage.setItem('buddy-switch:v1:aggregate-visible', JSON.stringify(['wb-cn:不存在','bad-key']))");
    if (ev("readAggVisible()") !== null) throw new Error("全失效键没有回退全量");
    ev("localStorage.clear()");
    return "损坏 / 全失效 ⇒ null";
  });

  /* ══════════ 三种「空」互斥 ══════════ */
  check("★ 未安装 / 没有账号的作用域：一行说明，**不渲染组标题**（否则名字出现两次）", () => {
    const notes = $$(".agg-note");
    const zero = ev("Object.keys(SCOPES).filter(id=>!SCOPES[id].accounts.length)");
    if (!zero.length) throw new Error("数据里没有 0 账号的作用域，断言前提不成立");
    if (notes.length !== zero.length) throw new Error(`说明行 ${notes.length} ≠ 空作用域 ${zero.length}`);
    const heads = $$(".agg-group-head").map((h) => h.textContent);
    for (const id of zero) {
      const label = ev(`scopeLabel(${JSON.stringify(id)})`);
      if (heads.some((h) => h.includes(label))) throw new Error(`空作用域「${label}」仍渲染了组标题`);
    }
    return zero.length + " 行说明";
  });

  check("★★ 某个作用域取数失败：组内整行占位 + 其余照常 + **组标题不写账号数**", () => {
    ev("state.aggFailed.add('qoder-cn'); renderContent()");
    const fail = $(".agg-fail");
    if (!fail) throw new Error("没有失败占位");
    if (!fail.textContent.includes("重试")) throw new Error("占位没有重试出口");
    const head = $$(".agg-group-head").find((h) => h.textContent.includes("Qoder 国内版"));
    if (!head) throw new Error("失败作用域的组标题没了（用户会以为整组被删）");
    if (/\d+\s*个账号/.test(head.textContent)) throw new Error("失败时仍写着「N 个账号」——拿旧值撒谎");
    if (!head.textContent.includes("加载失败")) throw new Error("组标题没说明失败");
    const others = $$(".acct").length;
    if (!others) throw new Error("其余作用域也没了");
    ev("state.aggFailed.clear(); renderContent()");
    return "占位 + 重试 + 其余 " + others + " 张卡照常";
  });

  /* ══════════ 平铺：必须换排序依据 ══════════ */
  check("★★ 平铺按紧迫度混排（过期在最前），不是「打散分组、顺序照旧」", () => {
    click('[data-act="agg-mode|flat"]');
    const first = $$(".acct")[0];
    const st = first.querySelector(".acct-chips").textContent;
    if (!/凭据已过期|需重新登录|今日未签到/.test(st)) throw new Error("第一张卡不是待处理项：" + st);
    const lines = $$(".acct-scope-line").length;
    const cards = $$(".acct").length;
    if (lines !== cards) throw new Error(`平铺视图每张卡都该带归属说明：${lines}/${cards}`);
    click('[data-act="agg-mode|group"]');
    return "首卡 = " + st.replace(/\s+/g, " ").trim().slice(0, 20);
  });

  /* ══════════ 跨作用域：动作必须落在那张卡所属的作用域 ══════════ */
  check("★★ 点第 7 张卡打开的抽屉是**它所属作用域**的账号（裸 id 会串号）", () => {
    const cards = $$(".acct");
    const target = cards[6];
    const ref = target.dataset.act.split("|")[1];
    if (!ref.includes(":")) throw new Error("聚合页卡片的 ref 没有作用域前缀：" + ref);
    target.click();
    const want = ev(`(() => { const [s,id]=${JSON.stringify(ref)}.split(":"); return SCOPES[s].accounts.find(a=>a.id===id).name; })()`);
    const title = txt("#drawer-title");
    const body = $("#drawer-body").textContent;
    if (!body.includes(want)) throw new Error(`抽屉里不是「${want}」：` + body.replace(/\s+/g, " ").slice(0, 60));
    click("#drawer-close");
    return ref + " → " + want;
  });

  check("★ 聚合页的「更多」菜单头部带作用域（同名号跨平台不混淆）", () => {
    const btn = $$(".acct .acct-actions button").find((b) => b.dataset.act.startsWith("acct-more|"));
    if (!btn) throw new Error("找不到「更多」");
    btn.click();
    const head = $(".acct-menu .am-head");
    if (!head) throw new Error("菜单没打开");
    if (!head.textContent.includes("·")) throw new Error("菜单头没带作用域：" + head.textContent);
    const label = head.textContent.trim();
    /* 关掉：下一条断言要重新打开同一个菜单，否则 toggle 会把它关掉（拿到 0 项）。 */
    doc.body.click();
    return label;
  });

  check("★ 卡片动作在聚合页**原地完成**（没有 navigate / 没有「请前往某作用域」）", () => {
    /* 每次都要重新查按钮：打开菜单会重绘列表，旧节点已脱离文档。 */
    const btn = $$(".acct .acct-actions button").find((b) => b.dataset.act.startsWith("acct-more|"));
    if (!btn) throw new Error("找不到「更多」");
    btn.click();
    const items = $$(".acct-menu .am-item");
    if (items.length < 6) throw new Error("菜单项不足：" + items.length);
    const bad = items.map((i) => i.textContent).find((t) => /前往|切换作用域/.test(t));
    if (bad) throw new Error("菜单里出现了跳转话术：" + bad);
    doc.body.click();
    return items.length + " 项，全部原地";
  });

  /* ══════════ 退出：顶栏是唯一出口 ══════════ */
  click("#scope-trigger");
  click('[data-act="scope|wb-cn"]');
  await sleep(400);
  check("★ 从顶栏作用域菜单选任一作用域即离开聚合，且侧栏恢复", () => {
    if (doc.body.classList.contains("agg-mode")) throw new Error("agg-mode 没摘掉");
    const d = window.getComputedStyle($(".sidebar")).display;
    if (d === "none") throw new Error("侧栏仍隐藏");
    if (!$(".page-head")) throw new Error("没回到账号页");
    if (ev("state.agg")) throw new Error("state.agg 还是 true");
    return "侧栏恢复 · " + txt(".page-head h1");
  });

  /* ─────────── 汇总 ─────────── */
  const pass = results.filter((r) => r.ok).length;
  const fail = results.length - pass;
  for (const r of results) console.log(`  ${r.ok ? "ok  " : " FAIL"} ${r.name}${r.detail ? "  → " + r.detail : ""}`);
  console.log(`\n${pass} passed / ${fail} failed`);

  if (MUTATE) {
    if (fail === 0) {
      console.error(`\n✗ 变异「${MUT}」没有打红任何断言 —— 断言在空转：${MUTATIONS[MUT].desc}`);
      process.exit(1);
    }
    console.log(`\n✓ 变异「${MUT}」打红了 ${fail} 条（${MUTATIONS[MUT].desc}）`);
    process.exit(0);
  }
  process.exit(fail ? 1 : 0);
})();
