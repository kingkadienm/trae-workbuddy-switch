#!/usr/bin/env node
/**
 * `aggregateByKeyDim` / `keyCoverage` 的**真实断言**（issue #76 / 规范 U4）。
 *
 * ## 为什么是脚本而不是 vitest
 *
 * 项目**没有前端测试框架**（`package.json` 没有 `test` 脚本，质量靠 `check:*` 门禁）。
 * 为一个纯函数引入 vitest + jsdom + CI 改造，成本远大于收益。
 *
 * 这里用**已有的** esbuild（vite 的依赖，无需新增）把 `src/lib/key-usage.ts`
 * 现场转译成 ESM、经 `data:` URL 导入，跑真断言 —— 测的是**生产代码本身**，
 * 不是复制一份逻辑（复制的那份必然与生产漂移）。
 *
 * ★ 2026-10-28：数据源从「请求日志（环形 200 条）」换成「网关账本的 keys 维度」，
 * 断言随之改为按**逐日维度项**喂入 —— 顺带把「窗口过滤」与「无该维度的天」也钉住。
 *
 * 用法：`node scripts/check-key-usage.mjs`（已挂进 `npm run check:scope`）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = readFileSync(path.join(root, "src/lib/key-usage.ts"), "utf8");

// ★ Windows：绝对路径必须转 `file://` URL，否则 ESM loader 把 `D:` 当成协议。
const { transform } = await import(
  pathToFileURL(path.join(root, "node_modules/esbuild/lib/main.js")).href
);
const { code } = await transform(source, { loader: "ts", format: "esm" });
const mod = await import(
  `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`
);
const {
  aggregateByKeyDim,
  keyCoverage,
  firstKeyDay,
  splitBucket,
  LEGACY_KEY_LABEL,
  NO_KEY_LABEL,
  LEGACY_BUCKET,
  NONE_BUCKET,
} = mod;

let failed = 0;
let passed = 0;
function check(name, fn) {
  try {
    fn();
    passed += 1;
  } catch (error) {
    failed += 1;
    console.error(`  ✗ ${name}\n    ${error.message}`);
  }
}
function eq(actual, expected, hint) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) throw new Error(`${hint ? hint + "：" : ""}期望 ${e}，实际 ${a}`);
}

const names = new Map([
  ["k1", "脚本用的 key"],
  ["k2", "CI 用的 key"],
]);

/** 造一天的维度记录（`entries` 是 `[桶键, input, output, requests, credit]`）。 */
const dayOf = (key, rows, keysPartial = false) => ({
  key,
  keysPartial,
  keys: rows.map(([bucket, input, output, requests, credit]) => ({
    key: bucket,
    input,
    output,
    total: input + output,
    requests,
    failed: 0,
    credit,
  })),
});
const set = (...keys) => new Set(keys);

/* ── ① 两种「没有」必须分开 ───────────────────────────────────────── */
check("老条目（升级前无 keyId）与无 Key 调用分开成两行", () => {
  const days = [
    dayOf("2026-10-28", [
      [LEGACY_BUCKET, 10, 5, 1, 0],
      [NONE_BUCKET, 1, 1, 1, 0],
    ]),
  ];
  const rows = aggregateByKeyDim(days, set("2026-10-28"), names, "wb");
  eq(rows.length, 2, "应有两行（合并会让历史数据凭空多出「匿名调用」）");
  const legacy = rows.find((row) => row.name === LEGACY_KEY_LABEL);
  const none = rows.find((row) => row.name === NO_KEY_LABEL);
  if (!legacy) throw new Error("缺「升级前的历史条目」行");
  if (!none) throw new Error("缺「无密钥调用」行");
  eq(legacy.tokens, 15);
  eq(none.tokens, 2);
});

/* ── ② 同一 id 在两套台账里是两把 Key ───────────────────────────── */
check("同一 keyId 不同 keyScope 分成两行，且跨台账标出归属", () => {
  const days = [
    dayOf("2026-10-28", [
      ["wb:same", 10, 0, 1, 0],
      ["unified:same", 20, 0, 1, 0],
    ]),
  ];
  const rows = aggregateByKeyDim(days, set("2026-10-28"), names, "wb");
  eq(rows.length, 2, "不同台账的同名 id 不能并成一行");
  const cross = rows.find((row) => row.name.includes("统一网关"));
  if (!cross) throw new Error("跨台账的 Key 必须标出「统一网关」而不是拿短 id 冒充本地 Key");
  eq(cross.tokens, 20);
});

/* ── ③ 跨日累加与排序 ────────────────────────────────────────────── */
check("跨日累加到同一行，且 token = input + output", () => {
  const days = [
    dayOf("2026-10-27", [["wb:k1", 100, 50, 1, 0.5]]),
    dayOf("2026-10-28", [["wb:k1", 1, 1, 1, 0], ["wb:k2", 10, 5, 1, 0]]),
  ];
  const rows = aggregateByKeyDim(days, set("2026-10-27", "2026-10-28"), names, "wb");
  eq(rows[0].name, "脚本用的 key", "token 多的排前面");
  eq(rows[0].tokens, 152, "两天必须累加到同一行");
  eq(rows[0].requests, 2);
  eq(rows[1].tokens, 15);
});

check("token 全 0（业务代理）时按请求数降序", () => {
  const days = [
    dayOf("2026-10-28", [
      ["wb:k1", 0, 0, 1, 0],
      ["wb:k2", 0, 0, 2, 0],
    ]),
  ];
  const rows = aggregateByKeyDim(days, set("2026-10-28"), names, "wb");
  eq(rows[0].name, "CI 用的 key", "请求数多的排前面");
  eq(rows[0].requests, 2);
});

/* ── ④ 窗口过滤（本次换源的核心：不是「最近 200 条」而是「这个窗口」）── */
check("窗口外的日期一条都不算", () => {
  const days = [
    dayOf("2026-09-01", [["wb:k1", 999, 0, 1, 0]]),
    dayOf("2026-10-28", [["wb:k1", 5, 0, 1, 0]]),
  ];
  const rows = aggregateByKeyDim(days, set("2026-10-28"), names, "wb");
  eq(rows[0].tokens, 5, "30 天窗口不该把窗口外的历史算进来");
  eq(rows[0].requests, 1);
});

/* ── ⑤ 取不到名字时的兜底 ───────────────────────────────────────── */
check("Key 名取不到时用短 id，不显示「未知」", () => {
  const rows = aggregateByKeyDim(
    [dayOf("2026-10-28", [["wb:abcdefghijklmnop", 5, 0, 1, 0]])],
    set("2026-10-28"),
    names,
    "wb",
  );
  eq(rows.length, 1);
  if (!rows[0].name.startsWith("abcdefgh")) {
    throw new Error(`应显示短 id（前 8 位），实际 ${rows[0].name}`);
  }
});

/* ── ⑥ 覆盖披露：「没有该维度」≠「用量为 0」──────────────────────── */
check("keyCoverage 把「没有该维度」的天单列，不并进 0 用量", () => {
  const days = [
    dayOf("2026-10-27", []), // 有记录、但没有 keys（升级前的日子等价于此）
    dayOf("2026-10-28", [["wb:k1", 1, 0, 1, 0]], true),
  ];
  const coverage = keyCoverage(days, ["2026-10-27", "2026-10-28", "2026-10-29"]);
  eq(coverage, { days: 1, missing: 2, partial: 1 });
  eq(firstKeyDay(days, set("2026-10-27", "2026-10-28", "2026-10-29")), "2026-10-28");
});

/* ── ⑦ 桶键解析 ─────────────────────────────────────────────────── */
check("splitBucket：没有归属的桶键算本台账，不编一个 scope 出来", () => {
  eq(splitBucket("plain-id", "trae"), { kind: "key", scope: "trae", id: "plain-id" });
  eq(splitBucket("unified:abc", "wb"), { kind: "key", scope: "unified", id: "abc" });
  eq(splitBucket(LEGACY_BUCKET, "wb"), { kind: "legacy" });
  eq(splitBucket(NONE_BUCKET, "wb"), { kind: "none" });
});

/* ── ⑧ 空输入 ────────────────────────────────────────────────────── */
check("空输入返回空数组（不崩、不造假行）", () => {
  eq(aggregateByKeyDim([], set("2026-10-28"), names, "wb"), []);
});

console.log(
  failed === 0
    ? `密钥用量聚合：${passed} 项通过`
    : `密钥用量聚合：${passed} 项通过，${failed} 项失败`,
);
if (failed > 0) process.exit(1);
