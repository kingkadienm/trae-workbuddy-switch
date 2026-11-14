#!/usr/bin/env node
"use strict";

// 外部 Agent 监听：**来源 id ↔ 展示名**一致性护栏（零依赖，仅用 Node 标准库）。
//
// ## 它防的是什么
//
// 一个来源 id 同时活在**三处**，且没有任何编译期保护（Rust 常量 / TS 表 / 词条文件）：
//   ① crates/buddy-switch-core/src/modules/agent_watch.rs —— `SOURCE_*` 常量 + `HOOK_SOURCES` 表
//   ② src/components/companion/companion-row.tsx          —— `SOURCE_LABEL_KEY` 映射
//   ③ src/locales/domains/shared.{zh,en}.ts               —— `shared.companion.source.*` 词条
//
// 漏改的症状**全部是静默的**：
//   - ① 有、② 没有 ⇒ 悬浮窗里出现裸 id（`trae-cn`），用户看不懂那是什么；
//   - ② 有、③ 没有 ⇒ 显示成词条键名（`shared.companion.source.traeCn`），比裸 id 更糟；
//   - ① 没有、② 有 ⇒ 该来源永远进不来（`--source` 白名单会把它回落成 WorkBuddy，
//     于是「Trae 里的会话」显示成「WorkBuddy 国内版」—— 错标，且没有任何报错）。
//
// 故本脚本把 ①②③ 三处硬对齐。**加一个客户端时它会直接拦下来**，而不是等用户发现窗口内容不对。
//
// 用法：node scripts/check-agent-sources.cjs   （npm run check:agent-sources）

const fs = require("fs");
const path = require("path");

const ROOT = path.resolve(__dirname, "..");
const AGENT_WATCH_RS = path.join(
  ROOT,
  "crates",
  "buddy-switch-core",
  "src",
  "modules",
  "agent_watch.rs",
);
const COMPANION_ROW_TSX = path.join(
  ROOT,
  "src",
  "components",
  "companion",
  "companion-row.tsx",
);
const SHARED_ZH = path.join(ROOT, "src", "locales", "domains", "shared.zh.ts");
const SHARED_EN = path.join(ROOT, "src", "locales", "domains", "shared.en.ts");

/** 收集到的问题；非空即以非零码退出。 */
const problems = [];

function fail(message) {
  problems.push(message);
}

function read(file) {
  if (!fs.existsSync(file)) {
    fail(`缺文件：${path.relative(ROOT, file)}`);
    return "";
  }
  return fs.readFileSync(file, "utf8");
}

/**
 * 去掉一行里的 `//` 行注释（**引号感知**）。
 *
 * 不这么做会把字符串里的 `//`（如 URL）也切掉；虽然本仓这两处当前没有这种字面量，
 * 但「靠碰巧成立」的解析在下次加词条时就会静默失效。
 */
function stripLineComment(line) {
  let quote = null;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i];
    if (quote) {
      if (ch === "\\") i += 1;
      else if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'" || ch === "`") {
      quote = ch;
      continue;
    }
    if (ch === "/" && line[i + 1] === "/") return line.slice(0, i);
  }
  return line;
}

/** 取出一段以 `open` 开头、以 `close` 结尾的文本（含首尾行）。 */
function sliceBlock(source, openPattern, closePattern, label) {
  const open = source.search(openPattern);
  if (open < 0) {
    fail(`找不到 ${label} 的开头`);
    return null;
  }
  const rest = source.slice(open);
  const close = rest.search(closePattern);
  if (close < 0) {
    fail(`找不到 ${label} 的结尾`);
    return null;
  }
  return rest.slice(0, close);
}

// ---------------------------------------------------------------------------
// ① agent_watch.rs：SOURCE_* 常量 + HOOK_SOURCES 表
// ---------------------------------------------------------------------------

const rust = read(AGENT_WATCH_RS);

/** `pub const SOURCE_X: &str = "value";` → { SOURCE_X: "value" } */
const sourceConsts = new Map();
for (const match of rust.matchAll(
  /pub const (SOURCE_[A-Z0-9_]+)\s*:\s*&str\s*=\s*"([^"]*)"\s*;/g,
)) {
  sourceConsts.set(match[1], match[2]);
}

if (sourceConsts.size === 0) {
  fail("agent_watch.rs 里一个 SOURCE_* 常量都没解析到 —— 解析规则可能已失效");
}

const hookSourcesBlock = sliceBlock(
  rust,
  /pub const HOOK_SOURCES\s*:\s*&\[&str\]\s*=\s*&\[/,
  /\]\s*;/,
  "HOOK_SOURCES 表",
);

/** HOOK_SOURCES 解析出的**字面量**来源 id（按表内顺序）。 */
const hookSources = [];
if (hookSourcesBlock) {
  for (const match of hookSourcesBlock.matchAll(/\b(SOURCE_[A-Z0-9_]+)\b/g)) {
    const value = sourceConsts.get(match[1]);
    if (value === undefined) {
      fail(`HOOK_SOURCES 引用了未定义的常量 ${match[1]}`);
      continue;
    }
    hookSources.push(value);
  }
}

if (hookSources.length === 0) {
  fail("HOOK_SOURCES 表解析为空 —— 空表会让 --source 白名单把所有来源都回落成国内版");
}

const duplicates = hookSources.filter((id, index) => hookSources.indexOf(id) !== index);
if (duplicates.length > 0) {
  fail(`HOOK_SOURCES 有重复项：${[...new Set(duplicates)].join(", ")}`);
}

// 本工具自己的操作（`SOURCE_THIS_TOOL`）**不是 hook 来源**，但也要有展示名。
const thisTool = sourceConsts.get("SOURCE_THIS_TOOL");

// ---------------------------------------------------------------------------
// ② companion-row.tsx：SOURCE_LABEL_KEY
// ---------------------------------------------------------------------------

const tsx = read(COMPANION_ROW_TSX);
const labelBlock = sliceBlock(
  tsx,
  /const SOURCE_LABEL_KEY\s*:\s*Record<string,\s*TranslationKey>\s*=\s*\{/,
  /\n\};/,
  "SOURCE_LABEL_KEY 表",
);

/** 来源 id → 词条键。 */
const labelKeyOf = new Map();
if (labelBlock) {
  for (const rawLine of labelBlock.split("\n")) {
    const line = stripLineComment(rawLine);
    // 两种写法都要认：`"workbuddy-cn": "key",` 与 `claude: "key",`
    const match = line.match(
      /^\s*(?:"([^"]+)"|'([^']+)'|([A-Za-z_$][\w$]*))\s*:\s*"([^"]+)"\s*,?\s*$/,
    );
    if (!match) continue;
    const id = match[1] ?? match[2] ?? match[3];
    labelKeyOf.set(id, match[4]);
  }
}

if (labelKeyOf.size === 0) {
  fail("companion-row.tsx 的 SOURCE_LABEL_KEY 解析为空 —— 解析规则可能已失效");
}

// ---------------------------------------------------------------------------
// ③ 词条文件：key 集合
// ---------------------------------------------------------------------------

/** 从词条文件里抽出所有 `"<key>":` 的 key。 */
function translationKeys(file) {
  const text = read(file);
  const keys = new Set();
  for (const rawLine of text.split("\n")) {
    const line = stripLineComment(rawLine);
    const match = line.match(/^\s*"([^"]+)"\s*:/);
    if (match) keys.add(match[1]);
  }
  return keys;
}

const zhKeys = translationKeys(SHARED_ZH);
const enKeys = translationKeys(SHARED_EN);

// ---------------------------------------------------------------------------
// 校验
// ---------------------------------------------------------------------------

// 1) 每个 hook 来源都必须有展示名（否则窗口里是裸 id）。
for (const id of hookSources) {
  if (!labelKeyOf.has(id)) {
    fail(
      `来源 ${id} 在 companion-row.tsx 的 SOURCE_LABEL_KEY 里没有展示名 —— ` +
        `悬浮窗会显示裸 id「${id}」`,
    );
  }
}

// 2) 每个展示名键必须在 zh / en **两套**词条里都存在。
for (const [id, key] of labelKeyOf) {
  if (!zhKeys.has(key)) fail(`来源 ${id} 的词条键 ${key} 在 shared.zh.ts 里不存在`);
  if (!enKeys.has(key)) fail(`来源 ${id} 的词条键 ${key} 在 shared.en.ts 里不存在`);
}

// 3) 反向：SOURCE_LABEL_KEY 里不得出现**已不存在**的来源 id
//    （常量改名后留下的死条目会让「有展示名」这件事看起来成立，其实映射的是旧 id）。
const knownIds = new Set(hookSources);
if (thisTool !== undefined) knownIds.add(thisTool);
for (const id of labelKeyOf.keys()) {
  if (!knownIds.has(id)) {
    fail(
      `SOURCE_LABEL_KEY 里的「${id}」不对应任何已知来源 —— ` +
        `常量可能已改名，这条映射现在是死的`,
    );
  }
}

// 4) 本工具自己的操作也要有展示名（`buddy-switch` 会出现在窗口里）。
if (thisTool !== undefined && !labelKeyOf.has(thisTool)) {
  fail(`SOURCE_THIS_TOOL（${thisTool}）没有展示名 —— 窗口里会显示裸 id`);
}

// 5) 两套词条**键集必须一致**（漏一套 ⇒ 切语言时显示键名）。
for (const key of zhKeys) {
  if (key.startsWith("shared.companion.source.") && !enKeys.has(key)) {
    fail(`词条 ${key} 只有中文，shared.en.ts 缺同键`);
  }
}

// ---------------------------------------------------------------------------

if (problems.length > 0) {
  console.error("✗ 来源 / 展示名一致性校验失败：");
  for (const problem of problems) console.error(`  - ${problem}`);
  console.error(
    `\n加一个客户端时要同时改：agent_watch.rs 的 HOOK_SOURCES、` +
      `companion-row.tsx 的 SOURCE_LABEL_KEY、shared.{zh,en}.ts 的 shared.companion.source.*`,
  );
  process.exit(1);
}

console.log(
  `✓ 来源 / 展示名一致（${hookSources.length} 个 hook 来源：${hookSources.join(", ")}）`,
);
