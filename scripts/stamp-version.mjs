#!/usr/bin/env node
/**
 * BuddySwitch 打包版本戳 —— **唯一实现**。
 *
 * ## 为什么是 Node 而不是 bash
 *
 * 本机（Windows）**没有 bash/sh**（连 Git Bash 都没装），所以 `package.json` 里
 * `sh scripts/stamp-version.sh` 那几条全部跑不了。于是把「写版本号」的逻辑收在这一份
 * `.mjs` 里，两侧都只是薄包装：
 *   - `scripts/stamp-version.sh` → `exec node scripts/stamp-version.mjs "$@"`（CI / macOS / Linux）
 *   - `scripts/package-windows.ps1` → 直接 `node scripts/stamp-version.mjs`（Windows 一键打包）
 * 逻辑只有一份，不会出现「bash 侧改了、PS 侧没改」的漂移。
 *
 * ## 版本形态 `<YYYY>.<M>.<DHHMM>`
 *
 * 例：2026-09-20 18:45 → `2026.9.201845`；2026-10-01 10:19 → `2026.10.11019`
 *
 *   - **必须 3 段数字**：Cargo 严格 semver，`2026.9.20.1845` 会被直接拒绝
 *     （`error: unexpected character '.' after patch version number`）。
 *     本脚本**前置拦下**，免得编了十几分钟才在 cargo 那里报一句难读的错。
 *   - 第三段 `DHHMM` 让**同一天多次打包版本号不同**。只写日期（`2026.9.20`）时，
 *     自动更新会把当天的新包误判成「已是最新」→ 修复推不出去，NSIS 产物还会同名覆盖。
 *   - **★ 年 / 月 / 日一律不补零，时 / 分必须补零**（两个方向都不能反）：
 *     第三段是 semver 的 **patch**，而 patch **禁止前导零** —— 日补零会让每月 1–9 号
 *     生成 `2026.10.011019`，cargo / npm / tauri 三家全部拒收。2026-10-01 真踩到：
 *     四个平台的 CI 全在 `Build server binary` 挂掉，只报一句
 *     `error: invalid leading zero in patch version number`。
 *     反过来 **HH/MM 必须补零**，第三段的数值才等于 `日 × 10000 + 时 × 100 + 分`，
 *     跨日比较才单调（不补零时 `1`+`0`+`19` = `1019` < 前一天的 `11019`）。
 *   - 与 `update::version_tuple()` 严格单调递增语义兼容：同日内第三段递增，
 *     跨日/跨月因前两段递增而递增。
 *
 * ## 用法
 *
 *   node scripts/stamp-version.mjs                # 用系统时间生成并写入
 *   node scripts/stamp-version.mjs --print         # 只打印，不写入
 *   node scripts/stamp-version.mjs --tag v1.2.3    # 显式指定（CI tag 构建；tag 是唯一真相）
 *   node scripts/stamp-version.mjs 2026.9.201845   # 位置参数等价于 --tag
 *   node scripts/stamp-version.mjs --check         # 只校验：所有载体是否都等于 package.json 的版本
 *                                                 #   不一致 → 退出码 1（用来兜住「漏改一处」）
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

// ---------------------------------------------------------------- 参数解析

let printOnly = false;
let checkOnly = false;
let explicit = '';

for (let i = 2; i < process.argv.length; i++) {
  const a = process.argv[i];
  if (a === '--print') {
    printOnly = true;
  } else if (a === '--check') {
    checkOnly = true;
  } else if (a === '--tag') {
    explicit = process.argv[++i] || '';
  } else if (/^v?\d/.test(a)) {
    explicit = a;
  } else {
    console.error(`未知参数: ${a}`);
    process.exit(2);
  }
}

/**
 * 从系统时间推导 `<YYYY>.<M>.<DHHMM>`。
 *
 * ⚠️ **日不补零**（补了会让每月 1–9 号生成 `…011019`，patch 段前导零 ⇒ semver 拒收，
 * 见文件头「版本形态」一节）；**时 / 分必须补零**（否则第三段的数值语义垮掉、跨日不单调）。
 */
function versionFromClock(now = new Date()) {
  const y = now.getFullYear();
  const m = now.getMonth() + 1; // 不补零
  const dd = String(now.getDate()); // ★ 不补零（patch 段最高位）
  const hh = String(now.getHours()).padStart(2, '0');
  const mi = String(now.getMinutes()).padStart(2, '0');
  return `${y}.${m}.${dd}${hh}${mi}`;
}

const version = explicit ? explicit.replace(/^v/, '') : versionFromClock();

if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(
    `[stamp-version] 版本号必须是 3 段数字 <YYYY>.<M>.<DHHMM>（Cargo 拒绝 4 段 semver），收到: ${version}`,
  );
  process.exit(2);
}

// ★ 段内不得有前导零。与上一条是**两类**不同的错：段数错是本脚本自己拼错，
//   前导零是**日期取值**触发的（每月 1–9 号必踩，2026-10-01 CI 四平台全红就是它）。
//   放在这里而不是只在 `versionFromClock` 里改，是因为 `--tag` 是**外部输入**
//   （CI 传 `GITHUB_REF_NAME`）—— 打错 tag 也要在本地就被拦下。
const leadingZero = version
  .split('.')
  .find((segment) => segment.length > 1 && segment.startsWith('0'));
if (leadingZero !== undefined) {
  console.error(
    `[stamp-version] 版本号不允许前导零段（semver 禁止）：段 "${leadingZero}" 以 0 开头，收到: ${version}`,
  );
  process.exit(2);
}

if (printOnly) {
  console.log(version);
  process.exit(0);
}

// ------------------------------------------------------------ 载体（读写）

/** 安全读 JSON；文件不存在返回 null，解析失败**抛错**（静默回落会掩盖坏文件）。 */
function readJson(rel) {
  const abs = path.join(ROOT, rel);
  if (!fs.existsSync(abs)) return null;
  try {
    return JSON.parse(fs.readFileSync(abs, 'utf8'));
  } catch (e) {
    throw new Error(`JSON 解析失败 ${rel}: ${e.message}`);
  }
}

/**
 * 一个「版本载体」。`read()` 返回若干 `{ key, value }` 条目（有的文件里版本出现在多处），
 * `write(v)` 写回并返回是否真的改动。
 */
const targets = [];

/** 顶层 `version` 字段（package.json / tauri.conf.json / 各 npm 包）。 */
function addJsonTarget(rel) {
  targets.push({
    rel,
    read() {
      const j = readJson(rel);
      if (!j || j.version === undefined) return [];
      return [{ key: 'version', value: String(j.version) }];
    },
    write(v) {
      const abs = path.join(ROOT, rel);
      if (!fs.existsSync(abs)) return false;
      const j = JSON.parse(fs.readFileSync(abs, 'utf8'));
      if (j.version === undefined || j.version === v) return false;
      j.version = v;
      fs.writeFileSync(abs, JSON.stringify(j, null, 2) + '\n');
      return true;
    },
  });
}

/**
 * `package-lock.json` 有**两个** version 字段：顶层 `version` 与 `packages[""]`。
 * 漏掉后者，`npm ci` 会因「lock 根包版本 ≠ package.json」报错；漏掉前者，
 * `npm version` / 部分 CI 工具读到旧值。两处都要写，且**不重排整个 lock**
 * （保持原格式便于 review diff）。
 */
function addLockTarget(rel = 'package-lock.json') {
  targets.push({
    rel,
    read() {
      const j = readJson(rel);
      if (!j) return [];
      const out = [];
      if (j.version !== undefined) out.push({ key: 'version', value: String(j.version) });
      if (j.packages && j.packages[''] && j.packages[''].version !== undefined) {
        out.push({ key: 'packages[""].version', value: String(j.packages[''].version) });
      }
      return out;
    },
    write(v) {
      const abs = path.join(ROOT, rel);
      if (!fs.existsSync(abs)) return false;
      const j = JSON.parse(fs.readFileSync(abs, 'utf8'));
      let changed = false;
      if (j.version !== undefined && j.version !== v) {
        j.version = v;
        changed = true;
      }
      if (j.packages && j.packages[''] && j.packages[''].version !== v) {
        j.packages[''].version = v;
        changed = true;
      }
      if (!changed) return false;
      fs.writeFileSync(abs, JSON.stringify(j, null, 2) + '\n');
      return true;
    },
  });
}

/**
 * 主包的 `optionalDependencies` 引用必须与平台包同步，否则 `npm install` 会去
 * 拉一个不存在的版本。
 */
function addOptionalDepsTarget(rel = 'npm/package.json') {
  targets.push({
    rel,
    read() {
      const j = readJson(rel);
      if (!j || !j.optionalDependencies) return [];
      return Object.entries(j.optionalDependencies).map(([k, v]) => ({ key: k, value: String(v) }));
    },
    write(v) {
      const abs = path.join(ROOT, rel);
      if (!fs.existsSync(abs)) return false;
      const j = JSON.parse(fs.readFileSync(abs, 'utf8'));
      let changed = false;
      for (const k of Object.keys(j.optionalDependencies || {})) {
        if (j.optionalDependencies[k] !== v) {
          j.optionalDependencies[k] = v;
          changed = true;
        }
      }
      if (!changed) return false;
      fs.writeFileSync(abs, JSON.stringify(j, null, 2) + '\n');
      return true;
    },
  });
}

/** Cargo.toml：只替换 `[package]` 段的首个行首 `version = "..."`（不碰 dependencies 里的同名键）。 */
function addCargoTarget(rel) {
  targets.push({
    rel,
    read() {
      const abs = path.join(ROOT, rel);
      if (!fs.existsSync(abs)) return [];
      const m = fs.readFileSync(abs, 'utf8').match(/^version = "([^"]+)"/m);
      return m ? [{ key: 'version', value: m[1] }] : [];
    },
    write(v) {
      const abs = path.join(ROOT, rel);
      if (!fs.existsSync(abs)) return false;
      const raw = fs.readFileSync(abs, 'utf8');
      const next = raw.replace(/^version = "[^"]+"/m, `version = "${v}"`);
      if (next === raw) return false;
      fs.writeFileSync(abs, next);
      return true;
    },
  });
}

// 载体清单（**新增载体只改这一段**）
addJsonTarget('package.json');
addJsonTarget('src-tauri/tauri.conf.json');
addJsonTarget('npm/package.json');
addOptionalDepsTarget('npm/package.json');
addLockTarget('package-lock.json');
for (const dir of sortedDirs('npm/platform')) {
  addJsonTarget(`npm/platform/${dir}/package.json`);
}
// 四个 crate 都要同步：漏掉任何一个，`env!("CARGO_PKG_VERSION")` 都会让
// 「API 服务」页显示成旧版本号（该值会被透出到网关 /status）。
/**
 * workspace 成员的 crate 名（**动态扫描**，不写死清单）。
 *
 * ## 为什么不能写死
 * 写死的清单每加一个 crate 就会漏一次 —— 漏掉的后果不是「版本号不统一」，
 * 而是 `Cargo.lock` 与 `Cargo.toml` 对不上，CI 的 `cargo check --locked` 直接红
 * （2026-10-29 现场：新增 `buddy-switch-vault` 后没进清单 ⇒ lock 过期 ⇒ 门禁失败）。
 */
function workspaceCrateNames() {
  const names = new Set();
  const dirs = [];
  const cratesDir = path.join(ROOT, 'crates');
  if (fs.existsSync(cratesDir)) {
    for (const entry of fs.readdirSync(cratesDir, { withFileTypes: true })) {
      if (entry.isDirectory()) dirs.push(path.join('crates', entry.name, 'Cargo.toml'));
    }
  }
  dirs.push(path.join('src-tauri', 'Cargo.toml'));
  for (const rel of dirs) {
    const abs = path.join(ROOT, rel);
    if (!fs.existsSync(abs)) continue;
    const raw = fs.readFileSync(abs, 'utf8');
    const m = raw.match(/^name = "([^"]+)"/m);
    if (m) names.add(m[1]);
  }
  return names;
}

const WORKSPACE_CRATES = workspaceCrateNames();

for (const rel of [
  'src-tauri/Cargo.toml',
  ...fs
    .readdirSync(path.join(ROOT, 'crates'), { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => path.join('crates', d.name, 'Cargo.toml')),
]) {
  addCargoTarget(rel);
}

/**
 * `Cargo.lock` 也要跟着走（**workspace 成员的版本**）。
 *
 * ## 为什么必须在这里
 * CI 跑的是 `cargo check --workspace --all-targets --locked` —— `--locked` 禁止
 * 自动改 lock。lock 里 workspace 成员的版本与各 `Cargo.toml` 不一致时，
 * cargo 会直接报错退出（而不是「顺手修好」）⇒ 门禁红。
 * （2026-10-29 现场：新增 `buddy-switch-vault` 没进载体清单 ⇒ lock 过期 ⇒ CI 红。）
 *
 * ## 为什么逐行处理而不是正则
 * lock 的条目形如 `name = "x"` 紧跟 `version = "y"`，但**换行符可能是 CRLF**
 * （Windows 上 checkout 出来的就是），正则里 `
` 与 `
` 的差异会让匹配静默失败 ——
 * 而「静默没改」正是这里最坏的失败方式（看起来成功了，lock 其实还是旧的）。
 * 逐行匹配对行尾不敏感。
 */
targets.push({
  rel: 'Cargo.lock',
  read() {
    const abs = path.join(ROOT, 'Cargo.lock');
    if (!fs.existsSync(abs)) return [];
    const out = [];
    const lines = fs.readFileSync(abs, 'utf8').split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^name = "(.+)"$/);
      if (!m || !WORKSPACE_CRATES.has(m[1])) continue;
      const vm = (lines[i + 1] || '').match(/^version = "(.+)"$/);
      if (vm) out.push({ key: m[1], value: vm[1] });
    }
    return out;
  },
  write(v) {
    const abs = path.join(ROOT, 'Cargo.lock');
    if (!fs.existsSync(abs)) return false;
    const lines = fs.readFileSync(abs, 'utf8').split(/\r?\n/);
    let changed = false;
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^name = "(.+)"$/);
      if (!m || !WORKSPACE_CRATES.has(m[1])) continue;
      const vm = (lines[i + 1] || '').match(/^version = "(.+)"$/);
      if (!vm || vm[1] === v) continue;
      lines[i + 1] = `version = "${v}"`;
      changed = true;
    }
    if (!changed) return false;
    fs.writeFileSync(abs, lines.join('\n'));
    return true;
  },
});

function sortedDirs(rel) {
  const abs = path.join(ROOT, rel);
  if (!fs.existsSync(abs)) return [];
  return fs
    .readdirSync(abs, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .sort();
}

// ---------------------------------------------------------------- 执行

const rows = [];

if (checkOnly) {
  const ref = readJson('package.json');
  const expected = ref && ref.version ? String(ref.version) : null;
  if (!expected) {
    console.error('[stamp-version] 无法从 package.json 取到 version，校验中止。');
    process.exit(2);
  }
  let mismatches = 0;
  let seen = 0;
  for (const t of targets) {
    for (const e of t.read()) {
      seen++;
      if (e.value !== expected) {
        mismatches++;
        console.error(`  [x] ${t.rel} ${e.key} = ${e.value}（应为 ${expected}）`);
      }
    }
  }
  if (mismatches > 0) {
    console.error(`[stamp-version] 版本不一致：${mismatches}/${seen} 处与 package.json(${expected}) 不符。`);
    process.exit(1);
  }
  console.log(`[stamp-version] 校验通过：${seen} 处载体均为 ${expected}`);
  process.exit(0);
}

let changedFiles = 0;

for (const t of targets) {
  const before = t.read().map((e) => e.value);
  const changed = t.write(version);
  if (changed) changedFiles++;
  rows.push({ rel: t.rel, changed, before: before.join(' / ') });
}

// ------------------------------------------------------------------ 输出

const width = rows.reduce((n, r) => Math.max(n, r.rel.length), 0);
for (const r of rows) {
  const mark = r.changed ? '已更新' : '无变化';
  console.log(`  ${mark}  ${r.rel.padEnd(width)}${r.changed ? `   (${r.before} -> ${version})` : ''}`);
}

console.log(`版本已统一为 ${version}（改动 ${changedFiles} 个文件）`);
