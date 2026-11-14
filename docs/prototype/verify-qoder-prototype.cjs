/* 单文件原型（buddy-switch-prototype.html）的 Qoder 实测。
 *
 * 为什么单独写一支：`verify-demo.cjs` / `verify-behaviors.cjs` 跑的是
 * `index.html` + 拆分文件，**覆盖不到单文件**；而单文件正是「双击即看」的交付物。
 * 这里直接加载单文件、驱动它，断言 Qoder 真的能用。
 *
 * ⚠️ 必须是 `.cjs`：本沙箱的 jsdom 装在隔离工作区，靠 `NODE_PATH` 找；
 * ESM 的 `import` **不认 `NODE_PATH`**（会 ERR_MODULE_NOT_FOUND）。
 *
 * 用法：NODE_PATH=<managed node workspace>/node_modules node verify-qoder-prototype.cjs
 */
const path = require("path");
const fs = require("fs");
const os = require("os");
const { JSDOM, VirtualConsole } = require("jsdom");

const BASE = __dirname;
const results = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/* ─────────── 变异注入 ───────────
 * 与 verify-demo.cjs / verify-behaviors.cjs 同一立场：**变异只写 os.tmpdir() 里的副本**，
 * 交付目录下的原文件永不被写；变异模式下**全绿即为失败**（说明断言打不红）。
 * ⚠️ 单文件是自包含的，所以只需要复制这一个文件。 */
const MUT = (process.argv.find((a) => a.startsWith("--mutate")) || "").replace("--mutate", "").replace(/^=/, "");
const MUTATIONS = {
  qoderregion: {
    desc: "Qoder 国际版被误标成国内版（区域标签错位）",
    from: '    id: "qoder-global", platform: "qoder", region: "global",',
    to: '    id: "qoder-global", platform: "qoder", region: "cn", /* MUTATED */',
  },
  uninamespace: {
    desc: "统一网关漏掉 Qoder 的两个命名空间（派生层没跟着平台长出来）",
    from: '  { id: "qoder-cn",       plat: "Qoder",     program: "Qoder 桌面版",   client: "Qoder CN",     fn: "—",              tier: "core" },\n'
        + '  { id: "qoder-global",   plat: "Qoder",     program: "Qoder 桌面版",   client: "Qoder",        fn: "—",              tier: "core" },\n',
    to: "",
  },
  backupname: {
    desc: "备份页作用域名退回 `SCOPES[id].label`（既有缺陷复现：每行都没名字）",
    from: "  label: scopeDisplayName(id),",
    to: "  label: SCOPES[id].label, /* MUTATED */",
  },
  /* ★ 限额护栏卡（2026-10-21）：池区直接顶上来、卡片不渲染。 */
  quotacard: {
    desc: "限额护栏卡未渲染（账号池区直接顶到分区顶部）",
    from: "  return quotaCardHtml(g) + `<div class=\"cat-bar\">",
    to: "  return `` + `<div class=\"cat-bar\"> /* MUTATED */",
  },
  /* ★ 顶栏刷新按钮的进行中反馈（2026-10-15）：去掉 doRefresh 的 spinning/禁用注入。 */
  refresh: {
    desc: "刷新按钮点击后无进行中反馈（去掉 spinning + 禁用）",
    from: '  if (btn) { btn.classList.add("spinning"); btn.disabled = true; }\n',
    to: "  /* MUTATED: 进行中反馈被移除 */\n",
  },
};

let DIR = BASE;
if (MUT) {
  if (!MUTATIONS[MUT]) {
    console.error("未知变异：" + MUT + "（可选 " + Object.keys(MUTATIONS).join(" / ") + "）");
    process.exit(2);
  }
  const m = MUTATIONS[MUT];
  DIR = fs.mkdtempSync(path.join(os.tmpdir(), "bsproto-"));
  const p = path.join(DIR, "buddy-switch-prototype.html");
  const src = fs.readFileSync(path.join(BASE, "buddy-switch-prototype.html"), "utf8");
  /* 锚点在 LF 归一化后的文本上匹配，写回还原原换行风格。 */
  const eol = src.includes("\r\n") ? "\r\n" : "\n";
  const norm = src.replace(/\r\n/g, "\n");
  if (!norm.includes(m.from)) { console.error("变异注入失败：锚点不匹配 → " + m.from.slice(0, 60)); process.exit(2); }
  fs.writeFileSync(p, norm.replace(m.from, m.to).replace(/\n/g, eol));
  console.log(`【变异注入】${m.desc}\n`);
}

const FILE = path.join(DIR, "buddy-switch-prototype.html");

function check(name, fn) {
  try {
    const r = fn();
    results.push({ name, ok: r !== false, detail: r === true || r === undefined ? "" : String(r) });
  } catch (e) {
    results.push({ name, ok: false, detail: e.message });
  }
}

async function boot(query) {
  const errors = [];
  const vc = new VirtualConsole();
  vc.on("jsdomError", (e) => errors.push(String(e && e.message ? e.message : e)));
  const dom = await JSDOM.fromFile(FILE, {
    runScripts: "dangerously",
    resources: "usable",
    pretendToBeVisual: true,
    virtualConsole: vc,
    url: "file://" + FILE.replace(/\\/g, "/") + (query ? "?" + query : ""),
  });
  await new Promise((r) => dom.window.addEventListener("load", r));
  await sleep(120);
  dom.__errors = errors;
  return dom;
}

const flat = (s) => String(s || "").replace(/\s+/g, " ").trim();

(async () => {
  /* ── 1. 作用域菜单 ── */
  const dom0 = await boot("hint=0");
  const d0 = dom0.window.document;
  check("单文件：脚本零异常加载", () => {
    if (dom0.__errors.length) throw new Error(dom0.__errors.slice(0, 2).join(" | "));
    return true;
  });
  d0.querySelector("#scope-trigger").click();
  /* ★ 只数**真作用域**（排除「聚合」入口）：聚合面板（issue #50）往作用域菜单加了一项，
     它与 6 个作用域不同性质 —— 聚合跨全部作用域，不是第 7 个作用域。
     原先用 `.scope-item` 计数，聚合一加进来就恒为 7 ⇒ 这条断言长期是红的。 */
  const scopeItems = [...d0.querySelectorAll("#scope-menu .scope-item:not(.agg-entry)")];
  check("单文件：作用域菜单 6 项（3 平台 × 2 区域）", () => {
    if (scopeItems.length !== 6) throw new Error("实际 " + scopeItems.length + " 项");
    return true;
  });
  check("单文件：「聚合」入口与作用域**不同类**（跨全部作用域，不是第 7 个）", () => {
    const agg = d0.querySelector("#scope-menu .scope-item.agg-entry");
    if (!agg) throw new Error("聚合入口不在作用域菜单里");
    if (!agg.textContent.includes("聚合")) throw new Error("聚合入口文案不含「聚合」：" + agg.textContent.trim());
    return true;
  });
  check("单文件：菜单含 Qoder 国内版 / 国际版", () => {
    const labels = scopeItems.map((b) => flat(b.textContent));
    for (const want of ["Qoder 国内版", "Qoder 国际版"]) {
      if (!labels.some((l) => l.includes(want))) throw new Error("缺少「" + want + "」：" + labels.join(" / "));
    }
    return "Qoder 国内版 / Qoder 国际版";
  });

  /* ── 2. Qoder 国内版：概览 ── */
  const domQ = await boot("scope=qoder-cn&dest=overview&hint=0");
  const dQ = domQ.window.document;
  check("单文件：切到 Qoder 国内版，顶栏跟随", () => {
    const t = flat(dQ.querySelector("#scope-name").textContent);
    if (t !== "Qoder 国内版") throw new Error("实际「" + t + "」");
    return true;
  });
  check("单文件：Qoder 概览显示当前登录账号与能力徽章", () => {
    const t = flat(dQ.querySelector("#content").textContent);
    if (!t.includes("jackey")) throw new Error("概览没显示当前登录账号：" + t.slice(0, 120));
    if (!t.includes("OAuth 登录")) throw new Error("缺少 OAuth 能力徽章");
    if (!t.includes("API 网关")) throw new Error("缺少 API 网关能力徽章");
    if (t.includes("自动旅行")) throw new Error("Qoder 不应出现「自动旅行」（那是 WorkBuddy 独有）");
    return true;
  });
  check("单文件：Qoder 侧栏无「会话」入口（无 sessionCopy 能力）", () => {
    const labels = [...dQ.querySelectorAll("#nav .nav-label")].map((n) => n.textContent.trim());
    if (labels.includes("会话")) throw new Error("Qoder 不应渲染「会话」入口：" + labels.join("/"));
    if (!labels.includes("网关")) throw new Error("缺少「网关」入口：" + labels.join("/"));
    return labels.join(" / ");
  });

  /* ── 3. Qoder 国内版：账号页 + 程序位 ── */
  const domA = await boot("scope=qoder-cn&dest=accounts&hint=0");
  const dA = domA.window.document;
  check("单文件：Qoder 账号页渲染 5 个账号卡", () => {
    const n = dA.querySelectorAll("#acct-list .acct").length;
    if (n !== 5) throw new Error("实际 " + n + " 张卡");
    return true;
  });
  check("单文件：程序位为 App / CLI；主号双绑并显示「使用中 2 个工具」", () => {
    /* ⚠️ **不能断言「只有一个账号带当前登录徽章」**：Qoder 有两个程序位
       （桌面版 / CLI），两个**不同**账号分别登录两个客户端是正常状态
       —— 徽章判据是「绑定了任一程序位」。这里钉的是夹具事实：
       q1 绑 App+CLI、q2 绑 CLI ⇒ 恰好 2 张卡带徽章，且 q1 出现「使用中 2 个工具」。 */
    const cards = [...dA.querySelectorAll("#acct-list .acct")];
    const progs = [...dA.querySelectorAll("#acct-list .acct .prog")].map((b) => b.textContent.trim());
    for (const want of ["App", "CLI"]) {
      if (!progs.some((p) => p.includes(want))) throw new Error("缺少程序位 " + want + "：" + progs.join("/"));
    }
    const cur = cards.filter((c) => c.classList.contains("is-current")).length;
    if (cur !== 2) throw new Error(`带「当前登录」的卡有 ${cur} 张（期望 2：q1 绑 App+CLI、q2 绑 CLI）`);
    const jackey = cards.find((c) => c.querySelector(".acct-id b").textContent.trim() === "jackey");
    if (!jackey) throw new Error("找不到 jackey 的卡片");
    if (!/使用中 2 个工具/.test(flat(jackey.textContent))) {
      throw new Error("jackey 卡没有「使用中 2 个工具」：" + flat(jackey.textContent).slice(0, 140));
    }
    return "程序位 App / CLI · 带徽章 2 张（q1 双绑、q2 单绑）";
  });

  /* ── 3-b. Qoder 网关「限额护栏」（2026-10-21）：与 App 的 `QoderQuotaGuard` 同构 ── */
  const domP = await boot("scope=qoder-cn&dest=gateway&gw=pool&hint=0");
  const dP = domP.window.document;
  const quotaCard = dP.querySelector("[data-quota-card]");
  check("单文件：账号池顶部有「限额护栏」卡，四个阈值输入齐全", () => {
    if (!quotaCard) throw new Error("没有 [data-quota-card]");
    const keys = [...quotaCard.querySelectorAll("[data-quota]")].map((i) => i.dataset.quota);
    for (const k of ["reserve", "daily", "credit", "model"]) {
      if (!keys.includes(k)) throw new Error("缺输入 " + k + "：" + keys.join("/"));
    }
    return keys.join(" / ");
  });
  check("单文件：护栏卡排在账号池表**之前**（它是池的上游条件）", () => {
    const html = dP.querySelector("#content").innerHTML;
    const a = html.indexOf("data-quota-card");
    const b = html.indexOf('data-act="pin|');
    if (a < 0 || b < 0 || a > b) throw new Error(`顺序不对（卡 ${a} / 池表 ${b}）`);
    return true;
  });
  check("单文件：四种拦截态各占一个账号，且都给了可读原因（title 非空）", () => {
    const blocked = [...quotaCard.querySelectorAll(".badge")].filter(
      (b) => b.classList.contains("warn") || b.classList.contains("bad"),
    );
    if (blocked.length !== 3) throw new Error("被拦徽标 " + blocked.length + " 个（夹具应有 3 态）");
    for (const b of blocked) {
      if (!(b.getAttribute("title") || "").trim()) throw new Error("徽标缺原因：" + b.textContent);
    }
    const labels = blocked.map((b) => b.textContent.trim()).sort().join(" / ");
    return labels;
  });
  check("单文件：保存会**真的改数据**（不是只弹 toast）", () => {
    const input = quotaCard.querySelector('[data-quota="daily"]');
    input.value = "1234";
    quotaCard.querySelector('[data-act="quota-save"]').click();
    const again = dP.querySelector('[data-quota="daily"]');
    if (!again || again.value !== "1234") throw new Error("保存后回填值 = " + (again && again.value));
    return true;
  });
  /* 国际版夹具是「四条都 0」⇒ 文案必须变成「未启用」，不能还写「计数来自记账」。 */
  const domP2 = await boot("scope=qoder-global&dest=gateway&gw=pool&hint=0");
  check("单文件：国际版护栏默认关（四条 0）⇒ 文案说「未启用」", () => {
    const card = domP2.window.document.querySelector("[data-quota-card]");
    if (!card) throw new Error("国际版没有护栏卡（切区域后不该消失）");
    const t = flat(card.textContent);
    if (!t.includes("未启用")) throw new Error("四阈值全 0 却没提示「未启用」：" + t.slice(0, 120));
    return true;
  });
  /* WorkBuddy 没有这套护栏 ⇒ 池区**不得**出现空壳卡。 */
  const domW = await boot("scope=wb-cn&dest=gateway&gw=pool&hint=0");
  check("单文件：WorkBuddy 账号池**不渲染**护栏卡（没这个能力就别画空壳）", () => {
    if (domW.window.document.querySelector("[data-quota-card]")) throw new Error("WorkBuddy 出现了护栏卡");
    return true;
  });

  /* ── 3-c. 表格视图的行尾「更多」（与卡片同一个菜单，含签到三态）── */
  const domT = await boot("scope=qoder-cn&dest=accounts&view=table&hint=0");
  const dT = domT.window.document;
  check("单文件：表格视图每行都有行尾「更多」，点开是同一个菜单（含签到三态）", () => {
    const rows = dT.querySelectorAll("#content tbody tr").length;
    const mores = [...dT.querySelectorAll('#content tbody [data-act^="acct-more|"]')];
    if (!rows || mores.length !== rows) throw new Error(`行 ${rows} 个、更多按钮 ${mores.length} 个`);
    mores[0].click();
    const menu = dT.getElementById("acct-menu");
    if (!menu) throw new Error("菜单没打开");
    const t = flat(menu.textContent);
    if (!t.includes("导出账号")) throw new Error("菜单缺「导出账号」：" + t.slice(0, 120));
    const states = ["今日已签到", "手动签到", "需先登录"].filter((w) => t.includes(w));
    if (states.length !== 1) throw new Error("签到三态命中 " + states.length + " 个：" + t.slice(0, 120));
    return `${rows} 行 · 签到态 ${states[0]}`;
  });

  /* ── 4. Qoder 国际版：用量 + 网关 ── */
  const domU = await boot("scope=qoder-global&dest=usage&hint=0");
  const dU = domU.window.document;
  check("单文件：Qoder 国际版用量页有视图参数条与四个子 Tab", () => {
    const t = flat(dU.querySelector("#content").textContent);
    if (!t.includes("视图参数")) throw new Error("缺少视图参数条");
    for (const k of ["趋势", "模型", "会话", "积分"]) if (!t.includes(k)) throw new Error("缺少子 Tab：" + k);
    return true;
  });
  /* ★ Qoder 的单位是 credits（客户端 token 计数恒为 0）—— 原型必须如实标注，
     否则评审会把「Token」当成 Qoder 的口径。 */
  check("单文件：Qoder 用量页标注「按 credits 计费」（不把 Token 当口径）", () => {
    const t = flat(dU.querySelector("#content").textContent);
    if (!t.includes("按 credits 计费")) throw new Error("缺少单位说明：" + t.slice(0, 160));
    if (!t.includes("余额快照")) throw new Error("「积分」视图没说清是余额快照");
    return true;
  });
  const domG = await boot("scope=qoder-cn&dest=gateway&gw=keys&hint=0");
  const dG = domG.window.document;
  check("单文件：Qoder 网关密钥表含「已吊销」行且同时给「恢复 + 删除」", () => {
    const t = flat(dG.querySelector("#content").textContent);
    if (!t.includes("已吊销")) throw new Error("密钥表没有已吊销行");
    const acts = [...dG.querySelectorAll("#content [data-act]")].map((e) => e.dataset.act);
    if (!acts.some((a) => a.startsWith("key-restore|"))) throw new Error("缺「恢复」出口");
    if (!acts.some((a) => a.startsWith("key-delete|"))) throw new Error("缺「删除」出口");
    return "已吊销 → 恢复 + 删除";
  });

  /* ── 5. 统一网关（单文件独有）也要认得 Qoder ──
     ★ 2026-10-15 第三轮：它已从「网关页子导航第 7 项」升为**独立目的地**（`dest=unified`）。
     旧深链 `dest=gateway&gw=unified` 的兼容性由 `verify-unified-pool.cjs` 单独钉住。 */
  const domN = await boot("scope=wb-cn&dest=unified&hint=0");
  const dN = domN.window.document;
  check("单文件：统一网关的命名空间表含 Qoder 两条（派生层）", () => {
    const t = flat(dN.querySelector("#content").textContent);
    for (const want of ["qoder-cn", "qoder-global"]) {
      if (!t.includes(want)) throw new Error("统一网关没有命名空间 " + want);
    }
    const n = domN.window.eval("UNI_NS.length");
    if (n !== 10) throw new Error(`UNI_NS ${n} 条，期望 10（6 派生 + 4 扩展）`);
    return "UNI_NS 10 条（含 qoder-cn / qoder-global）";
  });

  /* ── 6. 备份页 6 行且都有名字（既有缺陷的修复） ── */
  const domB = await boot("dest=settings&set=backup&hint=0");
  const dB = domB.window.document;
  check("单文件：备份页 6 行作用域，且每行都有名字", () => {
    const rows = [...dB.querySelectorAll(".bk-export-list .bk-row")];
    if (rows.length !== 6) throw new Error(rows.length + " 行，期望 6 行");
    const labels = rows.map((r) => flat(r.querySelector(".bk-scope").textContent));
    const blank = labels.filter((l) => !l);
    if (blank.length) throw new Error(blank.length + "/6 行没有名字");
    for (const want of ["WorkBuddy 国内版", "Qoder 国内版", "Qoder 国际版"]) {
      if (!labels.includes(want)) throw new Error("缺少「" + want + "」：" + labels.join(" / "));
    }
    return labels.join(" / ");
  });

  /* ── 7. ⌘5 快捷键能切到 Qoder ── */
  const domK = await boot("hint=0");
  const dK = domK.window.document;
  const ev = new domK.window.KeyboardEvent("keydown", { key: "5", metaKey: true, bubbles: true });
  dK.dispatchEvent(ev);
  await sleep(400);
  check("单文件：⌘5 切到 Qoder 国内版（快捷键已放宽到 1-9）", () => {
    const t = flat(dK.querySelector("#scope-name").textContent);
    if (t !== "Qoder 国内版") throw new Error("实际「" + t + "」");
    return true;
  });

  /* ── 8. 顶栏刷新：进行中反馈（与 verify-demo.cjs 的「刷新」三条断言同口径，两形态各守一份） ── */
  const domR = await boot("hint=0");
  const dR = domR.window.document;
  const refreshBtn = dR.querySelector("#refresh-btn");
  refreshBtn.click();
  await sleep(50);
  check("单文件：刷新点击后按钮进入进行中态（spinning + disabled）", () => {
    if (!refreshBtn.classList.contains("spinning")) throw new Error("缺少 spinning（点击无动效）");
    if (!refreshBtn.disabled) throw new Error("未 disabled（可重复点击）");
    return true;
  });
  await sleep(700);
  check("单文件：刷新完成后复原（不再 spinning / 不再禁用）", () => {
    if (refreshBtn.classList.contains("spinning")) throw new Error("spinning 残留未移除");
    if (refreshBtn.disabled) throw new Error("完成后仍处于禁用");
    return true;
  });
  check("单文件：刷新完成 toast 标题为「已刷新」（精确匹配标题，防批量条文案冒充）", () => {
    const titles = [...dR.querySelectorAll("#toasts .toast b")].map((b) => b.textContent.trim());
    if (!titles.includes("已刷新")) throw new Error("toast 标题：" + titles.join(" / "));
    return true;
  });

  /* ── 报告 ── */
  const pass = results.filter((r) => r.ok).length;
  const fail = results.length - pass;
  for (const r of results) console.log(`${r.ok ? "  ok  " : " FAIL "} ${r.name}${r.detail && !r.ok ? "  → " + r.detail : ""}`);
  console.log(`\n${pass} passed / ${fail} failed`);

  if (MUT) {
    console.log(fail > 0 ? "\n变异测试通过：断言在缺陷下确实变红。" : "\n变异测试失败：断言空转，注入缺陷后仍全绿。");
    process.exit(fail > 0 ? 0 : 1);
  }
  process.exit(fail === 0 ? 0 : 1);
})();
