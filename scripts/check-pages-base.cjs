#!/usr/bin/env node
"use strict";

// GitHub Pages 资源前缀护栏（零依赖，仅用 Node 标准库）。
//
// ## 为什么需要它
//
// Pages 项目站的地址是 `https://<owner>.github.io/<repo>/` —— 站点根**就是仓库名**。
// Vite 的 `base` 决定 `index.html` 里资源的绝对前缀，而它此前是**写死的字符串**：
//
//   base = "/trae-workbuddy-switch/"      ← 2026-10-17 之前的仓库名
//
// 仓库改名成 `qoder-trae-workbuddy-switch` 之后，症状是**最阴的一类**：
//   `GET /qoder-trae-workbuddy-switch/`            → 200（index.html 正常返回）
//   `GET /trae-workbuddy-switch/assets/index-*.js` → 404
//   ⇒ `#root` 永远没有子节点，浏览器只显示 `<title>Buddy Switch`
//   ⇒ 构建、部署、`deploy-pages` 三步全绿，没有任何一步会报错
//
// 换句话说：**它坏得完全静默**，只能靠人去点开线上地址才看得见。所以这里把
// 「base 的真值只能来自仓库名」钉成静态门禁：
//
//   ① `pages.yml` 的演示构建步骤必须显式传 `VITE_BASE: /${{ github.event.repository.name }}/`
//      （或等价的 `${{ github.repository }}` 派生写法），**不许**依赖代码里的常量；
//   ② `vite.config.ts` 里兜底的 slug 常量必须与本地 `origin` 的仓库名一致
//      （无 git 或无 origin 时跳过这条并打印说明，不让浅克隆 / tarball 构建假红）；
//   ③ 演示构建的输出目录必须是 `dist-demo`（`dist/` 是 rust-embed / Tauri 的输入，
//      混用会让 webui 与桌面端双双空白 —— 见 `docs/specs/` 与 `check-api-contract.cjs`）。
//
// 用法：node scripts/check-pages-base.cjs   （npm run check:pages-base）

const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const ROOT = path.resolve(__dirname, "..");
const PAGES_YML = path.join(ROOT, ".github", "workflows", "pages.yml");
const VITE_CONFIG = path.join(ROOT, "vite.config.ts");

const failures = [];
const notes = [];

function fail(message) {
  failures.push(message);
}

function read(file) {
  if (!fs.existsSync(file)) {
    fail(`缺少文件：${path.relative(ROOT, file)}`);
    return "";
  }
  return fs.readFileSync(file, "utf8");
}

/** 从 `vite.config.ts` 里取出兜底 slug 常量（`"..." || "slug"` 的右值）。 */
function readFallbackSlug(source) {
  const match = source.match(/pagesRepoSlug\s*=([\s\S]*?);/);
  if (!match) return null;
  const literals = match[1].match(/"([^"]+)"/g) || [];
  if (literals.length === 0) return null;
  return literals[literals.length - 1].replace(/"/g, "");
}

function readOriginRepoName() {
  try {
    const url = execFileSync("git", ["remote", "get-url", "origin"], {
      cwd: ROOT,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    const tail = url.replace(/\.git$/, "").split(/[/:]/).pop();
    return tail || null;
  } catch {
    return null;
  }
}

// ── ① Pages 工作流必须按仓库名传 base ────────────────────────────────────────
const pagesYml = read(PAGES_YML);
if (pagesYml) {
  if (!/repository\.name|github\.repository/.test(pagesYml)) {
    fail(
      "pages.yml 没有用 `github.event.repository.name` / `github.repository` 派生 base —— " +
        "仓库改名后线上 demo 会静默全站空白（index.html 200、JS 404、#root 为空）",
    );
  }
  if (!/VITE_BASE\s*:\s*\/\$\{\{[^}]+\}\}\//.test(pagesYml)) {
    fail("pages.yml 的演示构建步骤必须传 `VITE_BASE: /${{ ... }}/`，否则 base 退回代码里的常量");
  }
  if (!/path:\s*dist-demo\b/.test(pagesYml)) {
    fail("pages.yml 上传的必须是 `dist-demo`；上传 `dist` 会把演示构建编进 webui/桌面端");
  }
  if (/path:\s*dist\s*$/m.test(pagesYml)) {
    fail("pages.yml 出现了 `path: dist` —— 演示产物绝不能占用 `dist/`");
  }
}

// ── ② 兜底常量必须与本地 origin 一致 ────────────────────────────────────────
const viteConfig = read(VITE_CONFIG);
if (viteConfig) {
  const slug = readFallbackSlug(viteConfig);
  if (!slug) {
    fail("vite.config.ts 里找不到 `pagesRepoSlug` 兜底常量 —— 本地 `build:demo` 会拿不到 base");
  } else {
    const originName = readOriginRepoName();
    if (!originName) {
      notes.push("无 git origin（浅克隆 / tarball？），跳过「兜底 slug ↔ 仓库名」比对");
    } else if (originName !== slug) {
      fail(
        `vite.config.ts 的兜底 slug 是 "${slug}"，但 origin 仓库名是 "${originName}" —— ` +
          "本地 build:demo 会产出指向错误前缀的产物（CI 有 VITE_BASE 覆盖，所以只在本地发作）",
      );
    } else {
      notes.push(`兜底 slug 与 origin 一致：${slug}`);
    }
  }
}

// ── 输出 ────────────────────────────────────────────────────────────────────
for (const note of notes) console.log(`  ·  ${note}`);
if (failures.length > 0) {
  console.error("\n[check-pages-base] 失败：");
  for (const message of failures) console.error(`  ✗ ${message}`);
  process.exit(1);
}
console.log("[check-pages-base] 通过：Pages base 由仓库名派生，演示产物隔离在 dist-demo");
