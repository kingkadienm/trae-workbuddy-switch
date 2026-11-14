/* 用量页「视图参数 → 今天 / 自定义时间」的**双形态**实测。
 *
 * ## 为什么单独写一支
 *
 * `verify-demo.cjs` / `verify-behaviors.cjs` 跑拆分形态（`index.html` + `demo-*.js`），
 * `verify-qoder-prototype.cjs` 跑单文件 —— 三支都只断言「参数条存在」这类**文案**判据，
 * 没有一支回答「窗口到底取没取对日期」。本支补的是**数据形状**判据：
 * 堆叠柱列数必须**逐字等于**窗口天数，且自定义是**锚在日期上**的一段（末列不是今天）。
 *
 * 这三个判据都是「删掉被测逻辑它会红」的类型：
 * - 把「今天」当回老档位 ⇒ 末列不是今天 / 列数不是 1；
 * - 把自定义退化成「最后 N 天」⇒ 末列变成今天、数值与「近 7 天」不再相等。
 *    注意**只改 `rangeWindow` 不够**：派发里还写了 `state.range`，
 *    preset 回落可能**恰好**凑出同样结果 ⇒ 变异必须打真能改变行为的点（2026-10-14 踩过）。
 *
 * ⚠️ 必须是 `.cjs`：本沙箱的 jsdom 装在隔离工作区，靠 `NODE_PATH` 找；
 *    ESM 的 `import` **不认 `NODE_PATH`**（会 ERR_MODULE_NOT_FOUND）。
 *
 * 用法：
 *   NODE_PATH=<managed node workspace>/node_modules node docs/prototype/verify-usage-window.cjs
 *   ... --mutate=todayfit | customfit | customgone      （期望用例变红）
 */
const path = require("path");
const fs = require("fs");
const os = require("os");
const { JSDOM, VirtualConsole } = require("jsdom");

const BASE = __dirname;
const results = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/* ─────────── 变异注入 ───────────
 * 同 `verify-qoder-prototype.cjs`：**只写 os.tmpdir() 里的副本**，交付目录下的原文件永不被写；
 * 变异模式下**全绿即为失败**（说明断言打不红）。
 * 两形态的这段代码逐字相同 ⇒ 同一组 `from/to` 同时打 `demo-app.js` 与单文件。 */
const MUT = (process.argv.find((a) => a.startsWith("--mutate")) || "").replace("--mutate", "").replace(/^=/, "");
const MUTATIONS = {
  todayfit: {
    desc: "「今天」被当回老档位（天数拟合：不设 rangeMode，按 Number(a1) 解析）",
    from: [
      '      if (a1 === "today") { state.rangeMode = "today"; state.range = 1; }',
      "      else { state.rangeMode = \"preset\"; state.range = Number(a1); }",
    ].join("\n"),
    to: '      state.rangeMode = "preset"; state.range = Number(a1) || 30; /* MUTATED: 今天退回老档位 */',
  },
  customfit: {
    desc: "自定义退化成「最后 N 天」（不看起止日期，只数天数）",
    from: [
      '  if (state.rangeMode === "custom") {',
      "    let i = dayIndexOf(mdOfIso(state.customFrom));",
      "    let j = dayIndexOf(mdOfIso(state.customTo));",
      "    if (i < 0) i = 0;",
      "    if (j < 0) j = len - 1;",
      "    if (j < i) { const t = i; i = j; j = t; }",
      "    return { from: i, to: j };",
      "  }",
    ].join("\n"),
    to: [
      '  if (state.rangeMode === "custom") {',
      "    /* MUTATED: 退化成「最后 30 天」——用天数拟合的典型症状 */",
      "    return { from: Math.max(0, len - 30), to: len - 1 };",
      "  }",
    ].join("\n"),
  },
  customgone: {
    desc: "「自定义时间」那一档整个消失（还原成改造前的三档）",
    from:
      '      <button class="${state.rangeMode === "custom" ? "on" : ""}" data-act="range-custom-open" aria-haspopup="dialog" aria-expanded="${state.rangeOpen}">自定义时间</button>\n',
    to: "",
  },
};

let DIR = BASE;
if (MUT) {
  if (!MUTATIONS[MUT]) {
    console.error("未知变异：" + MUT + "（可选 " + Object.keys(MUTATIONS).join(" / ") + "）");
    process.exit(2);
  }
  const m = MUTATIONS[MUT];
  DIR = fs.mkdtempSync(path.join(os.tmpdir(), "bsuw-"));
  /* 拆分形态要相对加载三个 js/css ⇒ 整目录复制；单文件自包含但一起复制也无妨。 */
  for (const f of fs.readdirSync(BASE)) fs.copyFileSync(path.join(BASE, f), path.join(DIR, f));
  /* 两形态里这段代码逐字相同 ⇒ 都打。 */
  for (const f of ["demo-app.js", "buddy-switch-prototype.html"]) {
    const p = path.join(DIR, f);
    const src = fs.readFileSync(p, "utf8");
    const eol = src.includes("\r\n") ? "\r\n" : "\n";
    const norm = src.replace(/\r\n/g, "\n");
    if (!norm.includes(m.from)) {
      console.error("变异注入失败（" + f + "）：锚点不匹配 → " + m.from.slice(0, 60));
      process.exit(2);
    }
    fs.writeFileSync(p, norm.replace(m.from, m.to).replace(/\n/g, eol));
  }
  console.log(`【变异注入】${m.desc}\n`);
}

function check(name, fn) {
  try {
    const r = fn();
    results.push({ name, ok: r !== false, detail: r === true || r === undefined ? "" : String(r) });
  } catch (e) {
    results.push({ name, ok: false, detail: e.message });
  }
}

/* 与 `demo-data.js` 的 `series()` 同源：offset 0 = 今天，越大越早。 */
const pad = (n) => String(n).padStart(2, "0");
const dOf = (o) => new Date(Date.now() - o * 86400000);
const isoOf = (o) => `${dOf(o).getFullYear()}-${pad(dOf(o).getMonth() + 1)}-${pad(dOf(o).getDate())}`;
const mdOf = (o) => `${dOf(o).getMonth() + 1}/${dOf(o).getDate()}`;
const dayNumOf = (o) => String(dOf(o).getDate());

async function boot(htmlFile) {
  const errors = [];
  const vc = new VirtualConsole();
  vc.on("jsdomError", (e) => errors.push(String(e && e.message ? e.message : e)));
  const file = path.join(DIR, htmlFile);
  const dom = await JSDOM.fromFile(file, {
    runScripts: "dangerously",
    resources: "usable",
    pretendToBeVisual: true,
    virtualConsole: vc,
    url: "file://" + file.replace(/\\/g, "/") + "?hint=0",
  });
  await new Promise((r) => dom.window.addEventListener("load", r));
  await sleep(150);
  dom.__errors = errors;
  return dom;
}

const $ = (w, s) => w.document.querySelector(s);
const $$ = (w, s) => [...w.document.querySelectorAll(s)];
const click = (w, node) => node && node.click();

/* 原型 `fmtTok()` 的取值形态（K / M / B 后缀）—— 比较数值必须先统一量纲，
   否则 `128.7K` vs `2.8M` 会被读成 128.7 < 2.8（**假失败**）。 */
const num = (text) => {
  const m = String(text || "").replace(/,/g, "").match(/([\d.]+)\s*([KMB])?/);
  if (!m) return NaN;
  const scale = m[2] === "K" ? 1e3 : m[2] === "M" ? 1e6 : m[2] === "B" ? 1e9 : 1;
  return Number(m[1]) * scale;
};

/**
 * 跑完一个形态的全部用例。
 *
 * ⚠️ 整个序列包在 try 里：变异模式下若某一步取不到元素（如「自定义时间」那一档被删掉了），
 * 要把**剩下的断言**也记成失败而不是让脚本崩掉 —— 否则只看到「脚本异常」，
 * 读不出是哪条断言守住了（同 `verify-demo.cjs` 的取舍）。
 */
async function runForm(htmlFile, label) {
  const dom = await boot(htmlFile);
  const w = dom.window;

  check(`${label}：脚本零异常加载`, () => {
    if (dom.__errors.length) throw new Error(dom.__errors.slice(0, 2).join(" | "));
    return true;
  });

  /* ── 控件集 ── */
  click(w, $(w, '[data-act="nav|usage"]'));
  await sleep(60);

  const segOf = () => (w.document.querySelector('[data-act="range|today"]') || {}).parentNode || null;
  const btnsOf = () => {
    const seg = segOf();
    return seg ? [...seg.querySelectorAll("button")].map((b) => b.textContent.trim()) : [];
  };
  const pressedOf = () => {
    const seg = segOf();
    return seg ? [...seg.querySelectorAll("button.on")].map((b) => b.textContent.trim()) : [];
  };
  const firstStat = () => ($$(w, "#content .card.stat")[0] || null);
  const valueOf = () => { const n = firstStat(); return n ? n.querySelector(".v").textContent.trim() : ""; };
  const hintOf = () => { const n = firstStat(); const d = n && n.querySelector(".d"); return d ? d.textContent.trim() : ""; };
  const colsOf = () => $$(w, "#content .cbar .ccol");
  const lastColLabel = () => {
    const c = colsOf();
    const em = c.length && c[c.length - 1].querySelector("em");
    return em ? em.textContent.trim() : "";
  };

  check(`${label}：参数条 5 档，文案逐字一致`, () => {
    const b = btnsOf();
    const want = ["今天", "近 7 天", "近 14 天", "近 30 天", "自定义时间"];
    if (JSON.stringify(b) !== JSON.stringify(want)) throw new Error("实得 " + JSON.stringify(b));
    return true;
  });

  /* 默认窗口：原型与项目已**统一到 30 天**（2026-10-15；判据与跨仓断言见
     `scripts/check-scope-registry.cjs` 的「默认窗口：原型与项目必须同一个数」）。
     ⚠️ 这里**刻意写死 30**：统一之后若任一侧又漂回 7 / 14，这条与项目侧那条会**同时红**
     —— 恒等式必须被钉住，而不是再留一个「两边都别写死」的口子。 */
  check(`${label}：默认档 = 近 30 天，30 列，末列是今天`, () => {
    const p = pressedOf().join("");
    if (p !== "近 30 天") throw new Error("默认选中「" + p + "」，期望「近 30 天」——原型/项目默认窗口分叉了？");
    const c = colsOf();
    if (c.length !== 30) throw new Error("列数实得 " + c.length);
    if (lastColLabel() !== dayNumOf(0)) throw new Error("末列实得 " + lastColLabel() + "，期望今天 " + dayNumOf(0));
    return true;
  });
  const total30 = valueOf();

  try {
    /* ── 今天 ── */
    click(w, w.document.querySelector('[data-act="range|today"]'));
    await sleep(40);
    check(`${label}：点「今天」→ 选中态与文案都变今天`, () => {
      if (pressedOf().join("") !== "今天" || hintOf() !== "今天") throw new Error(`选中=${pressedOf().join("")} hint=${hintOf()}`);
      return true;
    });
    const todayVal = valueOf();
    check(`${label}：★ 今天 → 只剩 1 列，且末列是今天`, () => {
      const c = colsOf();
      if (c.length !== 1) throw new Error("列数实得 " + c.length);
      if (lastColLabel() !== dayNumOf(0)) throw new Error("末列实得 " + lastColLabel());
      return true;
    });
    check(`${label}：今天的数值真的收敛（今天 < 近 30 天）`, () => {
      const a = num(todayVal);
      const b = num(total30);
      if (!(a > 0 && a < b)) throw new Error(`${todayVal} vs ${total30}`);
      return true;
    });

    /* ── 近 7 天（自定义要对拍的基准） ── */
    click(w, w.document.querySelector('[data-act="range|7"]'));
    await sleep(40);
    const w7 = valueOf();
    check(`${label}：★ 近 7 天 → 7 列，末列是今天`, () => {
      const c = colsOf();
      if (c.length !== 7) throw new Error("列数实得 " + c.length);
      if (lastColLabel() !== dayNumOf(0)) throw new Error("末列实得 " + lastColLabel());
      return true;
    });

    /* ── 自定义弹层 ── */
    click(w, w.document.querySelector('[data-act="range-custom-open"]'));
    await sleep(40);
    check(`${label}：点「自定义时间」→ 弹层出现，含起止两个日期输入`, () => {
      if (!$ (w, ".range-pop")) throw new Error("弹层未出现");
      const d = $$(w, ".range-pop input[type=date]");
      if (d.length !== 2) throw new Error("日期输入实得 " + d.length);
      return true;
    });
    check(`${label}：弹层「可选范围」提示是**两个独立日期**（不是 x – x – y）`, () => {
      const hint = $(w, ".range-pop .rp-hint");
      if (!hint) throw new Error("没有可选范围提示");
      const t = hint.textContent.trim();
      if (t.split("–").length !== 2) throw new Error("破折号实得 " + (t.split("–").length - 1) + " 段：" + t);
      return true;
    });

    /* ── 自定义 7 天（今天-6..今天）⇒ 与「近 7 天」必须逐字相同 ── */
    (() => {
      const d = $$(w, ".range-pop input[type=date]");
      d[0].value = isoOf(6);
      d[1].value = isoOf(0);
    })();
    click(w, [...$$(w, ".range-pop button")].find((b) => b.textContent.trim() === "应用"));
    await sleep(60);
    const customVal = valueOf();
    check(`${label}：自定义 7 天 → 7 列`, () => {
      const c = colsOf();
      if (c.length !== 7) throw new Error("列数实得 " + c.length);
      return true;
    });
    check(`${label}：自定义 = 近 7 天那一段 ⇒ **数值逐字相同**（两条 UI 路径同源）`, () => {
      if (customVal !== w7) throw new Error("自定义=" + customVal + " vs 近7天=" + w7);
      return true;
    });
    check(`${label}：应用后弹层关闭`, () => {
      if ($(w, ".range-pop")) throw new Error("弹层仍在");
      return true;
    });

    /* ── 锚在过去的 3 天（表达不了用天数拟合的形态） ── */
    click(w, w.document.querySelector('[data-act="range-custom-open"]'));
    await sleep(40);
    (() => {
      const d = $$(w, ".range-pop input[type=date]");
      d[0].value = isoOf(10);
      d[1].value = isoOf(8);
    })();
    click(w, [...$$(w, ".range-pop button")].find((b) => b.textContent.trim() === "应用"));
    await sleep(60);
    const pastHint = hintOf();
    check(`${label}：★ 过去那 3 天 → 恰好 3 列，**末列不是今天**`, () => {
      const c = colsOf();
      if (c.length !== 3) throw new Error("列数实得 " + c.length + "（天数拟合会给「最后 N 天」）");
      if (lastColLabel() === dayNumOf(0)) throw new Error("末列落到了今天 —— 已退化成「最后 3 天」");
      return true;
    });
    check(`${label}：过去那 3 天的文案就是所选那两天`, () => {
      const want = `${mdOf(10)} – ${mdOf(8)}`;
      if (pastHint !== want) throw new Error("实得「" + pastHint + "」，期望「" + want + "」");
      return true;
    });

    /* ── 点外部关闭，且不改窗口 ── */
    click(w, w.document.querySelector('[data-act="range-custom-open"]'));
    await sleep(40);
    click(w, $(w, "#content h1"));
    await sleep(40);
    check(`${label}：重开弹层后点页面外部 → 收起且窗口不变`, () => {
      if ($(w, ".range-pop")) throw new Error("弹层未收起");
      if (hintOf() !== pastHint) throw new Error("窗口被外部点击改掉了：「" + hintOf() + "」");
      return true;
    });

    /* ── 切子 Tab 不重置 ── */
    click(w, w.document.querySelector('[data-act="usage-tab|models"]'));
    await sleep(50);
    click(w, w.document.querySelector('[data-act="usage-tab|trend"]'));
    await sleep(50);
    check(`${label}：切子 Tab（模型↔趋势）后窗口**不重置**`, () => {
      if (hintOf() !== pastHint) throw new Error("实得「" + hintOf() + "」，期望仍为「" + pastHint + "」");
      return true;
    });
  } catch (e) {
    /* 元素缺失 / 渲染中断 ⇒ 剩下的断言无从执行，把整段记为失败（含原因）。 */
    if (e && e.message === "Cannot read properties of null (click)") {
      results.push({ name: `${label}：交互序列完整执行`, ok: false, detail: "某一步取不到元素（多半是被变异删掉了控件）" });
    } else {
      results.push({ name: `${label}：交互序列完整执行`, ok: false, detail: String(e && e.message ? e.message : e) });
    }
  }

  w.close();
}

(async () => {
  await runForm("index.html", "拆分形态");
  await runForm("buddy-switch-prototype.html", "单文件");

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
})().catch((e) => { console.error("验证脚本自身异常：", e); process.exit(3); });
