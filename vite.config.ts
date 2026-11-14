import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// GitHub Pages serves only the public demo from the repository subpath.
// Normal WebUI and Tauri builds intentionally keep Vite's root base.
//
// Output-dir convention (keep these separate!):
//   `npm run build`       → `dist/`       base "/"                  — 被三处消费：
//                                       rust-embed(`crates/buddy-switch-server`)、
//                                       Tauri `frontendDist`、`scripts/fix-app.sh`
//   `npm run build:demo`  → `dist-demo/`  base "/<仓库名>/"   — 仅供 GitHub Pages
// 两者曾共用 `dist/`，导致「把演示构建编进 server/Tauri」→ index.html 请求
// `/trae-workbuddy-switch/assets/*`（embed 中不存在）→ 回退成 HTML → 模块脚本 MIME 校验失败
// → webui/桌面端空白页。`api::tests::embedded_index_html_references_only_embedded_assets`
// 是这条约定的回归护栏。
// ⚠️ `VITE_PAGES_DEMO` 同时决定两件事：**资源 base** 与 **路由类型**
// （`App.tsx` 用它选 `HashRouter`）。但本地验证需要「HashRouter + base=/」这一组合：
// Pages 形态的产物在本地预览下资源全部 404（`/<repo>/assets/*` 不存在于
// 本地静态根）⇒ 页面一行都不渲染，CDP 断言全假红。
// ⇒ 把 base 抽成可覆盖的 `VITE_BASE`（缺省行为**逐字不变**）。
//
// ⚠️⚠️ Pages 项目站的路径**就是仓库名**，所以 base 里的 slug 与仓库名是同一份真值。
// 2026-10-17 仓库改名（`trae-workbuddy-switch` → `qoder-trae-workbuddy-switch`）后，
// 这里写死的旧 slug 让线上 demo 变成**全站空白**：`index.html` 200，但它请求
// `/trae-workbuddy-switch/assets/index-*.js` → 404 ⇒ `#root` 永远是空的，浏览器只显示
// `<title>`。为让下次改名不再靠人记：
//   ① CI（`.github/workflows/pages.yml`）显式传 `VITE_BASE=/<github.event.repository.name>/`；
//   ② 本地 `npm run build:demo` 用 `GITHUB_REPOSITORY` 兜底，再退回下面的常量；
//   ③ `scripts/check-pages-base.cjs` 把「workflow 必须按仓库名传 base」钉成门禁。
// @ts-expect-error process is a nodejs global
const pagesRepoSlug =
  // @ts-expect-error process is a nodejs global
  process.env.GITHUB_REPOSITORY?.split("/")[1] || "trae-workbuddy-switch";
// @ts-expect-error process is a nodejs global
const base =
  process.env.VITE_BASE ??
  // @ts-expect-error process is a nodejs global
  (process.env.VITE_PAGES_DEMO === "1" ? `/${pagesRepoSlug}/` : "/");

// https://vite.dev/config/
export default defineConfig(async () => ({
  base,
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
    // 本地开发代理：将 API 和网关请求转发到 Rust 后端（默认 57890）
    proxy: {
      "/api": {
        target: "http://127.0.0.1:57890",
        changeOrigin: true,
      },
      "/v1": {
        target: "http://127.0.0.1:57890",
        changeOrigin: true,
      },
    },
  },
}));
