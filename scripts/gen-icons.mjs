#!/usr/bin/env node
/**
 * 从一张方形 PNG 原图重建 `src-tauri/icons` 下的全套应用图标。
 *
 * 为什么需要这个脚本：图标不是「换一张图」那么简单，各平台对图标的裁切方式不同——
 * macOS 会给方形图标套 squircle 遮罩，Windows 则原样显示位图。因此 Windows 侧的
 * ico / Square* / StoreLogo 必须自己烘焙圆角，否则会出现难看的直角边框。
 *
 * 流程（与仓库既有约定一致）：
 *   1. 校验源图：真实 PNG、正方形、边长 >= 1024（Tauri 官方要求）
 *   2. `tauri icon` 生成基础全套：png / icns / ico / Square* / StoreLogo / android / ios
 *   3. 强制同步 `icon.png` = 源图（托盘图标与圆角脚本都以它为输入，必须确保是新图）
 *   4. 运行 `gen-windows-rounded-icon.py`，把 Windows 侧图标烘焙成 macOS 风格圆角
 *   5. 运行 `gen-tray-icon.py`，生成 macOS 菜单栏单色模板（template image）
 *   6. 运行 `gen-public-icons.py`，同步 `public/` 下的前端图标（侧栏标记 + favicon）
 *   7. 校验关键产物齐备，并打印清单
 *
 * ⚠️ `public/` 下的图标**不在 `tauri icon` 的产出范围**内，缺少第 6 步时
 * 换 logo 只会更新安装包图标，而**侧栏品牌标记与 Web favicon 会一直停在旧图**
 * （曾实际发生：09-16 换了 logo，public/ 仍是 09-10 的旧绿色猫）。
 *
 * 用法：
 *   node scripts/gen-icons.mjs [源图路径]
 *   node scripts/gen-icons.mjs D:/path/to/logo.png
 *   （默认源图：docs/images/brand/logo.png）
 *
 * 依赖：Node（本项目自带）+ Pillow（`python -m pip install Pillow`）。
 * 可用 BUDDY_SWITCH_PYTHON 指定解释器路径。
 *
 * 注意：`tray-icon-template.rgba` 是 **单色剪影**，由 `gen-tray-icon.py` 从源图生成，
 * **不是彩色图标的等比缩小**。运行时真正生效的是 `.rgba`，PNG 只是预览——
 * 手工改 PNG 不会影响运行，且下次跑本脚本会被静默覆盖。
 * 填充色为白色：macOS 菜单栏走 template image 只取 alpha、忽略 RGB；
 * Windows 通知区按 RGB 原样显示（深色任务栏需要白），故同一份产物两端通用。
 * 该文件被 `tray.rs` 以 `include_bytes!` 引用，且有 `&[u8; 36 * 36 * 4]` 的编译期长度断言。
 */

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const ICONS_DIR = path.join(ROOT, "src-tauri", "icons");
const TAURI_CLI = path.join(ROOT, "node_modules", "@tauri-apps", "cli", "tauri.js");
const ROUNDED_SCRIPT = path.join(HERE, "gen-windows-rounded-icon.py");
const TRAY_SCRIPT = path.join(HERE, "gen-tray-icon.py");
const PUBLIC_SCRIPT = path.join(HERE, "gen-public-icons.py");

/** `tauri icon` 未生成、但被代码/配置实际依赖的产物（缺失即视为失败）。 */
const REQUIRED = [
  "32x32.png",
  "128x128.png",
  "128x128@2x.png",
  "icon.png",
  "icon.icns",
  "icon.ico",
  "StoreLogo.png",
  "Square150x150Logo.png",
  "android/mipmap-xxxhdpi/ic_launcher.png",
  "ios/AppIcon-512@2x.png",
  "tray-icon-template.png",
  "tray-icon-template.rgba",
];

/** `public/` 下由 `gen-public-icons.py` 产出、前端实际引用的图标。 */
const REQUIRED_PUBLIC = ["icon.png", "icon-transparent.png"];

function fail(msg) {
  console.error(`\n[gen-icons] 失败：${msg}\n`);
  process.exit(1);
}

/** 直接解析 PNG 头部（IHDR），不引入任何图像库依赖。 */
function readPngSize(file) {
  const buf = readFileSync(file);
  const isPng =
    buf.length > 24 &&
    buf[0] === 0x89 && buf[1] === 0x50 && buf[2] === 0x4e && buf[3] === 0x47;
  if (!isPng) return null;
  const width = buf.readUInt32BE(16);
  const height = buf.readUInt32BE(20);
  return { width, height };
}

/** 依次尝试候选解释器，返回第一个能 `import PIL` 的。 */
function resolvePython() {
  const candidates = [
    process.env.BUDDY_SWITCH_PYTHON,
    process.platform === "win32" ? "python" : "python3",
    "python3",
    "python",
    "C:\\Users\\JackeyYe\\.workbuddy\\binaries\\python\\versions\\3.13.12\\python.exe",
  ].filter(Boolean);

  for (const bin of candidates) {
    try {
      execFileSync(bin, ["-c", "import PIL"], { stdio: "ignore" });
      return bin;
    } catch {
      /* 换下一个候选 */
    }
  }
  return null;
}

function main() {
  // ---------- 1. 校验输入 ----------
  const input = path.resolve(process.argv[2] ?? path.join(ROOT, "docs", "images", "brand", "logo.png"));
  if (!existsSync(input)) fail(`源图不存在：${input}`);

  const size = readPngSize(input);
  if (!size) fail(`不是合法的 PNG 文件：${input}`);
  if (size.width !== size.height) {
    fail(`源图必须是正方形，当前 ${size.width}x${size.height}`);
  }
  if (size.width < 1024) {
    fail(`源图边长需 >= 1024（Tauri 要求），当前 ${size.width}`);
  }
  console.log(`[gen-icons] 源图 ${path.relative(ROOT, input)} (${size.width}x${size.height}) ✓`);

  if (!existsSync(TAURI_CLI)) {
    fail(`未找到 Tauri CLI：${TAURI_CLI}（先执行 npm install）`);
  }

  // ---------- 2. 生成基础全套 ----------
  console.log("[gen-icons] 1/5 生成基础图标（tauri icon）...");
  execFileSync(process.execPath, [TAURI_CLI, "icon", input, "--output", ICONS_DIR], {
    cwd: ROOT,
    stdio: "inherit",
  });

  // ---------- 3. 同步 icon.png ----------
  // 托盘图标（tray.rs include_bytes）与下一步的圆角脚本都以 icon.png 为输入，
  // 不能假设 tauri icon 一定产出它，这里显式对齐，避免出现「新图配旧托盘」。
  console.log("[gen-icons] 2/5 同步 icon.png <- 源图");
  const masterPng = path.join(ICONS_DIR, "icon.png");
  copyFileSync(input, masterPng);

  const python = resolvePython();
  if (!python) {
    fail(
      "未找到可用的 Python + Pillow。请先安装：\n" +
        "        python -m pip install Pillow\n" +
        "      或用 BUDDY_SWITCH_PYTHON 指定解释器。",
    );
  }

  // ---------- 4. Windows 圆角 ----------
  console.log("[gen-icons] 3/5 烘焙 Windows 圆角（含 .ico / Square* / StoreLogo）...");
  execFileSync(python, [ROUNDED_SCRIPT], { cwd: ROOT, stdio: "inherit" });

  // ---------- 5. macOS 菜单栏单色模板 ----------
  console.log("[gen-icons] 4/6 生成 macOS 菜单栏单色模板（template image）...");
  execFileSync(python, [TRAY_SCRIPT, input], { cwd: ROOT, stdio: "inherit" });

  // ---------- 6. public/ 前端图标 ----------
  // `tauri icon` 不管 public/，缺这一步会留下「安装包图标是新的、侧栏与 favicon 是旧的」。
  console.log("[gen-icons] 5/6 同步 public/ 前端图标（侧栏标记 + favicon）...");
  execFileSync(python, [PUBLIC_SCRIPT, input], { cwd: ROOT, stdio: "inherit" });

  // ---------- 7. 校验 ----------
  console.log("[gen-icons] 6/6 校验产物 ...");
  const missing = REQUIRED.filter((rel) => !existsSync(path.join(ICONS_DIR, rel)));
  if (missing.length) fail(`以下产物缺失：\n      ${missing.join("\n      ")}`);
  const missingPublic = REQUIRED_PUBLIC.filter(
    (rel) => !existsSync(path.join(ROOT, "public", rel)),
  );
  if (missingPublic.length) fail(`public/ 产物缺失：\n      ${missingPublic.join("\n      ")}`);

  const rows = REQUIRED.map((rel) => {
    const kb = (statSync(path.join(ICONS_DIR, rel)).size / 1024).toFixed(1);
    return `      ${rel.padEnd(42)} ${kb.padStart(8)} KB`;
  });
  console.log(`[gen-icons] 完成 ✓ 图标已写入 ${path.relative(ROOT, ICONS_DIR)}`);
  console.log(rows.join("\n"));
  console.log(
    `[gen-icons] 完成 ✓ 前端图标已写入 ${path.relative(ROOT, path.join(ROOT, "public"))}（${REQUIRED_PUBLIC.join(" / ")}）`,
  );
  console.log(
    "\n[gen-icons] 提示：托盘用的是单色剪影（白色填充），由 gen-tray-icon.py 从源图生成；\n" +
      "            macOS 只取 alpha 着色、Windows 按 RGB 显示，tray.rs 对尺寸有编译期断言，勿改 36x36。\n" +
      "            改托盘图标要改 gen-tray-icon.py，不要手改产物（会被本脚本覆盖）。",
  );
}

main();
