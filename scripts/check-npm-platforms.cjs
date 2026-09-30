#!/usr/bin/env node
/**
 * 护栏：npm 平台包的**四处声明**必须逐项一致。
 *
 * 为什么需要它：这四处是手工同步的，历史上已经漂过一次 —— `linux-arm64` 出现在
 * `bin/buddy-switch.js` / `scripts/install.js` 的 FILE 表与 `npm/platform/` 目录里，
 * 却既不在主包 `optionalDependencies`、也不在 CI 矩阵中。结果是用户装完主包即报
 * 「平台包未安装」，而代码看起来「明明支持」。这类缺陷不报错、只在特定平台上出现，
 * 靠人眼 review 抓不住，所以交给脚本。
 *
 * 四处声明：
 *   ① `npm/platform/<tag>/package.json` 的 `name` / `os` / `cpu`
 *   ② 主包 `npm/package.json` 的 `optionalDependencies`
 *   ③ `npm/bin/buddy-switch.js` 与 `npm/scripts/install.js` 的 FILE 键
 *   ④ `.github/workflows/build.yml` 的 `platform_tag`
 *
 * 用法：node scripts/check-npm-platforms.cjs
 * 退出码：0 = 一致；1 = 有漂移（打印逐条差异）。
 */
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const PLATFORM_DIR = path.join(ROOT, "npm", "platform");
const SCOPE = "@kingkadienm";

const problems = [];
const fail = (msg) => problems.push(msg);

function readJson(p) {
  // 畸形 JSON 要变成一条**可读的失败项**，而不是抛栈 —— 护栏自己崩了等于没有护栏，
  // 而且堆栈会把真正要修的文件名埋掉。
  let raw;
  try {
    raw = fs.readFileSync(p, "utf8");
  } catch (e) {
    fail(`${path.relative(ROOT, p)}: 读取失败（${e.message}）`);
    return null;
  }
  try {
    return JSON.parse(raw);
  } catch (e) {
    fail(`${path.relative(ROOT, p)}: JSON 解析失败（${e.message}）`);
    return null;
  }
}

/** 从 JS 源码里抽出 `const FILE = { ... }` 的键（形如 "darwin-arm64"）。 */
function fileMapKeys(p) {
  const src = fs.readFileSync(p, "utf8");
  const m = src.match(/const FILE = \{([\s\S]*?)\n\}/);
  if (!m) {
    fail(`${path.relative(ROOT, p)}: 未找到 \`const FILE = { ... }\`，无法校验平台表`);
    return [];
  }
  return [...m[1].matchAll(/"([a-z0-9]+-[a-z0-9_]+)":/g)].map((x) => x[1]);
}

const eqSet = (a, b) =>
  a.length === b.length && [...a].sort().join(",") === [...b].sort().join(",");

function diff(a, b) {
  const A = new Set(a);
  const B = new Set(b);
  return {
    onlyA: [...A].filter((x) => !B.has(x)).sort(),
    onlyB: [...B].filter((x) => !A.has(x)).sort(),
  };
}

// ---------------------------------------------------------------- ① 目录
if (!fs.existsSync(PLATFORM_DIR)) {
  console.error(`check-npm-platforms: 缺少目录 ${PLATFORM_DIR}`);
  process.exit(1);
}
// 目录名是 `buddy-switch-<tag>`（**不带 scope**，包名才带），平台 tag 是去掉前缀后的部分。
const PREFIX = "buddy-switch-";
const entries = fs
  .readdirSync(PLATFORM_DIR, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => {
    if (!d.name.startsWith(PREFIX)) {
      fail(
        `npm/platform/${d.name}: 目录名必须形如 ${PREFIX}<platform>-<arch>（包名才带 @scope）`,
      );
    }
    return { dir: d.name, tag: d.name.slice(PREFIX.length) };
  })
  .sort((a, b) => a.tag.localeCompare(b.tag));
const tags = entries.map((e) => e.tag);

if (tags.length === 0) {
  fail("npm/platform 下没有任何平台目录");
}

for (const { dir, tag } of entries) {
  const pkgPath = path.join(PLATFORM_DIR, dir, "package.json");
  if (!fs.existsSync(pkgPath)) {
    fail(`npm/platform/${tag} 缺少 package.json`);
    continue;
  }
  const pkg = readJson(pkgPath);
  if (!pkg) continue;
  const wantName = `${SCOPE}/buddy-switch-${tag}`;
  if (pkg.name !== wantName) {
    fail(`npm/platform/${tag}/package.json: name=${pkg.name}，应为 ${wantName}`);
  }
  // 目录名即 `<os>-<cpu>`，与包内 os/cpu 字段必须自洽（npm 按这两个字段挑包）
  const [os, cpu] = tag.split("-");
  if (!Array.isArray(pkg.os) || pkg.os[0] !== os) {
    fail(`npm/platform/${tag}/package.json: os=${JSON.stringify(pkg.os)}，应为 ["${os}"]`);
  }
  if (!Array.isArray(pkg.cpu) || pkg.cpu[0] !== cpu) {
    fail(`npm/platform/${tag}/package.json: cpu=${JSON.stringify(pkg.cpu)}，应为 ["${cpu}"]`);
  }
}

// ---------------------------------------------------------------- ② optionalDependencies
const mainPkg = readJson(path.join(ROOT, "npm", "package.json")) || {};
const optDeps = Object.keys(mainPkg.optionalDependencies || {}).sort();
const expectDeps = tags.map((t) => `${SCOPE}/buddy-switch-${t}`).sort();
if (!eqSet(optDeps, expectDeps)) {
  const d = diff(expectDeps, optDeps);
  fail(
    `主包 optionalDependencies 与平台目录不一致：` +
      (d.onlyA.length ? `目录里有但依赖没写 → ${d.onlyA.join(", ")}；` : "") +
      (d.onlyB.length ? `依赖里有但目录没有 → ${d.onlyB.join(", ")}` : ""),
  );
}

// ---------------------------------------------------------------- ③ 两处 FILE 表
const binKeys = fileMapKeys(path.join(ROOT, "npm", "bin", "buddy-switch.js")).sort();
const installKeys = fileMapKeys(path.join(ROOT, "npm", "scripts", "install.js")).sort();
if (!eqSet(binKeys, tags)) {
  const d = diff(tags, binKeys);
  fail(
    `bin/buddy-switch.js 的 FILE 表与平台目录不一致：` +
      `目录独有 → [${d.onlyA.join(", ")}]；FILE 独有 → [${d.onlyB.join(", ")}]`,
  );
}
if (!eqSet(installKeys, tags)) {
  const d = diff(tags, installKeys);
  fail(
    `scripts/install.js 的 FILE 表与平台目录不一致：` +
      `目录独有 → [${d.onlyA.join(", ")}]；FILE 独有 → [${d.onlyB.join(", ")}]`,
  );
}

// ---------------------------------------------------------------- ④ CI 矩阵
const wfPath = path.join(ROOT, ".github", "workflows", "build.yml");
const wf = fs.readFileSync(wfPath, "utf8");
const ciTags = [...new Set([...wf.matchAll(/"platform_tag":\s*"([^"]+)"/g)].map((m) => m[1]))].sort();
if (ciTags.length === 0) {
  fail(`${path.relative(ROOT, wfPath)}: 未匹配到任何 platform_tag（矩阵格式变了？请更新本脚本）`);
} else if (!eqSet(ciTags, tags)) {
  const d = diff(tags, ciTags);
  fail(
    `CI 矩阵的 platform_tag 与平台目录不一致：` +
      `目录独有 → [${d.onlyA.join(", ")}]；CI 独有 → [${d.onlyB.join(", ")}]`,
  );
}

// ---------------------------------------------------------------- 结论
if (problems.length) {
  console.error(`[check-npm-platforms] 发现 ${problems.length} 处不一致：`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log(
  `[check-npm-platforms] 一致：${tags.length} 个平台（${tags.join(", ")}）` +
    ` —— 目录 / optionalDependencies / 两处 FILE 表 / CI 矩阵 四处均对齐`,
);
