/* 原型验证：用 jsdom 真跑一遍交互，断言不是「看起来对」。
 * 用法：
 *   NODE_PATH=<managed workspace>/node_modules node docs/prototype/verify-demo.cjs [demoDir]
 *   ... --mutate   注入一个变异（去掉启动渲染），**期望测试变红**，用于证明断言不空转。
 */
const path = require("path");
const fs = require("fs");
const os = require("os");
const { JSDOM } = require("jsdom");

const BASE = __dirname;
/* 变异注入：把缺陷塞回去，**期望测试变红**。全绿就说明断言在空转。 */
const MUT = (process.argv.find((a) => a.startsWith("--mutate")) || "").replace("--mutate", "").replace(/^=/, "");
const MUTATE = Boolean(MUT);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const MUTATIONS = {
  boot: {
    desc: "去掉启动渲染（模拟白屏）",
    file: "demo-app.js",
    apply: (src) => {
      const out = src.replace(/\nbootFromUrl\(\);/,
        "\n/* MUTATED: 启动渲染被移除 */\ndocument.querySelector('#content').innerHTML = '';");
      if (out === src) throw new Error("未匹配到启动 bootFromUrl()");
      return out;
    },
  },
  cap: {
    desc: "忽略能力开关（模拟平台硬编码分支，让 Trae 也出现「自动旅行」）",
    file: "demo-app.js",
    apply: (src) => {
      const from = "const rows = g.rows.filter((r) => !r.cap || p.capabilities.includes(r.cap));";
      const to = "const rows = g.rows.filter(() => true); /* MUTATED: 忽略能力开关 */";
      if (!src.includes(from)) throw new Error("未匹配到能力过滤行");
      return src.replace(from, to);
    },
  },
  ime: {
    desc: "搜索时整页重绘（模拟打断中文输入法）",
    file: "demo-app.js",
    apply: (src) => {
      const from = 'if (t.id === "acct-q") { state.q = t.value; renderAccountList(); return; }';
      const to = 'if (t.id === "acct-q") { state.q = t.value; renderContent(); return; } /* MUTATED */';
      if (!src.includes(from)) throw new Error("未匹配到搜索处理行");
      return src.replace(from, to);
    },
  },
  /* ★ Qoder（第三个平台）—— 数据层变异：区域标签错位。
     命中「作用域菜单含 Qoder 的两个区域」这条断言。 */
  qoderregion: {
    desc: "Qoder 国际版被误标成国内版（区域标签错位）",
    file: "demo-data.js",
    apply: (src) => {
      const from = '    id: "qoder-global", platform: "qoder", region: "global",';
      const to = '    id: "qoder-global", platform: "qoder", region: "cn", /* MUTATED */';
      if (!src.includes(from)) throw new Error("未匹配到 qoder-global 的 region 行");
      return src.replace(from, to);
    },
  },
  /* ★ 限额护栏卡（2026-10-21）：池表直接顶上来、卡片不渲染。
     期望「有护栏卡」「卡在池表之前」两条一起变红 —— 只删渲染、不动数据。 */
  quotacard: {
    desc: "限额护栏卡未渲染（账号池表直接顶到分区顶部）",
    file: "demo-app.js",
    apply: (src) => {
      const from = "main = quotaCardHtml(g) + `";
      const to = "main = `` + ` /* MUTATED: 护栏卡被移除 */";
      if (!src.includes(from)) throw new Error("未匹配到 quotaCardHtml 渲染行");
      return src.replace(from, to);
    },
  },
  /* ★ 顶栏刷新按钮的进行中反馈（2026-10-15）：去掉 doRefresh 的 spinning/禁用注入，
     期望「点击后进入进行中态」那条断言变红（只删注入、不碰 500ms 计时与 toast）。 */
  refresh: {
    desc: "刷新按钮点击后无进行中反馈（去掉 spinning + 禁用）",
    file: "demo-app.js",
    apply: (src) => {
      const from = '  if (btn) { btn.classList.add("spinning"); btn.disabled = true; }\n';
      const to = '  /* MUTATED: 进行中反馈被移除 */\n';
      if (!src.includes(from)) throw new Error("未匹配到 spinning 注入行");
      return src.replace(from, to);
    },
  },
};

let dir = BASE;
if (MUTATE) {
  if (!MUTATIONS[MUT]) { console.error("未知变异：" + MUT + "（可选 " + Object.keys(MUTATIONS).join(" / ") + "）"); process.exit(2); }
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "bsdemo-"));
  for (const f of fs.readdirSync(BASE)) fs.copyFileSync(path.join(BASE, f), path.join(dir, f));
  const m = MUTATIONS[MUT];
  const p = path.join(dir, m.file);
  /* 锚点统一在 LF 归一化后的文本上做匹配，写回时还原原换行风格 —— 与 verify-behaviors.cjs 一致。
     本仓文件是 CRLF，若拿含 `\n` 的字面量直接 includes 会静默不匹配 ⇒ 假验证。 */
  const src = fs.readFileSync(p, "utf8");
  const eol = src.includes("\r\n") ? "\r\n" : "\n";
  fs.writeFileSync(p, m.apply(src.replace(/\r\n/g, "\n")).replace(/\n/g, eol));
  console.log(`【变异注入】${m.desc}\n`);
}

const results = [];
function check(name, fn) {
  try { const r = fn(); results.push({ name, ok: r !== false, detail: r === true || r === undefined ? "" : String(r) }); }
  catch (e) { results.push({ name, ok: false, detail: e.message }); }
}

(async () => {
  const dom = await JSDOM.fromFile(path.join(dir, "index.html"), {
    runScripts: "dangerously", resources: "usable", pretendToBeVisual: true,
  });
  const { window } = dom;
  const doc = window.document;
  const $ = (s) => doc.querySelector(s);
  const $$ = (s) => [...doc.querySelectorAll(s)];
  const click = (s) => { const n = typeof s === "string" ? $(s) : s; if (!n) throw new Error("找不到元素: " + s); n.click(); };
  const text = (s) => { const n = $(s); return n ? n.textContent.trim() : ""; };

  await new Promise((r) => window.addEventListener("load", r));
  await sleep(60);

  /* ── 骨架 ── */
  check("首屏非空白（#content 有实质内容）", () => {
    const len = $("#content").innerHTML.length;
    if (len < 800) throw new Error("#content 只有 " + len + " 字符，疑似白屏");
    return len + " 字符";
  });
  check("侧栏默认 6 个目的地（含「会话」，可配置）", () => {
    const items = $$("#nav .nav-item");
    if (items.length !== 6) throw new Error("实际 " + items.length + " 项");
    const labels = items.map((el) => el.querySelector(".nav-label").textContent.trim());
    for (const want of ["概览", "账号", "用量", "会话", "网关", "设置"]) {
      if (!labels.includes(want)) throw new Error("缺少「" + want + "」：" + labels.join("/"));
    }
    return labels.join(" / ");
  });
  check("作用域名默认 = WorkBuddy 国内版", () => {
    const t = text("#scope-name");
    if (t !== "WorkBuddy 国内版") throw new Error("实际「" + t + "」");
    return true;
  });
  check("顶栏运行态徽章 = 运行中", () => {
    if (!text("#run-pill").includes("运行中")) throw new Error("实际「" + text("#run-pill") + "」");
    return true;
  });
  check("概览页有「待处理」区块", () => {
    if (!text("#content").includes("待处理")) throw new Error("缺少待处理区块");
    return true;
  });

  /* ── 账号工作台 ── */
  try {
  click('[data-act="nav|accounts"]');
  await sleep(30);
  check("账号页渲染 8 张账号卡", () => {
    const n = $$(".acct").length;
    if (n !== 8) throw new Error("实际 " + n + " 张");
    return true;
  });
  check("账号页有搜索/排序/分组工具栏", () => {
    for (const k of ["#acct-q", '[data-input="sort"]', '[data-input="group"]']) if (!$(k)) throw new Error("缺少 " + k);
    return true;
  });

  /* 搜索：只重绘列表区，输入框 DOM 不换（中文输入法不被打断） */
  const inputBefore = $("#acct-q");
  inputBefore.value = "jackey";
  inputBefore.dispatchEvent(new window.Event("input", { bubbles: true }));
  await sleep(30);
  check("搜索 jackey 收敛到 1 张卡", () => {
    const n = $$(".acct").length;
    if (n !== 1) throw new Error("实际 " + n + " 张");
    return true;
  });
  check("搜索时输入框 DOM 未被替换（保住光标与 IME）", () => {
    if ($("#acct-q") !== inputBefore) throw new Error("输入框被重绘了");
    return true;
  });
  const q2 = $("#acct-q");
  q2.value = "zzz-no-match";
  q2.dispatchEvent(new window.Event("input", { bubbles: true }));
  await sleep(30);
  check("无匹配时给出空态 + 清除搜索", () => {
    const t = text("#content");
    if (!t.includes("没有匹配的账号")) throw new Error("没有空态");
    if (!$('[data-act="clear-q"]')) throw new Error("没有清除搜索按钮");
    return true;
  });
  click('[data-act="clear-q"]');
  await sleep(30);
  check("空态里点清除搜索恢复列表", () => {
    const n = $$(".acct").length;
    if (n !== 8) throw new Error("实际 " + n + " 张");
    return true;
  });

  /* 排序 / 分组 / 视图 */
  const sel = $('[data-input="group"]');
  sel.value = "status";
  sel.dispatchEvent(new window.Event("change", { bubbles: true }));
  await sleep(30);
  check("按状态分组出现分组标题", () => {
    const heads = $$("#acct-list .badge").map((b) => b.textContent.trim());
    if (!heads.some((h) => ["正常", "今日未签到", "凭据已过期", "需重新登录"].includes(h))) throw new Error("没有分组标题：" + heads.join(","));
    return true;
  });
  click('[data-act="view|table"]');
  await sleep(30);
  check("切换到表格视图", () => {
    if (!$("table.tbl")) throw new Error("没有渲染表格");
    return true;
  });
  click('[data-act="view|card"]');
  await sleep(30);

  /* 勾选 + 批量条 */
  click($("#acct-list .check"));
  await sleep(30);
  check("勾选后浮现批量操作条", () => {
    const bar = $(".batch-bar");
    if (!bar) throw new Error("批量条未出现");
    if (!bar.textContent.includes("已选 1 个")) throw new Error("计数不对：" + bar.textContent.trim());
    return true;
  });
  click('[data-act="batch-clear"]');
  await sleep(30);
  check("取消选择后批量条消失", () => {
    if ($(".batch-bar")) throw new Error("批量条仍在");
    return true;
  });

  /* 程序位切换（含未检测到的分支） */
  click('[data-act="acct-prog|a3|ide"]');
  await sleep(30);
  check("可把 IDE 程序位切到 backup-01", () => {
    const t = $$(".toast").map((x) => x.textContent).join(" | ");
    if (!t.includes("CodeBuddy IDE") || !t.includes("backup-01")) throw new Error("toast 内容不符：" + t);
    return true;
  });
  click('[data-act="acct-prog|a3|cli"]');
  await sleep(30);
  check("未检测到的程序位给出解释而非静默失败", () => {
    const t = $$(".toast").map((x) => x.textContent).join(" | ");
    if (!t.includes("未检测到客户端")) throw new Error("toast 内容不符：" + t);
    return true;
  });

  /* 抽屉 */
  click($("#acct-list .acct"));
  await sleep(30);
  check("点击账号卡打开右侧抽屉", () => {
    if (!$("#drawer").classList.contains("open")) throw new Error("抽屉未打开");
    if (!text("#drawer-body").includes("账号信息")) throw new Error("抽屉内容不符");
    return true;
  });
  check("抽屉内可进入三步切换向导", () => {
    click('[data-act^="wizard|"]');
    if (!text("#drawer-body").includes("目标程序位")) throw new Error("向导第一步未渲染");
    return true;
  });
  click('[data-act="wiz-next"]');
  check("向导第二步 = 迁移内容", () => {
    if (!text("#drawer-body").includes("迁移内容")) throw new Error("第二步未渲染");
    return true;
  });
  click('[data-act="wiz-next"]');
  check("向导第三步 = 确认页", () => {
    if (!text("#drawer-body").includes("影响")) throw new Error("确认页未渲染");
    return true;
  });
  click("#drawer-close");
  await sleep(30);

  /* ── 作用域切换 ── */
  click("#scope-trigger");
  await sleep(30);
  /* ⚠️ 这里**写死 6**，不拿 `Object.keys(SCOPES).length` 对拍 ——
     菜单本就是从 SCOPES 生成的，对拍恒真（永远绿）。写死才是对
     「3 平台 × 2 区域」这条**设计意图**的钉死。 */
  check("作用域菜单列出 6 个作用域（3 平台 × 2 区域）", () => {
    /* `:not(.agg-entry)` —— 菜单第一项是「聚合」入口（issue #50），它**不是作用域**
       （`Scope` 是封闭字面量 `${平台}-${区域}`，塞进去会同时破坏类型、注册表一致性与
       快捷键序号）。所以数作用域时要把它排除掉，而不是把 6 改成 7。 */
    const n = $$("#scope-menu .scope-item:not(.agg-entry)").length;
    if (n !== 6) throw new Error("实际 " + n + " 个，期望 6 个");
    return true;
  });
  check("★ 作用域菜单第一项是「聚合」入口，且与 6 个作用域用分隔线隔开", () => {
    const items = $$("#scope-menu .scope-item");
    if (!items.length) throw new Error("菜单是空的");
    if (!items[0].classList.contains("agg-entry")) throw new Error("第一项不是聚合入口");
    if (items.filter((b) => b.classList.contains("agg-entry")).length !== 1) throw new Error("聚合入口应恰好一项");
    if (!$("#scope-menu .sep")) throw new Error("聚合入口与作用域之间没有分隔线（视觉上会像第 7 个作用域）");
    return "第一项 + 分隔线";
  });
  check("作用域菜单含 Qoder 的两个区域", () => {
    /* ⚠️ 按钮的 textContent 含换行（模板是多行的）⇒ 必须归一成单行再比对，
       否则失败信息会被换行撑成一大段，读不出到底缺哪个。 */
    const labels = $$("#scope-menu .scope-item:not(.agg-entry)").map((b) => b.textContent.replace(/\s+/g, " ").trim());
    for (const want of ["Qoder 国内版", "Qoder 国际版"]) {
      if (!labels.some((l) => l.includes(want))) throw new Error("缺少「" + want + "」：" + labels.join(" / "));
    }
    return "Qoder 国内版 / Qoder 国际版";
  });
  click('[data-act="scope|trae-global"]');
  await sleep(360);
  check("切到 TraeWork 国际版后顶栏跟随", () => {
    const t = text("#scope-name");
    if (t !== "TraeWork 国际版") throw new Error("实际「" + t + "」");
    return true;
  });
  /* 当前作用域是 `trae-global`（0 个账号、本机无客户端）⇒ 走「无客户端形态」引导卡，
     不是旧的「这个作用域还没有账号」。两种都算**结构化空态**，判据是「有块、有出路」，
     而不是某一句固定文案 —— 否则改一次空态设计就要改一次门禁，门禁会退化成文案快照。 */
  check("空作用域给出结构化空态而非空白", () => {
    click('[data-act="nav|accounts"]');
    const t = text("#content");
    const structured = doc.querySelector("#content .empty, #content .srv-card");
    if (!structured) throw new Error("没有空态块：" + t.slice(0, 80));
    if (!/还没有账号|无客户端形态/.test(t)) throw new Error("空态没有说清是什么情况：" + t.slice(0, 80));
    return true;
  });

  /* ── 能力开关驱动 UI（扩展性的核心断言） ── */
  click('[data-act="scope|wb-cn"]');
  await sleep(360);
  click('[data-act="nav|settings"]');
  await sleep(30);
  click('[data-act="set-sub|checkin"]');
  await sleep(30);
  const wbHasTravel = text("#content").includes("自动旅行");
  click('[data-act="scope|trae-cn"]');
  await sleep(360);
  click('[data-act="set-sub|checkin"]');
  await sleep(30);
  const traeHasTravel = text("#content").includes("自动旅行");
  check("能力开关驱动 UI：WorkBuddy 有「自动旅行」", () => wbHasTravel || (() => { throw new Error("WorkBuddy 缺失旅行设置"); })());
  check("能力开关驱动 UI：Trae 无「自动旅行」（同一组件、同一路由）", () => {
    if (traeHasTravel) throw new Error("Trae 不应出现旅行设置——说明页面里存在平台硬编码分支");
    return true;
  });

  /* ── 用量 / 网关 ── */
  click('[data-act="nav|usage"]');
  await sleep(30);
  check("用量工作台：Token 与积分合并、含视图参数条", () => {
    const t = text("#content");
    if (!t.includes("视图参数")) throw new Error("缺少视图参数条");
    for (const k of ["趋势", "模型", "会话", "积分"]) if (!t.includes(k)) throw new Error("缺少子 Tab：" + k);
    return true;
  });
  click('[data-act="usage-tab|models"]');
  await sleep(30);
  /* ★ 2026-10-21 账本改版：模型 tab 的「横向排行条（.hbar + 迷你趋势线）」
     换成了「每本账一张表 + 表内分布条（.lbar）」，并补了每日按模型堆叠图。
     断言跟着改成新形态 —— 要的是「模型维度有可视化的量分布」，不是某个类名。 */
  check("模型用量分布有横向条（账本表内的分布条）", () => {
    const bars = $$("#content .lbar i").filter((i) => parseFloat(i.style.width) > 0);
    if (!bars.length) throw new Error("没有 .lbar 分布条");
    if (!$$("#content .cbar").length) throw new Error("缺少每日按模型堆叠图");
    return true;
  });

  click('[data-act="nav|gateway"]');
  await sleep(30);
  check("网关工作台有 6 项子导航（不再是 6 卡堆叠）", () => {
    const n = $$(".subnav button").length;
    if (n !== 6) throw new Error("实际 " + n + " 项");
    /* v2.7 起第 6 项是「请求代理」（§3.19 / P12） */
    const labels = $$(".subnav button").map((b) => b.textContent.trim());
    if (!labels.includes("请求代理")) throw new Error("子导航缺少「请求代理」：" + labels.join("/"));
    return labels.join("/");
  });
  click('[data-act="gw-sub|keys"]');
  await sleep(30);
  check("密钥表只显示掩码", () => {
    const t = text("#content");
    if (!t.includes("••")) throw new Error("没有掩码");
    if (/bs_live_[0-9a-f]{8}(?!•)/.test(t)) throw new Error("疑似泄露明文密钥");
    return true;
  });
  click('[data-act="gw-sub|logs"]');
  await sleep(30);
  check("请求日志渲染出状态码", () => {
    if (!text("#content").includes("200")) throw new Error("没有状态码");
    return true;
  });

  /* ── 命令面板 ── */
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true }));
  await sleep(40);
  check("⌘K 打开命令面板", () => {
    if (!$("#palette-overlay").classList.contains("open")) throw new Error("面板未打开");
    return true;
  });
  check("面板包含账号/页面/作用域/动作四类", () => {
    const t = text("#palette-list");
    for (const k of ["页面", "作用域", "账号", "动作"]) if (!t.includes(k)) throw new Error("缺少分组：" + k);
    return true;
  });
  const pi = $("#palette-input");
  pi.value = "jackey";
  pi.dispatchEvent(new window.Event("input", { bubbles: true }));
  await sleep(30);
  check("面板搜索收敛", () => {
    const n = $$("#palette-list .palette-item").length;
    if (n < 1) throw new Error("没有结果");
    if (n > 12) throw new Error("结果过多：" + n);
    return true;
  });
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await sleep(30);
  check("ESC 关闭命令面板", () => {
    if ($("#palette-overlay").classList.contains("open")) throw new Error("面板未关闭");
    return true;
  });

  /* ── 主题 ── */
  const wasDark = doc.documentElement.classList.contains("dark");
  click("#theme-btn");
  await sleep(30);
  check("明暗主题可切换", () => {
    if (doc.documentElement.classList.contains("dark") === wasDark) throw new Error("class 未变化");
    return true;
  });

  /* ── 顶栏刷新：进行中反馈（点击 → 图标旋转 + 禁用 → 完成复原 + toast）。
     断言锚在**按钮自身状态**（DOM 探针）而非时长猜测：删掉 doRefresh 的注入
     （变异 `refresh`）第一条必红；「复原」那条守住「加了不移除」的反向缺陷。 ── */
  const refreshBtn = $("#refresh-btn");
  click("#refresh-btn");
  await sleep(50);
  check("刷新：点击后按钮进入进行中态（spinning + disabled）", () => {
    if (!refreshBtn.classList.contains("spinning")) throw new Error("缺少 spinning（点击无动效）");
    if (!refreshBtn.disabled) throw new Error("未 disabled（可重复点击）");
    return true;
  });
  await sleep(700);
  check("刷新：完成后复原（不再 spinning / 不再禁用）", () => {
    if (refreshBtn.classList.contains("spinning")) throw new Error("spinning 残留未移除");
    if (refreshBtn.disabled) throw new Error("完成后仍处于禁用");
    return true;
  });
  check("刷新：完成 toast 标题为「已刷新」（精确匹配标题，防批量条文案冒充）", () => {
    const titles = [...doc.querySelectorAll("#toasts .toast b")].map((b) => b.textContent.trim());
    if (!titles.includes("已刷新")) throw new Error("toast 标题：" + titles.join(" / "));
    return true;
  });

    /* ── 限额护栏（Qoder 网关 · 账号池）────────────────────────────────
       与 App 的 `QoderQuotaGuard` 同构：四阈值 + 逐账号用量 + 拦截原因。
       单文件那一套由 `verify-qoder-prototype.cjs` 守；这里守**门禁跑的这一套**。
       另起一个 JSDOM 深链到账号池分区（不干扰上面的交互序列）。 ── */
    const qUrl = (q) => "file://" + path.join(dir, "index.html").replace(/\\/g, "/") + "?" + q;
    const domQ = await JSDOM.fromFile(path.join(dir, "index.html"), {
      runScripts: "dangerously", resources: "usable", pretendToBeVisual: true,
      url: qUrl("scope=qoder-cn&dest=gateway&gw=pool"),
    });
    const dQ = domQ.window.document;
    await new Promise((r) => domQ.window.addEventListener("load", r));
    await sleep(80);
    const card = dQ.querySelector("[data-quota-card]");
    check("拆分形态：Qoder 账号池顶部有「限额护栏」卡，四个阈值输入齐全", () => {
      if (!card) throw new Error("没有 [data-quota-card]");
      const keys = [...card.querySelectorAll("[data-quota]")].map((i) => i.dataset.quota);
      for (const k of ["reserve", "daily", "credit", "model"]) {
        if (!keys.includes(k)) throw new Error("缺输入 " + k + "：" + keys.join("/"));
      }
      return keys.join(" / ");
    });
    check("拆分形态：护栏卡排在账号池表**之前**（它是池的上游条件）", () => {
      const html = dQ.querySelector("#content").innerHTML;
      const a = html.indexOf("data-quota-card");
      const b = html.indexOf('data-act="pin|');
      if (a < 0 || b < 0 || a > b) throw new Error(`顺序不对（卡 ${a} / 池表 ${b}）`);
      return true;
    });
    check("拆分形态：每种拦截态都给**可读原因**（徽标 title 非空）", () => {
      const blocked = [...dQ.querySelectorAll("[data-quota-card] .badge")].filter(
        (b) => b.classList.contains("warn") || b.classList.contains("bad"),
      );
      if (blocked.length < 3) throw new Error("被拦徽标只有 " + blocked.length + " 个（夹具应有 3 态）");
      for (const b of blocked) {
        if (!(b.getAttribute("title") || "").trim()) throw new Error("徽标缺原因：" + b.textContent);
      }
      return blocked.length + " 个被拦徽标都有原因";
    });
    check("拆分形态：保存会**真的改数据**（不是只弹 toast）", () => {
      const input = card.querySelector('[data-quota="daily"]');
      input.value = "1234";
      card.querySelector('[data-act="quota-save"]').click();
      const again = dQ.querySelector('[data-quota="daily"]');
      if (!again || again.value !== "1234") throw new Error("保存后回填值 = " + (again && again.value));
      return true;
    });
    /* ── 表格视图的行尾「更多」：与卡片**同一个菜单**（含签到）──────────
       App 侧对应 `renderActions` + `QoderAccountMenu`；原型此前表格没有任何行尾动作。 */
    const domT = await JSDOM.fromFile(path.join(dir, "index.html"), {
      runScripts: "dangerously", resources: "usable", pretendToBeVisual: true,
      url: qUrl("scope=qoder-cn&dest=accounts&view=table"),
    });
    const dT = domT.window.document;
    await new Promise((r) => domT.window.addEventListener("load", r));
    await sleep(80);
    const tableMore = dT.querySelector('tbody [data-act^="acct-more|"]');
    check("拆分形态：表格视图每行都有行尾「更多」（不是只有卡片有）", () => {
      const rows = dT.querySelectorAll("#content tbody tr").length;
      const mores = dT.querySelectorAll('#content tbody [data-act^="acct-more|"]').length;
      if (!rows || mores !== rows) throw new Error(`行 ${rows} 个、更多按钮 ${mores} 个`);
      return `${mores} 行都有`;
    });
    check("拆分形态：表格行的「更多」打开的是同一个菜单（含「手动签到」）", () => {
      if (!tableMore) throw new Error("找不到行尾更多按钮");
      tableMore.click();
      const menu = dT.getElementById("acct-menu");
      if (!menu) throw new Error("菜单没打开");
      const t = String(menu.textContent || "").replace(/\s+/g, " ").trim();
      for (const want of ["导出账号", "刷新积分"]) {
        if (!t.includes(want)) throw new Error("菜单缺「" + want + "」：" + t.slice(0, 140));
      }
      /* 签到是**三态**：已签到 / 可点 / 需先登录 —— 断言「命中且只命中一种」，
         而不是钉死某一个词（夹具里那条账号恰好已签到，钉死就成了假红）。 */
      const states = ["今日已签到", "手动签到", "需先登录"].filter((w) => t.includes(w));
      if (states.length !== 1) throw new Error("签到三态命中 " + states.length + " 个：" + t.slice(0, 140));
      return "签到态：" + states[0];
    });

    /* ── Qoder 用量：四个子 Tab + 单位说明（拆分形态） ── */
    const domU = await JSDOM.fromFile(path.join(dir, "index.html"), {
      runScripts: "dangerously", resources: "usable", pretendToBeVisual: true,
      url: qUrl("scope=qoder-cn&dest=usage"),
    });
    await new Promise((r) => domU.window.addEventListener("load", r));
    await sleep(80);
    check("拆分形态：Qoder 用量页有四个子 Tab，并标注「按 credits 计费」", () => {
      const t = String(domU.window.document.querySelector("#content").textContent || "")
        .replace(/\s+/g, " ")
        .trim();
      for (const k of ["趋势", "模型", "会话", "积分"]) {
        if (!t.includes(k)) throw new Error("缺少子 Tab：" + k);
      }
      if (!t.includes("按 credits 计费")) throw new Error("缺少单位说明：" + t.slice(0, 160));
      return true;
    });

    /* ⚠️ `check()` 只支持同步函数（返回值不是 `false` 即算通过）⇒ 异步部分必须
       **在 check 外面**先做完，否则断言里的异常会变成未处理的 rejection、静默假绿。 */
    const domW = await JSDOM.fromFile(path.join(dir, "index.html"), {
      runScripts: "dangerously", resources: "usable", pretendToBeVisual: true,
      url: qUrl("scope=wb-cn&dest=gateway&gw=pool"),
    });
    await new Promise((r) => domW.window.addEventListener("load", r));
    await sleep(80);
    check("拆分形态：WorkBuddy 作用域**不渲染**护栏卡（没这个能力就别画空壳）", () => {
      if (domW.window.document.querySelector("[data-quota-card]")) {
        throw new Error("WorkBuddy 页面出现了护栏卡");
      }
      domW.window.close();
      return true;
    });
  } catch (e) {
    /* 元素缺失/渲染中断也要落成一条失败，而不是让脚本崩掉——否则变异模式下
       只能看到「脚本异常」，读不出是哪条断言守住了。 */
    results.push({ name: "交互序列完整执行", ok: false, detail: e.message });
  }

  /* ── 报告 ── */
  const pass = results.filter((r) => r.ok).length;
  const fail = results.length - pass;
  for (const r of results) console.log(`${r.ok ? "  ok  " : " FAIL "} ${r.name}${r.detail && !r.ok ? "  → " + r.detail : ""}`);
  console.log(`\n${pass} passed / ${fail} failed`);

  if (MUTATE) {
    console.log(fail > 0 ? "\n变异测试通过：断言在缺陷下确实变红。" : "\n变异测试失败：断言空转，注入缺陷后仍全绿。");
    process.exit(fail > 0 ? 0 : 1);
  }
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("验证脚本自身异常：", e); process.exit(3); });
