/* 原型验收：随机抖动开关（issue #27 的另一支）。
 *
 * 立场与其他 verify-*.cjs 一致：**不信任自述**，直接开页面点开关、读真实 DOM 里的时刻。
 * 要证明的只有一件事 —— 「打开就抖动、关掉就准点」在原型里是**看得见**的，
 * 而不是一行说明文字。
 *
 * 用法：
 *   NODE_PATH=<managed workspace>/node_modules node docs/prototype/verify-jitter.cjs
 *   ... --file=buddy-switch-prototype.html   同一套断言跑单文件原型（两份原型不许漂移）
 *   ... --mutate=jitter                      注入「偏移恒为 0」，**期望测试变红**。
 */
const path = require("path");
const fs = require("fs");
const os = require("os");
const { JSDOM } = require("jsdom");

const SRC = __dirname;
const MUT = (process.argv.find((a) => a.startsWith("--mutate")) || "").replace("--mutate", "").replace(/^=/, "");
/* 默认跑多文件原型（index.html + demo-*.js/css）；`--file=` 可切到单文件原型。
   两份原型是同一套交互的两个载体，用同一份断言跑，才能发现「改了一边忘了另一边」。 */
const TARGET = (process.argv.find((a) => a.startsWith("--file=")) || "").slice(7) || "index.html";
const FILES = TARGET === "index.html" ? ["index.html", "demo.css", "demo-data.js", "demo-app.js"] : [TARGET];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const MUTATIONS = {
  jitter: {
    desc: "偏移恒为 0（模拟「开关只翻样式、不真的抖」）",
    apply: (dir) => {
      const from = "return ((h >>> 0) % (max * 2 + 1)) - max;";
      /* 多文件原型在 demo-app.js，单文件原型在同一份 html 里 —— 逐个文件找，
         找到才改，避免「变异没生效、测试却全绿」的假阴性。 */
      for (const f of FILES) {
        const file = path.join(dir, f);
        const src = fs.readFileSync(file, "utf8");
        if (!src.includes(from)) continue;
        fs.writeFileSync(file, src.replace(from, "return 0; /* MUTATED */"));
        return;
      }
      throw new Error("未匹配到 jitterOffset 的返回行");
    },
  },
};

/** 把原型复制到临时目录再改，交付目录下的原文件永不被写。 */
function prepareDir() {
  if (!MUT) return SRC;
  if (!MUTATIONS[MUT]) throw new Error("未知变异: " + MUT);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bs-jitter-"));
  for (const f of FILES) fs.copyFileSync(path.join(SRC, f), path.join(dir, f));
  MUTATIONS[MUT].apply(dir);
  return dir;
}

const results = [];
function check(name, fn) {
  try {
    const detail = fn();
    results.push({ name, ok: detail !== false, detail: detail === true || detail === undefined ? "" : String(detail) });
  } catch (e) {
    results.push({ name, ok: false, detail: e.message });
  }
}

const toMin = (hhmm) => {
  const [h, m] = hhmm.split(":").map(Number);
  return h * 60 + m;
};
/** 环形最短差值（跨零点时 07:52 相对 08:00 是 −8，不是 +1432）。 */
const delta = (base, t) => {
  const d = (((toMin(t) - toMin(base)) % 1440) + 1440) % 1440;
  return d > 720 ? d - 1440 : d;
};

(async () => {
  const dir = prepareDir();
  const dom = await JSDOM.fromFile(path.join(dir, TARGET), {
    runScripts: "dangerously", resources: "usable", pretendToBeVisual: true,
  });
  const { window } = dom;
  const doc = window.document;
  const $ = (s) => doc.querySelector(s);
  const $$ = (s) => [...doc.querySelectorAll(s)];
  const click = (s) => { const n = typeof s === "string" ? $(s) : s; if (!n) throw new Error("找不到元素: " + s); n.click(); };

  await new Promise((r) => window.addEventListener("load", r));
  await sleep(60);

  /** 抖动行（按 label 文本定位，避免依赖行序）。 */
  const jitterRow = () => $$(".srow").find((r) => (r.querySelector(".lab b") || {}).textContent?.includes("随机抖动"));
  const switchOn = () => jitterRow().querySelector('[data-act="jitter-toggle"]').classList.contains("on");
  const cells = () => $$(".jcell").map((c) => ({
    label: c.querySelector("i").textContent.trim(),
    time: c.querySelector("b").textContent.trim(),
    delta: c.querySelector("em").textContent.trim(),
  }));

  /* ── 进入 设置 → 签到与积分 ── */
  click('[data-act="nav|settings"]');
  await sleep(30);
  click('[data-act="set-sub|checkin"]');
  await sleep(30);

  check("「签到与积分」里存在「随机抖动」行", () => {
    if (!jitterRow()) throw new Error("未找到该行");
    return true;
  });

  check("默认关闭（不改变任何既有行为）", () => {
    if (switchOn()) throw new Error("默认就是开的");
    const s = jitterRow().querySelector(".lab > span").textContent;
    if (!s.includes("准点触发")) throw new Error("关闭态文案不对：" + s);
    return true;
  });

  check("关闭时预览三格都是准点 08:00", () => {
    const c = cells();
    if (c.length !== 3) throw new Error("预览格数 = " + c.length);
    for (const x of c) if (x.time !== "08:00") throw new Error(`${x.label} 是 ${x.time}`);
    return c.map((x) => x.time).join(" / ");
  });

  /* ── 打开 ── */
  click('[data-act="jitter-toggle"]');
  await sleep(30);

  check("点开后开关为 on", () => {
    if (!switchOn()) throw new Error("开关没变");
    return true;
  });

  check("点开后出现幅度分段器（±5 / ±15 / ±30）", () => {
    const segs = $$('[data-act^="jitter-range|"]');
    const want = ["±5 分", "±15 分", "±30 分"];
    const got = segs.map((b) => b.textContent.trim());
    if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error("实际 " + got.join(" / "));
    return got.join(" / ");
  });

  check("点开后预览真的抖了（至少一格不是准点）", () => {
    const c = cells();
    const moved = c.filter((x) => x.time !== "08:00");
    if (!moved.length) throw new Error("三格都还是 08:00 —— 开关只翻了样式");
    return c.map((x) => `${x.label} ${x.time}(${x.delta})`).join(" / ");
  });

  check("偏移都落在 ±15 分钟内且与标注一致", () => {
    for (const x of cells()) {
      const d = delta("08:00", x.time);
      if (Math.abs(d) > 15) throw new Error(`${x.label} 偏移 ${d} 超出 ±15`);
      const label = x.delta === "准点" ? 0 : Number(x.delta.replace(/[+−]\s*/, "").replace(" 分", "")) * (x.delta.startsWith("−") ? -1 : 1);
      if (label !== d) throw new Error(`${x.label} 标注 ${x.delta} 与实际 ${d} 不符`);
    }
    return true;
  });

  const before = cells().map((x) => x.time);

  check("同一配置重渲染后时刻不变（确定性）", () => {
    click('[data-act="nav|accounts"]');
    click('[data-act="nav|settings"]');
    click('[data-act="set-sub|checkin"]');
    const after = cells().map((x) => x.time);
    if (JSON.stringify(after) !== JSON.stringify(before)) {
      throw new Error(`重渲染前后不同：${before.join(",")} → ${after.join(",")}`);
    }
    return before.join(" / ");
  });

  /* ── 换幅度 ── */
  click('[data-act="jitter-range|30"]');
  await sleep(30);

  check("切到 ±30 后幅度生效且仍在界内", () => {
    const on = $$('[data-act^="jitter-range|"]').find((b) => b.classList.contains("on"));
    if (!on || on.textContent.trim() !== "±30 分") throw new Error("分段器未选中 ±30");
    for (const x of cells()) {
      const d = delta("08:00", x.time);
      if (Math.abs(d) > 30) throw new Error(`${x.label} 偏移 ${d} 超出 ±30`);
    }
    if (!jitterRow().querySelector(".lab > span").textContent.includes("±30 分钟")) {
      throw new Error("说明文案没跟着变");
    }
    return cells().map((x) => x.time).join(" / ");
  });

  check("幅度换回 ±15 后时刻与第一次完全一致（同组合恒定）", () => {
    click('[data-act="jitter-range|15"]');
    const after = cells().map((x) => x.time);
    if (JSON.stringify(after) !== JSON.stringify(before)) {
      throw new Error(`${before.join(",")} → ${after.join(",")}`);
    }
    return true;
  });

  /* ── 改「执行时刻」→ 预览基准跟随 ── */
  check("改执行时刻为 10:00，预览基准随之变", () => {
    const input = $('[data-bind="checkin-time"]');
    if (!input) throw new Error("找不到执行时刻输入框");
    input.value = "10:00";
    input.dispatchEvent(new window.Event("change", { bubbles: true }));
    const c = cells();
    for (const x of c) if (Math.abs(delta("10:00", x.time)) > 15) throw new Error(`${x.label} 未围绕 10:00：${x.time}`);
    return c.map((x) => x.time).join(" / ");
  });

  /* ── 关掉 ── */
  click('[data-act="jitter-toggle"]');
  await sleep(30);

  check("关掉后恢复准点、分段器消失", () => {
    if (switchOn()) throw new Error("开关没关");
    if ($$('[data-act^="jitter-range|"]').length) throw new Error("分段器还在");
    for (const x of cells()) if (x.time !== "10:00") throw new Error(`${x.label} 仍是 ${x.time}`);
    return true;
  });

  const failed = results.filter((r) => !r.ok);
  for (const r of results) {
    console.log(`${r.ok ? "✓" : "✗"} ${r.name}${r.detail ? `  ·  ${r.detail}` : ""}`);
  }
  console.log(`\n${results.length - failed.length}/${results.length} 通过`);
  if (MUT) {
    console.log(failed.length ? `变异「${MUT}」被检出（符合预期：断言不空转）` : `⚠️ 变异「${MUT}」未被检出 —— 断言在空转`);
    process.exit(failed.length ? 0 : 1);
  }
  process.exit(failed.length ? 1 : 0);
})();
