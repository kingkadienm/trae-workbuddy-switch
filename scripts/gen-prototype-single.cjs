#!/usr/bin/env node
/**
 * 把 `docs/prototype/` 的模块化原型（`index.html` + `demo.css` + `demo-data.js` + `demo-app.js`）
 * 拼成单文件版 `buddy-switch-prototype.html`，供「双击即开 / 发给别人评审」用。
 *
 * ## 为什么要有这个脚本（2026-10-21 的真实教训）
 *
 * 这两份此前是**各自手改**的。改用量页时实测：单文件的脚本块比 `demo-app.js` 落后
 * 300+ 行（积分包明细那段完全不是同一个版本），而 `scripts/check-scope-registry.cjs`
 * 只校验一个锚点，抓不到这种分叉 ⇒ 改完「看起来两边都改了」，实际只有一边生效。
 *
 * ⇒ 单文件从此是**产物**，不是源码。改完模块化那份跑 `npm run proto:single`。
 * 手改单文件会在下次生成时被覆盖 —— 这是刻意的。
 */
const fs = require("node:fs");
const path = require("node:path");

const dir = path.join(__dirname, "..", "docs", "prototype");
const read = (file) => fs.readFileSync(path.join(dir, file), "utf8");

/** 内联时唯一会破坏 HTML 结构的序列。 */
function assertInlinable(name, text) {
  if (/<\/script>/i.test(text) || /<\/style>/i.test(text)) {
    throw new Error(`${name} 里出现了 </script> 或 </style>，无法内联`);
  }
}

let html = read("index.html");
const css = read("demo.css");
const data = read("demo-data.js");
const app = read("demo-app.js");

assertInlinable("demo.css", css);
assertInlinable("demo-data.js", data);
assertInlinable("demo-app.js", app);

const pairs = [
  ['<link rel="stylesheet" href="demo.css" />', `<style>\n${css}\n</style>`],
  ['<script src="demo-data.js"></script>', `<script>\n${data}\n</script>`],
  ['<script src="demo-app.js"></script>', `<script>\n${app}\n</script>`],
];

for (const [from, to] of pairs) {
  if (!html.includes(from)) throw new Error(`index.html 里找不到外链锚点：${from}`);
  html = html.replace(from, to);
}
if (/<link rel="stylesheet"|<script src=/.test(html)) {
  throw new Error("还有没被内联的外链 —— 生成结果不是自包含的");
}

/* 单文件头部加一行来源说明：打开的人第一眼就知道改哪儿。 */
html = html.replace(
  "<title>Buddy Switch — 新架构交互原型</title>",
  "<title>Buddy Switch — 新架构交互原型（单文件版）</title>\n"
  + "<!-- 本文件由 scripts/gen-prototype-single.cjs 生成：index.html + demo.css + demo-data.js + demo-app.js。"
  + "不要手改，改模块化那四份后跑 `npm run proto:single`。 -->",
);

const out = path.join(dir, "buddy-switch-prototype.html");
fs.writeFileSync(out, html);
console.log(`wrote ${path.relative(process.cwd(), out)} (${html.length} chars)`);
