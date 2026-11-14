#!/usr/bin/env node
/**
 * 生成 README 里那批浅色 / 深色截图（`docs/images/*-light.png` / `*-dark.png`）。
 *
 * ## 为什么是一个脚本而不是手动截图
 *
 * README 首屏就是这 4 组图，改版界面后手点 20 张图必然漏、而且两次截图之间窗口大小/滚动位置
 * 不一致，读者看到的是「同一页两种排版」。这里把「路由 → 文件名 → 视口 → 缩放」全部写死，
 * 重跑一次就是一套同规格的图。
 *
 * ## 怎么跑（本机无 playwright 依赖，用临时前缀装，别污染 package.json）
 *
 * ```bash
 * npm install --no-save --prefix .tmp-pcli playwright
 * node .tmp-pcli/node_modules/playwright/cli.js install chromium
 * node scripts/run-with-env.mjs VITE_DEMO_MODE=1 -- npx vite build --outDir dist-shot
 * node .tmp-pcli/serve.mjs dist-shot 4319        # 一个只做 SPA 回退的静态服务器
 * node scripts/gen-readme-shots.mjs
 * ```
 *
 * ⚠️ **必须是演示构建**（`VITE_DEMO_MODE=1`）：图里出现的账号、积分、请求记录都是虚构夹具。
 * 真实数据截图会把别人的邮箱和 Token 带进仓库。
 *
 * ## 两条规格（改之前先读）
 *
 * 1. **视口 1280×820 + DPR 2** ⇒ 输出 2560×1640，与既有图片逐像素同规格；
 * 2. **浅色 / 深色各自一个 context**（`colorScheme` + `localStorage` 双写）——
 *    只靠 `matchMedia` 不够，主题偏好存在 `buddy-switch.theme` 里，两个都要给。
 */

import { createRequire } from "node:module";

/* playwright 是**临时安装**的（`.tmp-pcli`，不进 package.json）：ESM 不认 `NODE_PATH`，
   只能按显式路径 require。把它装成 devDependency 时删掉这段、改回 `import` 即可。 */
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE ?? "../.tmp-pcli/node_modules/playwright");

const BASE = process.env.SHOT_BASE ?? "http://127.0.0.1:4319";
const OUT = process.env.SHOT_OUT ?? "docs/images";

/** 目标页 → 文件名（顺序即截图顺序）。
 *
 * `scope` + `dest` 决定路由；**一律先从该作用域的「概览」进站，再点侧栏导航**：
 * 顶栏那颗作用域按钮的「N 个账号」来自各页自己维护的 store / 快照，
 * 直接 `goto` 深链时 WorkBuddy 侧还没人拉过账号列表 ⇒ 图上会写「0 个账号」。
 * 按真实用法走一遍（进站 → 点导航）才有 8 个账号，也不必为截图改产品代码。
 */
const SHOTS = [
  { scope: "wb-cn", dest: "概览", name: "overview" },
  { scope: "wb-cn", dest: "账号", name: "workbuddy-accounts" },
  { scope: "wb-cn", dest: "用量", name: "usage-trend" },
  { scope: "wb-cn", dest: "用量", name: "usage-credits", tab: "积分" },
  { scope: "wb-cn", dest: "会话", name: "sessions" },
  { scope: "wb-cn", dest: "网关", name: "api-service" },
  { scope: "wb-cn", dest: "设置", name: "settings" },
  { scope: "trae-cn", dest: "账号", name: "trae-accounts" },
  { scope: "trae-cn", dest: "用量", name: "trae-usage" },
  { scope: "qoder-cn", dest: "账号", name: "qoder-accounts" },
  { scope: "qoder-cn", dest: "网关", name: "qoder-gateway" },
  { scope: "qoder-cn", dest: "概览", name: "qoder-overview" },
];

const THEMES = ["light", "dark"];

const settle = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function shoot(context, shot, theme) {
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));

  const waitForShell = async () => {
    await page.waitForFunction(() => {
      const root = document.querySelector("#root");
      return root !== null && root.textContent.trim().length > 80;
    }, { timeout: 20_000 });
    await settle(500);
  };

  // ① 进站（概览会把账号列表读进 store）② 点侧栏导航到目标页 —— 见 SHOTS 的说明。
  await page.goto(`${BASE}/s/${shot.scope}/overview`, { waitUntil: "load", timeout: 30_000 });
  await waitForShell();
  if (shot.dest !== "概览") {
    await page.getByRole("link", { name: new RegExp(`^${shot.dest}`) }).first().click({ timeout: 15_000 });
    await settle(700);
  }
  // 演示夹具是同步注入的，但图表（recharts）要等一帧才有宽度。
  await settle(500);

  if (shot.tab) {
    await page.getByRole("tab", { name: shot.tab }).click({ timeout: 10_000 });
    await settle(600);
  }

  const file = `${OUT}/${shot.name}-${theme}.png`;
  await page.screenshot({ path: file, animations: "disabled" });
  const text = (await page.locator("#root").innerText()).replace(/\s+/g, " ").slice(0, 60);
  await page.close();
  return { file, text, errors };
}

const browser = await chromium.launch();
const report = [];
for (const theme of THEMES) {
  const context = await browser.newContext({
    viewport: { width: 1280, height: 820 },
    deviceScaleFactor: 2,
    colorScheme: theme,
    reducedMotion: "reduce",
    locale: "zh-CN",
    timezoneId: "Asia/Shanghai",
  });
  await context.addInitScript((value) => {
    // 主题偏好的唯一真值在 localStorage；`colorScheme` 只影响 `system` 分支。
    localStorage.setItem("buddy-switch.theme", value);
    // 首次引导浮层（「快速上手」/「知道了」）会盖住截图底部，先按「已关闭」处理。
    localStorage.setItem("buddy-switch:v1:first-run-hint-dismissed", "1");
  }, theme);

  for (const shot of SHOTS) {
    const result = await shoot(context, shot, theme);
    report.push({ theme, ...result });
    console.log(`${theme.padEnd(5)} ${shot.name.padEnd(20)} ${result.text}`);
    if (result.errors.length > 0) console.log(`        ⚠️ pageerror: ${result.errors.join(" | ")}`);
  }
  await context.close();
}
await browser.close();

const bad = report.filter((row) => row.errors.length > 0);
console.log(`\n${report.length} 张图已写入 ${OUT}`);
if (bad.length > 0) {
  console.log(`⚠️ ${bad.length} 张有 pageerror，请检查：${bad.map((row) => row.file).join(", ")}`);
  process.exitCode = 1;
}
