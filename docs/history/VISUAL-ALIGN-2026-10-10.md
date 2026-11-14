# 视觉 1:1 对齐：差异清单与改造记录

> **状态：只读历史（不再更新，2026-10-17 归档）** —— 视觉 1:1 对齐改造记录，已完结。
> 现行文档见 [`docs/README.md`](../README.md)（文档地图）。
> 日期：2026-10-10 · 基准：`docs/prototype/demo.css` + `buddy-switch-prototype.html`
> 范围：**表现层**（几何 / 配色 / 字体 / 间距 / 断点 / 交互态 / 空态）
> **业务逻辑、数据流、接口调用零改动。**
>
> ⚠️ 与 `ALIGN-2026-10-10.md` 的关系：那份是**行为**差异（14 项，已全部处理），
> 本份是**视觉**差异（首次系统性比对）。两份互补，不重叠。

---

## 0. 判据：先分清「已对齐」与「看起来像差异」

`src/index.css` 里到处写着「取自原型 `:NN`」—— 这些是前人**刻意对齐**过的。
逐条核对后确认**全部一致**，本轮**未动**：

| 项 | 原型 | 实现 | 结论 |
|:---|:--|:--|:--|
| 顶栏 | `height:56px` `padding:0 14px` `gap:10px` | `h-14 px-3.5 gap-2.5` | ✅ |
| 侧栏 | `width:240px` `padding:14px 12px 12px` | `w-[240px] px-3 pt-3.5 pb-3` | ✅ |
| `.page` | `max-width:1240px` `padding:26px 28px 60px` | `max-w-[1240px] px-7 pt-[26px] pb-[60px]` | ✅ |
| `.nav-item` | `38px` `0 10px` `r10` `13px` `gap:10px` | 同 | ✅ |
| `.nav-badge` | `min-w20 h18 0 6px r999 10.5px` | 同 | ✅ |
| `.brand-mark` | `32px r10 svg17` | 同 | ✅ |
| `.run-pill` | `h26 r999 success15% 11px` | 同 | ✅ |
| `.search-btn` | `h34 r10 12px` | 同 | ✅ |
| 令牌 | light/dark 两套 oklch | 逐值一致（含 `--card-2`/`--border-strong`/三级 `--elevation`） | ✅ |

> **结论：令牌层与外壳早已对齐，本轮偏差全在「原子组件 size 阶梯」与「基础排版」两处。**

---

## 1. 差异清单与处置（25 项，全部已落实）

### 1.1 基础排版（`src/index.css`）

| # | 原型 | 原实现 | 严重度 | 处置 |
|:-:|:---|:---|:-:|:---|
| 1 | `body{font-size:13.5px}`（`:64`） | **缺失** ⇒ 浏览器默认 **16px** | **高** | 补 `font-size:13.5px` |
| 2 | `body{line-height:1.5}`（`:65`） | 仅靠 preflight 继承（隐式耦合） | 低 | 显式重写 |
| 3 | `svg{width:16px;height:16px}`（`:70`） | 无全局兜底 | 中 | 补 `@layer base` 全局规则 |
| 4 | `kbd` 等宽 10.5px/r5px/`--muted` 底（`:71-75`） | **全站无 kbd 规则**，两处各写一套且三项全不同 | 中 | 补全局规则，两处调用点去重 |
| 5 | `[aria-label="Token 总览"]` 死规则 | 全库无此 aria-label，永不生效 | 中 | **删除** |

> **#1 是本轮影响最大的一项**：Tailwind preflight 只给 `html` 设 `line-height`，**不设字号**
> ⇒ 凡未显式写 `text-*` 的文本节点全部放大 18.5%，而写了 `text-xs`(12px) 的又比原型偏小
> ⇒ 形成「正文偏大、标注偏小」的反向错配。

### 1.2 原子组件

| # | 组件 | 原型 | 原实现 | 处置 |
|:-:|:---|:---|:---|:---|
| 6 | Button `default` | `h34 px13 r10 12.5px gap6 svg14` | `h-9 px-4 py-2 text-sm gap-2 svg-16` | 逐值对齐 |
| 7 | Button `sm` | `h28 0 10px 12px r8` | `h-8 px-3`（14px） | 逐值对齐 |
| 8 | Button `xs` | `h24 0 8px 11px r7` | **不存在** | 新增 `xs` 档 |
| 9 | Button `destructive` | **描边式**（`demo.css:203`） | 实心 `bg-destructive` | 改描边式 |
| 10 | Button `ghost` | 静止态即弱化字色（`:201`） | 无 `text-muted-foreground` | 补字色 |
| 11 | Button `outline` | 底 `--card-2` | 底 `--background`（亮一档） | 改 `bg-card-2` |
| 12 | Button `:disabled` | `opacity:.45`（`:207`） | `.5` | 改 `.45` |
| 13 | Badge | `h20 r6 10.5px 0 7px svg11` | 自适应高 `r10 12px 0 8px svg12` | 逐值对齐 |
| 14 | Badge `.ok/.warn` | `success16%` / `warning18%` | 均 `15%` | 改 `16/18` |
| 15 | Skeleton | **扫光** 1.3s + `r7`（`:365-366`） | `animate-pulse` + `r10` | 换扫光 + `r7` |
| 16 | EmptyState | `padding:56px 20px`（`:357`） | `px-6 py-10` | 改 `px-5 py-14` |
| 17 | EmptyState 图标 | **44px/r14/`--muted` 底块** + 20px 图标（`:358-362`） | 32px **裸图标** | 补底块 |

> **#9 的依据**：原型不可逆操作确认框的确认键就是 `btn danger`（`buddy-switch-prototype.html:1027`），
> 全站仅此一处 `danger` 用法，故按描边式对齐；实测本仓 46 处 `variant="destructive"`
> 分布在确认框 / 删除列表项 / 危险区，均属该语义。

> **变体名与原型类名是错位的**（极易按名字对齐错）：
> 原型 `.btn`（最常见那个）↔ 本仓 `outline`；原型 `.btn.primary` ↔ 本仓 `default`。

### 1.3 页头与容器

| # | 项 | 原型 | 原实现 | 处置 |
|:-:|:---|:---|:---|:---|
| 18 | `h1` | `640` / `-.02em`（`:179`） | 9/10 页 `600`(semibold) / `-.025em`(tight) | 10 处统一 |
| 19 | 页头副标题 `p` | `12.5px` / `margin-top:6px`（`:180`） | `12px`(xs) / 6px | 改 12.5px |
| 20 | 4 个统计页 | `h1` 28px | `h2 text-xl` = **20px** | 升级为页头规格 |
| 21 | `SettingsPage` 容器 | `.page` = **1240px** | `max-w-5xl` = **1024px** | 改 1240px |
| 22 | `TraeSettingsPage` 容器 | `.page` = **1240px** | `max-w-3xl` = **768px** | 改 1240px |

> **#21/#22 的依据**：原型 `viewSettings`（`buddy-switch-prototype.html:3232`）用的是标准
> `<div class="page">`，**不是**更窄的容器。已核对原型源码确认。

### 1.4 透明度与细节

| # | 位置 | 原型 | 原实现 | 处置 |
|:-:|:---|:---|:---|:---|
| 23 | `.nav-item.active` 底 | `primary 16%`（`:155`） | `/15` | 改 `/16` |
| 24 | `.brand-mark` 底 | `primary 18%`（`:139`） | `/15` | 改 `/18` |
| 25 | `.nav-badge.warn` 底 | `warning 22%`（`:162`） | `/20` | 改 `/22` |
| 26 | `.nav-item.active .nav-badge` | `primary 25%` + 前景色（`:163`） | **未实现** | 补（`NavLink` 改函数式 children 取 `isActive`） |
| 27 | `.scope-trigger` 展开环 | `0 0 0 3px ring22%`（`:97`） | `2px / 20%` | 改 3px/22% |
| 28 | `.run-pill .dot` 光晕 | `0 0 0 3px success22%`（`:113`） | 无 | 补 |

### 1.5 响应式断点

原型 4 条断点全部接入（`index.css`，**断点值照抄原型**，不改成 Tailwind 的 `sm/md/lg`）：

| 原型 | 行为 | 接入方式 |
|:---|:---|:---|
| `max-width:1100px`（`:227`） | `.g4` → 2 列 | `[data-grid-cols="4"]`，宿主 `usage/stat-tiles.tsx` |
| `max-width:760px`（`:228`） | `.g4` → 1 列 | 同上 |
| `max-width:900px`（`:379`） | `.split` 转纵向 | `[data-split-layout]`，宿主 `SettingsPage` / `ApiServicePage` |
| ~~`max-width:980px`（`:721`）~~ | ~~`.chart-2col` 单列~~ | **未接入** —— 本仓无 `1.6fr` 网格，无宿主 |

> ⚠️ **无宿主的规则不写**。原型有、本仓没有对应结构的断点，宁可留空并在此说明，
> 也不留一条永不生效的规则（旧代码的 `[aria-label="Token 总览"]` 就是这么来的）。

---

## 2. demo 中不存在的自定义设计：已清除

| 项 | 说明 | 处置 |
|:---|:---|:---|
| **Bricolage Grotesque 字体** | demo 全文 **0 处**引用；真实实现在 6 个文件内联注入 `fontFamily`（`App.tsx` 品牌名、`account-card` / `trae-account-card` 积分大数字、`stat-tiles` 两处、`TraeCreditsPage`、`TraeTokenStatsPage`），并在 `main.tsx` import fontsource | 删除全部 6 处注入 + `main.tsx` import + `package.json` 依赖 |

> 依据「不引入 demo 中没有的自定义设计」。字体栈回归 `body` 的
> `ui-sans-serif / -apple-system / "SF Pro Text" / "Segoe UI" / "Microsoft YaHei"`（与原型一致）。

---

## 3. 边界情况处置（需求 §4）

### 3.1 长文本溢出 —— Badge（**本轮新引入的风险，已修**）

固定 `h-5` 后，`w-fit` + `whitespace-nowrap` + `overflow-hidden` 会在父容器不够宽时
**物理裁掉右侧字符且无任何提示**。真实受害点：

- `client-limits-card.tsx` 传后端模型名（`client_limits.rs` 的 `model: Option<String>`，
  **运行时数据、无长度上限**）；
- `TraeCreditsPage` 传用户自定义分组名（限 40 字）。

**修法**：`text-overflow:ellipsis` **只对块容器生效**，外层是 `inline-flex`
（其文本成为匿名 flex 项）⇒ 只给外层加 `truncate` 等于没加。
故外层给 `max-w-full`，**内层包 `<span class="min-w-0 truncate">`**；
`asChild` 时不包（避免非法嵌套），并给字符串内容补 `title` 以便悬停看全文。

### 3.2 图片加载失败

本轮涉及的组件**不使用位图**（全部为矢量图标 + CSS 渐变），故无图片失败态。
唯一渐变是骨架屏的 `linear-gradient`，其色标取自 CSS 变量，主题切换不会失效。

### 3.3 缺失的第三方资源

清掉 Bricolage 后，`src/` 内已无任何外部字体依赖，产物 `dist/assets/*.woff2` 数量
从 3 降到 **0** —— 消除了「字体加载失败导致回退字形」这一类风险。

### 3.4 窄窗口（需求 §1「响应式断点」）

**发现一处真问题（既有缺陷，非本轮引入）**：顶栏最小需求 ≈795px，而
`tauri.conf.json` 的 `minWidth` 是 **720px** ⇒ 最窄窗口下右侧设置按钮被挤出视口。
成因是作用域按钮 `shrink-0` + `max-w-300px`、搜索钮 `min-w-250px` **都不可压缩**。

**修法**：搜索钮改响应式 `min-w`（窄屏 120px、宽屏 250px）+ 作用域按钮去掉 `shrink-0`。
> ⚠️ 这是**主动偏离原型**（原型 `min-width:250px` 是给浏览器窗口用的），
> 桌面端有 720px 硬下界，照抄会让最窄窗口破版。已在代码注释写明理由。

### 3.5 核查后确认「无实际影响」的（不凑数）

`scope-selector`（`max-w` + `min-w-0` + 双行 `truncate` 四要素齐备）、
`empty-state`（flex 列 + `max-w-sm`，`<p>` 天然换行）、页面 `h1`
（最长英文 18 字符 ≈270px ≪ 可用 424px）、Button 文字（`whitespace-nowrap` 在位，全库 0 处覆盖）。

---

## 4. 自测结果（全部亲手跑）

| 套件 | 结果 |
|:---|:---|
| `npx tsc --noEmit` | ✅ 0 error |
| `npm run check:scope` | ✅ **273 / 0** |
| `npm run check:api` | ✅ routes=166 / call=154 / invoke=172 |
| `npm run build` | ✅ built in 8.94s |
| `cargo check --workspace` | ✅ Finished dev profile |
| **产物 CSS 逐值核验** | ✅ 11/11（`font-size:13.5px` / `kbd` 全局 / 扫光 `@keyframes` / `svg` 兜底 / `rounded-[6px]` / `h-[34px]` / 两个断点钩子 / 搜索钮宽度…） |

**变异验证**（证明可证伪）：把 `font-size:13.5px` 改成 `16px` 重建 ⇒
产物出现 `font-size:16px`、`13.5px` 消失 ⇒ **可检出**；已还原并重建确认无残留。

**改动范围核验**：`git diff` 全量扫描确认**未触及任何** `api.` / `call(` / `invoke` /
`useState` / `useEffect` / 命令名 ⇒ 业务逻辑零改动。

---

## 5. 改动文件清单（28 个，+334 / −121）

| 类别 | 文件 |
|:---|:---|
| **基础样式** | `src/index.css`（+100/−3） |
| **原子组件** | `ui/button.tsx`、`ui/badge.tsx`、`ui/skeleton.tsx`、`ui/empty-state.tsx` |
| **外壳** | `App.tsx`、`shell/top-bar.tsx`、`shell/scope-selector.tsx`、`shell/run-pill.tsx`、`command-palette/command-palette.tsx` |
| **页头/容器** | 10 个页面：`AccountsPage` `ApiServicePage` `SessionsPage` `SettingsPage` `UsagePage` `TokenStatsPage` `CreditStatsPage` `TraeAccountsPage` `TraeApiServicePage` `TraeSettingsPage` `TraeTokenStatsPage` `TraeCreditsPage` `TraeSessionsPage` |
| **字体清除** | `main.tsx`、`package.json`、`account-card.tsx`、`trae-account-card.tsx`、`usage/stat-tiles.tsx`、`TraeCreditsPage.tsx`、`TraeTokenStatsPage.tsx` |

---

## 6. 仍存在的差异与限制（诚实列出）

| 项 | 说明 |
|:---|:--|
| **`.chart-2col` 的 980px 断点未接入** | 本仓无 `1.6fr` 双列图表网格，无宿主元素。接入需先改结构，超出「仅调整表现层」范围 |
| **`.card` 默认内边距未改** | 原型 `.card` 无内边距（由 `.card-pad`/`.card-head` 提供），shadcn `Card` 默认 `py-6 gap-6`。全库 20+ 调用点，且多数已传 `py-0 gap-0` 覆盖；改动波及面大、收益低，本轮**有意不动** |
| **`.card-head` 页面级写法未收口** | 20+ 处手写 `px-5 py-3` + `text-sm`(14px)，原型是 `13px 16px` + `13.5px`。建议后续抽 `CardHeader` 原语统一，本轮未做（结构性重构，非纯表现层） |
| **`.badge.info` / `.badge.bad` 未新增** | 原型有这两个变体，本仓无使用点。按「不引入 demo 中没有的**用到**的设计」未加 |
| **统计块两套几何并存** | 概览走 `separated` 形态（已逐值对齐 `.stat`）；4 个统计页走默认形态（26px/600/居中，原型是 20px/640/左对齐）。**默认形态未对齐** —— 它在原型里没有对应物，强行对齐需先判定原型统计页该用哪种形态 |
| **视觉回归无自动门禁** | `check-scope-registry.cjs` 仍是 273 条、本轮未加断言。现有断言全是行为契约；加「断言 class 名」会把门禁变成锁死实现细节。**逐值依据已写进各组件注释（带 `demo.css:NN` 行号）作为可追溯来源** |
| **未提交** | 28 个文件留在工作区，按 §3「精确 add、绝不 `git add -A`」等用户确认 |

---

## 7. 第二轮：补齐结构性残留（同日，用户要求「完全按原型」）

§6 列出的残留里，有 7 项是**用户能直接看到**的结构性差异。本轮全部落地：

| # | 差异 | 原型依据 | 处置 |
|:-:|:---|:---|:---|
| 1 | **用量页重复页头** —— `UsagePage` 渲染「用量」页头后，子页 `TokenStatsPage` / `CreditStatsPage` / `TraeTokenStatsPage` / `TraeCreditsPage` 各自再渲染 28px 标题 + 副标题 + 刷新按钮 + 独立 `max-w-[1240px]` 容器 ⇒ 出现第二个标题与双层内边距 | `viewUsage`（`buddy-switch-prototype.html:2823-2832`）只有一个页头，结构是「页头 → seg → params → body」 | 页头/页容器收敛到 `UsagePage` 一处；四个子页删除标题、副标题、`1240px` 容器，只留视图参数条与视图内容。刷新改为页头按钮 `key={reloadKey}` **只重挂当前视图**（不跨视图打空请求）。⚠️ Trae 积分页保留「同步积分」行内动作 —— 它是**独立的上游动作**，不是视图刷新 |
| 2 | 账号卡**鼠标点击主体不打开详情**（只有 Enter/Space） | `.acct` 整卡是 `data-act="acct-open"`（`:2497-2500`） | 补 `onClick`，守卫内部可点区（`button/a/input/[role=checkbox]/[role=switch]`），键盘走 `e.target===e.currentTarget` |
| 3 | 账号页列表前**多一行客户端状态徽标**（WB / IDE / CLI 图标行 + `mb-6`） | `viewAccounts`（`:2424-2440`）是「页头 → 工具栏 → 网格」，无此行 | 删除该行；连带清理无用变量与 `product-marks` 导入 |
| 4 | 批量条是**底部浮条**（`fixed bottom-6`） | `.batch-bar`（`:303-307`）是**文档流内**横条：`primary 35%` 描边 + `color-mix(primary 10%, card)` 底，插在工具栏与列表之间 | 改回文档流内；两平台账号页同步 |
| 5 | 会话页**无排序控件** | `.sess-sort-seg`（`:3187-3190`）「最近更新 / 最早更新」 | 补排序；★ 只排**列表顺序**，`updatedAt` **不参与**副本状态推断（§3.15.2「时间新 ≠ 内容新」） |
| 6 | 用量统计 tile 是「合并成一张带分隔线的卡」 | 四视图的 `statTile()` 全是**独立卡**（`:2661-2666` 等） | `StatTiles` 默认 `separated = true` |
| 7 | 卡片头 padding / 代理卡外层 `py-6` | `.card-head` = `13px 16px` + 标题 `13.5px`（`:234-239`）；`.card` 自身无内边距 | `proxy-panel` 外层卡补 `gap-0 py-0`；网关地址卡头改 `px-4 py-[13px]` + `13.5px` |

**顺带对齐**：`AccountsPage` 的 `h1` 由 `font-semibold tracking-tight` 改 `font-[640] tracking-[-0.02em]`（原型 `.page-head h1`），副标题 `text-xs` → `text-[12.5px]`。

### 7.1 验证

| 项 | 结果 |
|:---|:---|
| `npx tsc --noEmit` | ✅ 0 error |
| `npm run check:scope` | ✅ 273 / 0 |
| `npm run check:api` | ✅ routes=166 |
| `check:store-selectors` / `check:npm-platforms` | ✅ |
| `npm run build` / `build:demo` | ✅ |
| **CDP 真机核验**（headless Chrome + 演示 dev server） | ✅ 用量页 `h1`=1、独立统计卡=4；账号页无状态行、3 卡；会话页排序按钮在且默认 `recent`；批量条 `position:static` + `top(283) > toolbarBottom(269)` ⇒ 确在文档流内 |

截图：`logs/verify-2026-10-10/{usage-trend,accounts,sessions,accounts-batchbar}.png`。

### 7.2 核验脚本的三个坑（都会造假绿，务必记住）

1. **演示构建的 `base` 是 `/trae-workbuddy-switch/`**（`vite.config.ts:21`）⇒ 直接挂在根路径的静态服务会 404、页面全白。核验改用 `VITE_DEMO_MODE=1 npx vite`（dev server 走根路径，且是 BrowserRouter）。
2. **只改 hash 不会重新挂载 SPA** ⇒ 截出来的图与打印的 DOM 事实**全是上一页的**。必须整页 `Page.navigate` 到真实路径。
3. **Vite dev server 只监听 `localhost`（IPv6）**，`127.0.0.1` 返回 502。

### 7.3 本轮仍未做

- `.card` 默认 `py-6 gap-6` 未动（20+ 调用点，多数已 `py-0 gap-0` 覆盖，改动面大收益低）；
- `.card-head` 页面级手写写法未抽原语统一；
- `.chart-2col` 的 980px 断点仍无宿主。

---

## 8. 第三轮：**跨平台交互统一**（账号页为主）

§7 只做了「单页对齐原型」，没做本次重构真正的目标 —— **用原型统一 WorkBuddy 与 Trae 两个平台的交互**。
用户明确指出后，逐项核对两个账号页，把「同一件事两种做法」全部收敛。

原型本身是**平台无关**的：只有一个 `viewAccounts()`（`:2403-2444`）、一个 `accountCard()`（`:2490-2536`）、
一个 `tableView()`（`:2538-2561`）、一个 `batchBarHtml()`（`:2446-2457`），靠 `curPlatform().programs`
取当前平台的程序位表。也就是说：**同一段代码 + 不同数据**。本仓却是两个独立页面各自演化。

### 8.1 修正的差异（全部「同一件事两种做法」）

| # | 维度 | 改造前 | 现在 |
|:-:|:---|:---|:---|
| 1 | **Trae 卡片整卡可点** | 只有「更多 → 查看详情」能打开详情 | 与 WB 同：`tabIndex` + `onClick` + Enter/Space，内部控件用 `closest` 守卫 |
| 2 | **Trae 卡片当前登录信号** | 只有头部底色 + 幽灵水印 | 补「当前登录」徽章（与昵称同行）+ 左侧 3px 竖条 + 选中态高亮，与 WB 逐项一致 |
| 3 | **Trae 卡片头像** | 48px 圆形、只取 1 字 | 32px / 圆角 10px / 12.5px / 2 字，与 WB 同（`AVATAR_TEXT_CLASS` + `accountInitials` 已导出共用） |
| 4 | **Trae 卡片指标行** | 无 | 补 `.acct-metrics` 同形指标行（积分 / 到期 / 最后更新） |
| 5 | **Trae 卡片头** | `min-h-[104px]` 硬撑 + `border-b` 分隔线 + `px-5` | 与 WB 同：`min-h-[32px] px-3.5 py-2`、无分隔线、正文 `px-3.5` |
| 6 | **Trae 卡片页脚** | 程序位 + 「最后更新」文字 | 只留程序位（与 WB 同）；「最后更新」上移到指标行 |
| 7 | **程序位行单一定义** | Trae 叫 `programControls`、WB 叫 `programRow` | 统一 `programRow`（门禁对两张卡用**同一套判据**） |
| 8 | **Trae 页大横幅** | 「添加与迁移账号」大卡（OAuth/导入/粘贴 JWT/导入备份/导出备份） | 删除；动作进工具栏 `addAction` + `secondaryActions`（与 WB 同槽位） |
| 9 | **Trae 页环境卡** | 一张 `Card`（版本/运行态/签到数/积分/冷却/数据目录） | 压成**一行细条**（与 WB 的 CLI 提示条同形） |
| 10 | **Trae 列表可见标题** | `h2 账号` + 计数徽章 | 删除（原型列表区无标题）；语义交给 `aria-label`，计数在页头副标题 |
| 11 | **Trae 分组筛选** | 挂在列表标题旁 | 移入工具栏 `children` 槽位（该槽位的设计用途就是平台专属控件） |
| 12 | **Trae 批量条** | 缺「导出」 | 补 `onExport`（导出**所选**账号，与 WB 同）；批量条动作集两端完全一致 |
| 13 | **Trae 表格** | 账号列无头像 | 补头像（共享 `AccountsTableAvatar`：26px / 圆角 8px / 2 字，原型 `:2551`） |
| 14 | **WB 表格头像** | `rounded-full` + 1 字 | 同上，与 Trae 共用同一实现 |
| 15 | **Trae 抽屉** | 只注入 packs/sections/programs/remark | 补 `current` / `switchTargets` / `actions` / `checkinHistory` / `checkin`（5 项），抽屉两平台同构 |
| 16 | **Trae 错误态** | Alert 只有文字，无出口 | 补「重新检测」出口（与 WB 同） |
| 17 | **骨架屏** | WB 3×`rounded-lg` 固定列 / Trae 顶部条 + `auto-fit` `rounded-2xl` | 共用 `AccountsSkeleton`（列数 / 圆角 / 间距与真实栅格逐值一致） |
| 18 | **批量签到结果条** | WB 只有 toast / Trae 一张页面底部大卡 | 共用 `CheckinResultBar`，**位置 = 工具栏与列表之间**（原型 `#batch-slot`），两端同形 |
| 19 | **分组标题** | 两端都是 `h3` + 数量 `Badge`（与原型**反了**） | 共用 `AccountsGroupHeader`：组名进 `badge`、数量是右侧小字（原型 `:2471`） |
| 20 | **`featuresDisabled`** | WB 传 `false`、Trae 传 `api.isDemoMode()` ⇒ 演示构建里 **WB 的按钮能点、Trae 的点不动** | 两端统一 `false`（演示构建的写操作由 `api.ts` 统一拦截并给可读原因） |

新增共享件：`src/components/account-list-parts.tsx`（骨架 / 分组标题 / 批量签到结果条 / 表格头像）、
`src/lib/checkin-history.ts`（`localDateKey` / `lastSevenDays`，两平台签到历史同口径）。

### 8.2 保留的平台差异（**是数据/能力差异，不是做法差异**）

| 项 | 原因 |
|:---|:---|
| Trae 排序少「最近使用」 | `TraeAccount` 无 `lastUsedAt` 字段 |
| Trae 状态桶含「冷却」 | Trae 有冷却机制，WB 没有 |
| Trae 工具栏多一个「分组筛选」 | Trae 有命名分组（WB 无），走 `children` 插槽 |
| Trae 抽屉无「分组」区 | Trae 分组是 `groupId` 非自由文本（抽屉内已注明） |
| Trae 抽屉无「使用向导切换」 | 会话复制只有 WorkBuddy 具备（§3.15.1） |
| WB 列表下方多「客户端限流」区 | WB 独有数据源（PRD S4 已定挂载位置） |
| 程序位数量不同 | WB 5 个 / Trae 4 个（含跨区域消歧） |
| 卡片 chips 集不同 | WB 有旅行/优先/工具数；Trae 有冷却/分组/JWT 自动刷新 |

### 8.3 验证

| 项 | 结果 |
|:---|:---|
| `tsc --noEmit` | ✅ 0 error |
| `check:scope` | ✅ **273 / 0** |
| `check:api` | ✅ routes=166 |
| `check:store-selectors` | ✅ |
| `npm run build` | ✅ |
| **CDP 跨平台逐值对比** | ✅ `h1` / 无可见列表标题 / 无横幅 / 卡片 `tabIndex`+`role` / 头像 32px·10px·12.5px / 页脚 `DIV` 包装 / 页头无分隔线 —— **逐项相同** |
| **CDP 抽屉 + 批量条对比** | ✅ 批量条动作集**完全相同**（刷新/批量签到/导出/删除/清除选择）；抽屉区块骨架相同（切换到此账号 / 积分·资源包 / 签到历史（近 7 天）/ 程序位（只读）/ 操作） |
| **门禁可证伪** | ✅ 把 `requestSwitchTo` 里的 `setSwitchConfirm` 去掉 ⇒ 门禁**变红**且逐字复现；还原后 `sha256` 与变异前**完全一致**、变异标记 0 |

截图：`logs/verify-2026-10-10/{parity-wb-accounts,parity-trae-accounts,parity2-wb-drawer,parity2-trae-drawer}.png`。

### 8.4 ★ 本轮踩到的两个坑（都会静默造假）

1. **★★ 门禁的注释剥离器是朴素正则，行注释里的 `/*` 会吞掉半份文件。**
   `shared.zh.ts` 里有一行 `// ---- 桌面悬浮窗（components/companion/*，独立窗口）----` ——
   那个 `/*` 被 `stripComments` 当成块注释起点，一直吞到文件里**下一个 `*/`**。
   我新增的块注释恰好提供了那个 `*/` ⇒ **一次性删掉 250 行词条**，门禁报「词条缺 shared.drawer.checkin.title」。
   修法：① 去掉行注释里的通配（`companion/`）；② 词条文件里一律用 `//` 行注释。
2. **CDP 只改 hash 不会重新挂载 SPA** ⇒ 截到的图与打印的 DOM **全是上一页的**。
   必须 `Page.navigate` 到真实路径整页加载（本次因此先得到过一份「两页一模一样」的假数据）。

### 8.5 仍未做

- Trae 表格**没有**「今日 Token」列：Trae 无按账号的今日 token 数据源 ⇒ 按「无数据不渲染」**不加**空列（与 Trae 不渲染「模型/会话」视图同一原则）。

---

## 9. 第四轮：抽 `<AccountWorkbench>` 容器（把「两份实现」变成「一段代码」）

§8 把 20 项差异逐条对齐了，但两个页面**仍是两份 JSX** —— 只是暂时长得一样。这一轮把布局与交互
真正收成一份：`src/components/account-workbench.tsx`（326 行）。

### 9.1 边界

| 属于容器 | 属于页面 |
|:---|:---|
| 首屏骨架 / 首屏空态分支 | 页头（标题 + 作用域 chip） |
| 提示条（`notices`）的位置 | **取数**（两个平台的数据源完全不同） |
| 工具栏位置 + 全部工具栏 props | 筛选 / 排序 / 分组的**数据**（`sortOptions` / `groupOptions`） |
| 结果条 + 批量条的位置与渲染 | 平台专属控件（`toolbarExtra` / `secondaryActions` / `addAction`） |
| 两种列表空态的分支 | 卡片与表格列的**内容**（`renderCard` / `columns`） |
| 分桶循环 + 分组标题 + 网格/表格切换 | 抽屉装配 |
| 列表下方插槽（`footer`） | 列表下方的平台专属区块内容 |

容器内**禁止** `platform === …`；也**禁止**做 `filter/sort`（`buckets` 已是最终结果）——
容器一旦「顺便筛一下」，两个平台的口径就又会分叉。

### 9.2 ★ 顺带修掉的真实缺陷（不是重构引入的）

**Trae 卡片的「当前登录」徽章 / 左侧竖条 / 头部高亮在国内版作用域下永远不亮。**

根因：`logins` 的键是**程序位标识**（`trae_work` / `trae_cn` / `global` / `global_trae_code`），
而页面写的是 `logins[variant]`，`variant` 是**区域**（`cn` / `global`）。
国际版恰好有一个程序位就叫 `global` ⇒ 那边**碰巧**正常；**国内版两个程序位叫
`trae_work` / `trae_cn`，永远查不到** ⇒ 国内版三张卡全不亮。

实测（同一份演示数据）：

| 作用域 | 主号卡「当前登录」 |
|:---|:---|
| `/s/trae-global/accounts` | ✅ 有 |
| `/s/trae-cn/accounts` | ❌ 无 → 修复后 ✅ 有 |

修法：改为**逐程序位判定** `programsFor(account).some(p => p.current)`，
与 WorkBuddy 卡片 `isCurrentAccount`（「绑定任一程序位即当前」）**同一口径**，
并与抽屉的 `detailIsCurrent` 共用同一个 `isCurrentAccount`。
新增门禁钉住（`logins[variant]` 一出现即变红），**已做变异验证**。

### 9.3 验证

| 项 | 结果 |
|:---|:---|
| `tsc --noEmit` | ✅ 0 error |
| `check:scope` | ✅ **274 / 0**（新增 1 条 Trae「当前登录」门禁） |
| `check:api` / `check:store-selectors` | ✅ |
| `npm run build` | ✅ |
| CDP 两平台逐值对比 | ✅ `h1` / 无横幅 / 主操作在工具栏 / 文档顺序 `[toolbar, cards]` / 卡片 `tabIndex`+`role` / 头像 32px·10px·12.5px / 页脚 `DIV` —— **逐项相同** |
| CDP 批量条 | ✅ 动作集完全相同（刷新/批量签到/导出/删除/清除选择） |
| CDP「被筛没」空态 | ✅ 两平台同文案 + 同「清除筛选」出口 |
| 门禁可证伪 | ✅ 把 `isCurrentAccount` 改回 `logins[variant]` ⇒ 门禁变红且逐字复现；还原后 `sha256` 与变异前**完全一致**、变异标记 0 |

截图：`logs/verify-2026-10-10/{wb-workbench-accounts,trae-workbench-accounts}.png`。

## 9.4b 用户报障：卡片「内容被遮挡」+「有些大有些小」

两个问题都先**量出来**再改，不靠肉眼。

### ① 卡片不等高 —— 栅格多了 `items-start`

实测（1440px 宽，演示数据）：

| 页面 | 卡片高度 | 不同高度数 |
|:---|:---|:---|
| Trae 账号 | `406 / 381 / 361` | **3** |
| WB 账号 | `359 / 334 / 334` | **2** |

根因：卡片栅格写了 `items-start`。原型 `.grid { display:grid; gap:12px }`（`:242`）**没有**
`align-items` ⇒ 默认 `stretch` ⇒ **同行等高**（卡片是 `flex-col` + 正文 `flex-1`，页脚自然落在底部对齐）。

去掉 `items-start` 后：Trae `406/406/406`、WB `359/359/359` —— **各只有 1 种高度**。

### ② 徽章内容被裁 —— `Badge` 的图标选择器失配 + preflight 的 `svg{display:block}`

实测 WB「已签到」徽章：`clientHeight 18` 但 `scrollHeight 25` ⇒ **被裁 7px**；内部 svg 是 **16px**（应为 11px）。

根因链（两步）：

1. `Badge` 为了「长文本出省略号」把 children 包进了内层 `truncate` span
   ⇒ 图标**不再**是 `Badge` 的直接子级 ⇒ `[&>svg]:size-[11px]` **失配**，图标回到 16px；
2. Tailwind preflight 给 `svg` 设了 `display:block`，而内层 span 是 `block`
   ⇒ 图标与文字**上下堆叠成两行**（实测内层 33px），塞进 `h-5`（内容区 18px）被 `overflow-hidden` 硬裁。

修法（`src/components/ui/badge.tsx`）：`[&>svg]` → **`[&_svg]`**（后代）+ `[&_svg]:inline`
（抵消 preflight 的 block）+ `[&_svg]:shrink-0`。修后：svg 11px、内层 17/17、徽章 59/59 —— **零裁切**。

**全站回归抽查**（`Badge` 是全局组件，30 个文件在用）：9 个页面（两平台 × 账号/概览/用量/网关/设置）
共 **65 个徽章，被裁 0 个**，图标尺寸统一 11px。
**省略号仍生效**（这是内层 span 存在的理由）：注入超长文本后 `textOverflow:ellipsis`、
内层 `scrollWidth 352 > clientWidth 149`、徽章自身不溢出。

### 新增门禁（2 条，均做变异验证）

| 门禁 | 变异后 |
|:---|:---|
| 卡片栅格**不得**含 `items-start` + 必须是原型三列断点 | 加回 `items-start` ⇒ **变红** |
| `Badge` 图标必须用 `[&_svg]` + 有 `size-[11px]` + `inline` | 改回 `[&>svg]` ⇒ **变红** |

两次变异还原后 `sha256` 与变异前**完全一致**、变异标记 0。

截图：`logs/verify-2026-10-10/{cards-wb,cards-trae}.png`（改前 / 改后同名覆盖，最终态）。

---

### 9.4c 用户报障：去掉 Trae 账号页的两块

用户圈了两处要求删除，都已删除：

| 块 | 内容 | 处置 |
|:---|:---|:---|
| 顶部「客户端环境说明行」 | 客户端版本 / 运行状态 / 今日已签 / 可用积分 / 今日新增 / JWT 临期 / 冷却中 / 客户端数据目录 | 整块删除（含其 `mb-4` 细条容器） |
| 底部「上次签到」卡 | 上次签到时间 + 逐账号结果 + 冷却明细 | 整卡删除 |

页面骨架因此回到原型的**页头 → 工具栏 → 卡片网格**（中间不再有任何说明条）。

**连带清理**（本仓约定：不留未被引用的词条）：删掉 25 个只剩空引用的文案键
（`client` / `runtime` / `checkedTodayLabel` / `availableCredits` / `todayEarned` /
`jwtExpiring` / `cooling` / `clearCooldownAll` / `dataDirLabel` / `lastTitle` / `lastCooldown*` …），
以及随之失效的 16 个局部变量与 5 个导入。
⚠️ `dataDirMissing` **保留** —— 它仍被首屏空态 `EmptyTraeCard` 引用。

**⚠️ 一并消失的能力（如实列出，需要就说一声，我挪个位置）**：

1. **「全部清除冷却」** —— 原在环境条里（一次清掉所有账号的冷却）。**单账号**的「解除冷却」
   仍在卡片「更多」菜单里，未受影响。
2. 客户端版本 / 运行状态 / 数据目录的**页内**展示 —— 顶栏仍有「运行中」胶囊；
   首屏空态（无账号时）仍会显示数据目录。

### 9.4d 用户报障：国内版不显示国际版的程序位（**决策反转**）

**原设计**（P4 §1.4 / §5）：跨区域**常显** + 显示层加区域标识消歧 ——
任意区域页面都列出 4 个程序位，跨区域那两枚可见但不可直接切（点了只是引导切区域）。
实际效果：国内版页面上一直挂着「TraeWork AI 国际版 / Trae AI 国际版」两枚按钮。

**新要求**（用户 2026-10-10）：国内版不显示国际版的 ⇒ 改为 `programsFor` **按当前区域过滤**。

| 项 | 处置 |
|:---|:---|
| `TraeAccountsPage::programsFor` | 加 `.filter(status => status.variant === variant)` —— 只返回本区域程序位 |
| 卡片的跨区域分支 | **删除**（`crossRegion` / `regionLabelOf` / `regionHint` 与两枚引导按钮）—— 永远走不到，留着是死代码 |
| 卡片 props | 删 `currentRegion` / `onSwitchRegion`；页面不再取 `useTraeVariant` 的 setter |
| 抽屉「切换到此账号」 | 删 `crossRegion` 分支（不可切换只剩「没装 / 没有可写数据目录」一种原因） |
| 文案 | 删 `trae.comp.card.switch.regionHint` + `guideRegion`（后者此前就已无引用） |

**⚠️ 仍然保留**：`TraeProgram.region` 字段、`programsFor` **禁止按 `program` 去重** ——
`region` 仍是抽屉徽标的 key 组成部分，且「按 `program` 去重会丢客户端」这条依然成立。

**门禁**：原「跨区域按钮必须带区域标识」那条的**前提已反转**，整条重写为
「账号页只渲染本区域程序位」，三条判据（按区域过滤 / 卡片无跨区域分支 / 切换目标仍是 `program.variant`）。
**已做变异验证**：删掉 `.filter(status => status.variant === variant)` ⇒ 门禁变红且逐字复现；
还原后 `sha256` 与变异前**完全一致**、变异标记 0。

**CDP 实测**：

| 作用域 | 页面出现「国际版」字样 | 卡片程序位 |
|:---|:---|:---|
| `/s/trae-cn/accounts` | **false** | `TraeWork` / `TraeCode` |
| `/s/trae-global/accounts` | true | `TraeWork AI` / `Trae AI` |

卡片仍同行等高（374×3），「当前登录」徽章仍正常。

---

### 9.4e 用户报障：紧凑模式「用户名被截断 + 内容溢出卡片」

**实测（1440px 视口 / 卡片 373px / 紧凑模式）——改前**：

| 指标 | 值 | 含义 |
|:---|:---|:---|
| `h3` 宽度 | **0px** | 名字完全看不见（用户看到 `ne…`） |
| `h3` 截断 | true | — |
| 头部 `scrollWidth / clientWidth` | **371 / 343** | **溢出卡片 28px** |

根因：紧凑头部把「勾选 + 名称 + 徽章 + 状态标签 + 程序位行」全塞进**一行**。
程序位行（最多 6 枚按钮 ≈188px）与状态标签都是 `shrink-0` **不可压缩**，
而名称是 `flex-1 min-w-0` ⇒ 全部亏空由名称吃，吃到 0 之后行仍然溢出。

**修法**：紧凑头部改为**两行**（两张卡同构）：

```
第 1 行：[☐] [名称 —— 独占整行] [当前登录]
第 2 行：[状态标签]            [程序位（外层 flex-1 min-w-0 接管剩余宽度）]
```

- 名称那行只剩两个尺寸固定的小件（勾选框 18px、徽章），名称因此拿到几乎整行。
- 状态标签**必须**离开名称行：实测留在同一行时长名字只剩 113~142px（它们 `shrink-0`）。
- 程序位外层加 `flex-1 min-w-0` ⇒ `programRow` 自带的 `w-full` 等于「这个容器的 100%」，
  按钮在容器内换行，**顶不出卡片**。

**改后实测**：

| 名称 | 名称宽度 | 是否截断 | 头部溢出 |
|:---|:---|:---|:---|
| `new-account-2026-10`（19 字符） | 209–277px | **否** | 否 |
| `mr-long-name@example.com`（23 字符） | 209–277px | **否** | 否 |
| 26 个汉字的昵称（压测用） | 277px | 是（有 `title` 提示） | **否** |

⇒ 真实长度的昵称全部完整显示；只有极端长昵称才截断，且**任何情况下都不再溢出卡片**。

**新增门禁**（第 3 条「紧凑卡片」）并做变异验证：
把状态标签挪回名称那一行 ⇒ **变红**。
⚠️ 首版判据写得太松（`[\s\S]{0,N}</div>[\s\S]{0,M}{statusChips}`）被**第二行**的
`{statusChips}` 兜住、照样绿 —— 改成「**第一个** `{statusChips}` 之前是否已闭合一行」才有效。
还原后 `sha256` 与变异前**完全一致**、变异标记 0。

---

### 9.4f 用户报障：Trae 工具栏「需要和 workbuddy 一样的交互」

用户圈了整个 Trae 工具栏。**先量控件集**（CDP 按 DOM 顺序列出工具栏内全部控件），
实测两个工具栏的**唯一控件集差异**：

| | WB | Trae |
|:---|:---|:---|
| 搜索 / 排序 / 分组 / 视图 / 全选 | ✅ | ✅ |
| 两个开关 | 自动签到 / **自动旅行** | 自动签到 / **跳过已签到**（平台差异） |
| 密度 / 回收站 / 刷新 | ✅ | ✅ |
| 次级操作 | 导入本机账号 / 导入备份 / 导出备份 | 导入本机账号 / **粘贴 JWT** / 导入备份 / 导出备份 |
| 主操作 | 添加账号 | OAuth 网页登录 |
| **额外控件** | — | **`全部账号（N）` 分组筛选下拉** ← 唯一多出来的 |

⇒ **删除**那个「分组**筛选**」下拉（WB 与原型都没有它，且它把工具栏挤成两行）。
连带清掉 `groupFilter` / `groupFiltered` / `UNGROUPED` 三个状态与四条文案。

**分组能力未丢**：卡片上仍有分组标签，`分组 = 按自定义组` 仍能把列表按分组**分桶**；
`overview.groups` 仍供「添加账号」抽屉选用。

**改后实测**：两个工具栏**行数相同（6 行）**、控件位置逐一对应。
仍保留两处**数据差异**（不是做法差异）：第二个开关的语义（自动旅行 / 跳过已签到）、
Trae 多一个「粘贴 JWT」图标（OAuth 不可用时的兜底路径，WB 无等价物）。

**新增门禁**（第 4 条）并做变异验证：把 `groupFilter` 加回 Trae 页 ⇒ **变红**。
还原后 `sha256` 与变异前**完全一致**、变异标记 0。

---

### 9.4 关于代码量（诚实说明）

| 文件 | 前 | 后 |
|:---|--:|--:|
| `AccountsPage.tsx` | 2715 | 2670 |
| `TraeAccountsPage.tsx` | 2475 | 2702 |
| `account-workbench.tsx` | — | 326 |

**总行数没有下降**（约 +180），因为 Trae 页这一轮同时补了抽屉装配（§8 的 5 个 prop）
与批量条状态。收益不在行数，而在**结构**：布局与交互现在只有一份实现，
两页剩下的都是「取数 + 装配 props」，不会再各自漂移。

---

## 10. 账号卡片完全对齐原型（2026-10-12）

用户要求「**完全按照 `buddy-switch-prototype.html` 的账号卡片和交互来改**」。
参照物 = 原型 `accountCard()`（`:2496-2535`）+ `.acct*` CSS（`demo.css:294-343`）。

### 10.1 原型的卡片只有**四个块**

```
[可选 span[data-current-bar]]   ← 当前账号左侧 3px 竖条（.acct.is-current::before）
header   (.acct-top)            ← 勾选 + 头像 + 名称/身份 + 「当前登录」徽章
div      (.acct-chips)          ← 状态标签行（flex-wrap, gap:5px）
section  (.acct-metrics)        ← 指标行（一行小字）
footer   (.acct-actions)        ← 程序位行(.progs) + 「...」更多菜单
```

`.acct` 根上统一 `display:flex; flex-direction:column; gap:10px; padding:13px 14px`
⇒ 各块**不再自带** `px-*` / `mt-*`。

### 10.2 本轮删掉的四处（都是「原型没有」）

| 删除项 | 原型依据 |
|:---|:---|
| 头部**底色带**（`bg-primary/5` / `bg-muted/30`） | `.acct-top` 是**透明的**，整张卡只有一个底色；底色带把它变成「页眉」、把卡切成两半 |
| 头部**幽灵水印**（`WorkBuddyMark` / `TraeMark`） | `.acct-top` 内没有装饰层 |
| 卡片正文的**大数字行**（`Sparkles` + 22px 余额 + 「N 个积分包」+ 「更新于 hh:mm」） | `.acct-metrics` 只是一行 11.5px 小字，没有大字号层级 |
| **「近期到期」逐包列表**（剩余徽标 / 包名 / 到期日 / 比例条） | 原型卡内没有该块；逐包明细改由**详情抽屉**承载（WB / Trae 抽屉都已读 `creditPackages`） |

### 10.3 移动的两处

- **`.acct-chips` 从头部搬出来**，成为 `header` 的**兄弟块**（此前塞在头像右侧，
  把头部撑成三行、`min-h` 顶到 104px）。
- **`...` 更多菜单从头部搬到页脚**（原型 `.acct-actions` = `.progs` + 更多按钮），
  头部因此不再需要 `pr-10` / `pr-[112px]` 让位。

### 10.4 改后实测（CDP，1440px 视口 / 卡片 371px）

| 项 | WB | Trae |
|:---|:---|:---|
| 卡片直接子块 | `[span#bar] header div section footer` | 同左 |
| 头部底色 | `rgba(0,0,0,0)`（透明） | 同左 |
| 头部绝对定位子件 | 0 | 0 |
| 含「大数字行 / 近期到期」 | false | false |
| 页脚含「更多」按钮 | true | true |
| 同行卡片高度 | 202 / 202 / 202 | 259 / 259 / 259 |
| 卡片横向溢出 | 0 | 0 |
| 紧凑模式头部行数 | 2 | 2 |
| 紧凑模式名称宽度 | 249 / 317 / 317（零截断） | 同左 |

两平台**块序列完全一致**，差异只剩数据：WB 指标行 = 积分 / 今日 Token / 最后使用；
Trae = 积分 / 到期 / 最后更新（Trae 无「今日 Token」数据源 ⇒ 不摆永远「—」的假指标）。

### 10.5 顺带清掉的级联死代码

删块后按本仓「不留死代码」纪律清理（`tsc` 的 **TS6133** 就是清单）：

- `account-card.tsx`：`formatCreditUpdatedAt` / `expiryClass` / `creditResources` 三个 helper、
  `visibleResources` / `expiringAmount` / `resources` 局部变量、`Clock3` / `Sparkles` 图标导入、
  `CreditResource` 类型导入、`creditUpdatedAt` prop（`Props` + 解构 + 调用点）。
- `trae-account-card.tsx`：`expiryTone` helper、`visiblePackages`、`Clock3` / `Sparkles` / `TraeMark` 导入。
- `stores/accounts.ts`：`creditUpdatedAtMap` **整链删除**（`RegionSlice` / `emptySlice` /
  `selectRegionSlice` / `AccountsState` / 删除账号分支 / 拉取积分 setter）——它只有卡片一个消费者。
- 词条：`wbAccounts.card.*` 9 条 + `trae.comp.card.*` 9 条（中英各一份）。

### 10.6 门禁改动

| 门禁 | 动作 |
|:---|:---|
| `★ 原型卡片四块结构`（**新增**） | 覆盖**两张卡**：根上内边距/块间距 · 头部无底色 · chips 独立块 · 页脚 = progs + menu · 不得回归「大数字行 / 近期到期」 |
| `★ B3 当前账号信号` | **前提被反转** ⇒ 整条重写：原断言「头部底色用 `isCurrentAccount`」在底色被删后失效，改为「竖条 + 徽章同读 `isCurrentAccount` **且**头部不得再有底色」 |

**变异验证**（三条，均逐字复现）：① 给 Trae 头部加回 `bg-muted/30` ⇒ 红；
② 把 WB 的 chips 块停用 ⇒ 红；③ 把 WB 竖条改读 `codebuddyCliActive` ⇒ 红。
三次还原后 `sha256` 与变异前**逐字节一致**、变异标记 0。

`tsc` 0 error · `check:scope` **279/0** · `check:api` 166 · `build` ✅

---

## 11. 账号页 1:1 对齐原型（2026-10-12，第二轮）

用户要求「**认真对比，1:1 还原**」（圈了工具栏与卡片网格）。

### 11.1 方法：逐属性 DOM diff，不靠肉眼

把原型（`file://…/buddy-switch-prototype.html?dest=accounts&hint=0&scope=wb-cn`）与本仓
（`/s/wb-cn/accounts`，演示构建）都渲染在 **1440px** 下，用同一套选择器提取
**24 类元素的 22 个计算样式属性**（几何 / 圆角 / 内边距 / 间距 / 字号 / 行高 / 边框 / 底色 / 字色），
逐键 diff。差异分两类：**样式微差**（直接改）与**本仓多出来的控件**（改了就丢功能，先问）。

### 11.2 用户拍板的三件事

| 问题 | 选择 |
|:---|:---|
| 工具栏多出的 8 个控件（自动签到 / 自动旅行 / 密度 / 回收站 / 刷新 / 导入本机 / 导入备份 / 导出备份） | **收进右侧「⋯」溢出菜单**（工具栏回到一行，入口保留） |
| 网格下方的「客户端限流台账」整块（原型没有） | **保留** |
| 卡片页脚程序位按钮的动态文案（「设为当前」/「CLI 当前」） | **改成固定短名 + ✓**（原型做法） |

### 11.3 改动清单

**工具栏（`account-toolbar.tsx`）**

| 项 | 原型 | 改前 | 改后 |
|:---|:---|:---|:---|
| 行数 / 高度 | 1 行 **56px** | **2 行 100px** | 1 行 **56px** |
| 搜索框 | `label.field`：图标与输入**同处一个 32px 字段**（`gap 6` / `padding 0 10` / 圆角 9 / `card-2` 底） | 图标绝对定位 + input 自带边框、`pl-8`（左内边距 32 而非 30） | 同原型 |
| 排序 / 分组 | 字段内**先写「排序」/「分组」再跟下拉**，宽 138 | 只有下拉，宽 150 | 带前缀文字，**136** |
| 视图切换 | `.seg button` **纯图标**，38×26、`padding 0 11` | 图标 + 文字「卡片」「表格」，62×26 | **38×26、纯图标**（文字移到 `title`/`aria-label`） |
| 全选 | `btn ghost sm` **文字按钮** 46×28 | 勾选框 + 文字 | **44×28 文字按钮**（`aria-pressed` 承担勾选态） |
| 主操作 | `btn primary` 98×34、`padding 0 13`、圆角 10、12.5px | 尺寸对但内边距 11（`Button` 对带图标者会收窄） | **98×34、`padding 0 13`** |
| 溢出菜单 | 原型没有 | 无 | 「⋯」36×28（`aria-label="更多操作"`） |

**卡片（`account-card.tsx`）**

| 项 | 原型 | 改前 | 改后 |
|:---|:---|:---|:---|
| 徽章 | 圆角 **6** / 字号 **10.5** / `padding 0 7` / **无边框** | 10 / 11 / `0 6` / 1px 透明边框 | **6 / 10.5 / 0 7 / 0** |
| 「优先」标签 | **带文字**的 `badge info` | 只有一枚星形图标 | **文字「优先」** |
| 「使用中 N 个工具」 | `badge info` + ✓，排在 **chips 第一枚** | `badge secondary`，排在**最后** | 同原型（位置 + 配色） |
| 「N 积分 M 天后过期」 | `badge warn`（原型 `.acct-chips` 第三条） | **没有这一枚** | 新增（`expiringSoonRemaining` + `soonestExpireAt`；拿不到就不渲染） |
| 状态标签 | `accountStatus()` **互斥的一枚**（凭据已过期 → 需重新登录 → 今日未签到 → 已签到） | 「已签到」与「需重新登录」**并列**（自相矛盾） | **互斥**，文案对齐（「今日未签到」「凭据已过期」） |
| 指标行 | `gap 14` / **不换行** | `gap 4px 14px` / `flex-wrap` | 同原型 |
| 第二格标签 | 「今日」 | 「今日 Token」 | **「今日」** |
| 第三格 | **无标签的相对时间**（`2 分钟前`） | 「最后使用 10-12 17:10」 | **相对时间**（共享 `formatRelativeTime`），绝对时刻挂 `title` |
| 名称 / 身份行高 | 19.5 / 16.5（继承 1.5） | 20 / 20（`leading-5`） | **19.5 / 16.5** |
| 勾选框 | 17×17 | 18×18 | **17×17** |
| 「更多」按钮 | `btn ghost sm` 36×28 | 32×32 | **36×28** |
| 程序位按钮 | `✓ + 短名`（`WB`/`IDE`/`CLI`/`VSC`，**无产品图标**） | 产品图标 + 动态文案（「设为当前」/「CLI 当前」） | **固定短名 + ✓**（产品图标只在紧凑模式保留） |

**顺带修掉的真缺陷**：`<ProductCurrentState … compact />` 是 JSX 简写 = `compact={true}`
⇒ 三个「当前登录」程序位**永远**渲染成紧凑态（只剩图标），「✓ + 短名」从来没出现过。改成 `compact={compact}`。

### 11.4 改后实测（CDP，1440px）

| 元素 | 原型 | 改后 | |
|:---|:---|:---|:---|
| 工具栏 | 1144×56 · `pad 10px 12px` · 圆角 12 | 1144×**56** · 同 | ✅ |
| 搜索字段 | 295×32 · 圆角 9 · `pad 0 10` · `gap 6` | 296×**32** · 同 | ✅ |
| 排序 / 分组字段 | 138×32 | 136×**32** | ✅ |
| 视图 seg / 按钮 | 82×30 / 38×26 · `pad 0 11` | **82×30 / 38×26 · `pad 0 11`** | ✅ |
| 全选 | 46×28 · 圆角 8 · `pad 0 10` · 12px | **44×28 · 同** | ✅ |
| 添加账号 | 98×34 · 圆角 10 · `pad 0 13` · 12.5px · primary | **98×34 · 逐值相同** | ✅ |
| 卡片 | 373×170 · `pad 13px 14px` · `gap 10` | 373×**195** | ⚠️ 见下 |
| 卡片头部 | 343×36 | **343×36** | ✅ |
| 名称 / 身份行高 | 19.5 / 16.5 | **19.5 / 16.5** | ✅ |
| 勾选框 | 17×17 | **17×17** | ✅ |
| 徽章 | 圆角 6 · 10.5px · `pad 0 7` · 无边框 | **逐值相同** | ✅ |
| 指标行 | `gap 14` · nowrap | **同** | ✅ |
| 页脚 / 更多按钮 | 343×39 / 36×28 | **343×39 / 36×28** | ✅ |
| 程序位按钮 | 51×26（当前态）· `gap 4` · `pad 0 8` | **51×26 · 同** | ✅ |
| 程序位短名 | — | `WB / IDE / CLI / VSC / JB` | ✅ |
| 卡片块序列 | `[竖条] header chips metrics actions` | **同** | ✅ |
| 头部底色 | 透明 | **透明** | ✅ |

**⚠️ 卡片高度 195 vs 170 的唯一原因**：chips 行折了 2 行（45px）。
本仓演示账号有 **5 枚** chips（多一枚「旅行中」图标标签——原型没有旅行概念），
且积分数位更长（`386.4 积分 6 天后过期` vs `320 积分 3 天后过期`）⇒ 总宽 **354 > 343**。
**去掉那枚旅行标签后总宽 330 < 343 ⇒ 单行 ⇒ 卡片高度正好 170**，与原型一致。
即：布局与样式逐值相同，差异来自**数据**（多一个本仓独有的状态 + 更长的数字）。

### 11.5 Trae 侧同步

- 指标行同步为 `gap 14` / 不换行 / 无下边距；第三格改**相对时间**（共享 `formatRelativeTime`）。
- 修掉「**到期 10/03 到期**」重复词：新增 `formatExpiryDate`（只到日），标签与值不再各带一次后缀。
- 徽章几何同样交回 `Badge` 基础样式（本主题 `rounded-md` = 10px，不是原型的 6px）。
- 工具栏同样一行 56px、同样「⋯」溢出菜单、同样纯图标视图切换。

### 11.6 门禁

| 门禁 | 动作 |
|:---|:---|
| `★ 账号页工具栏 = 原型一行结构`（**新增**） | 搜索框是 `label.field` 结构 · 排序/分组带前缀文字 · 视图**纯图标** · 全选是 ghost 按钮（不是勾选框）· 「⋯」存在且顺序为「⋯ → grow → 全选 → 主操作」· 主操作 `padding 13px` |
| `★ 紧凑卡片：名称必须独占一行` | **前提被新代码抢先匹配**：卡片里出现了别的 `{compact ? (` 三元（`ProductCurrentState` / `pluginControl`）⇒ 正则命中错误片段。改为**先锚定 `<header>`** 再取紧凑分支 |
| `★ 「今日 Token」诚实边界` | 第三格由 `formatDate` 改 `formatRelativeTime` ⇒ 断言同步（并要求走**共享**实现） |

**变异验证**（三条，均逐字复现）：① 视图按钮加回文字 ⇒ 红；② 全选改回勾选框 ⇒ 红；
③ 第三格改回绝对时间 ⇒ 红。三次还原后 `sha256` 与变异前**逐字节一致**、变异标记 0。

`tsc` 0 error · `check:scope` **280/0** · `check:api` 166 · `build` ✅

---

## 12. 演示数据对齐原型 + 三处真缺陷（2026-10-12 第三轮）

用户：「**你做的这个卡片和原型的 [图] 完全不一样**」。

### 12.1 先量，别猜

把原型与本仓**同一视口（1440px）**渲染，裁到**卡片网格**整体比对。量出来的结论：

- **卡片组件本身是逐值对齐的** —— 结构（四块）、几何（373×170）、chips 集合与顺序、
  指标行、页脚全部一致（§11 已逐属性核过）。
- 真正「看起来完全不一样」的是**演示数据**：本仓演示只有 **3 个**账号（测试 A/B/C）、
  状态只覆盖「已签到」，而原型是 **8 个**账号、覆盖
  「当前登录 / 需重新登录 / 凭据已过期 / 今日未签到 / 优先 / 临期积分」全部状态。
- 另有 **3 处真缺陷**（与数据无关，下面 12.3）。

### 12.2 演示数据与原型**逐账号对齐**

`src/lib/screenshot-demo.ts` + `shared.*.ts`：

| 项 | 改前 | 改后 |
|:---|:---|:---|
| 账号数 | 3（测试 A/B/C） | **8**（jackey / team-shared / backup-01 / qa-bot / design-lab / ops-02 / spare-global / demo） |
| 邮箱 | `test-*.example.com` | 与原型逐条一致（`jackey@` / `shared@` / `backup01@` …） |
| 积分 | 小数（`5,378.4/9,100`） | 原型值（`1,240/2,000` …），jackey 拆两包使「临期 320 + 总额 2,000」同时成立 |
| 今日 Token | 只有 2 个账号 + `gatewayTruncated: true` ⇒ 卡片出现 `今日 —` / `≥N` | **8 个账号全给 + `gatewayTruncated: false`** ⇒ 干净的 `42.1K` / `0` |
| 最后使用 | 绝对日期 | 相对时刻（`2 分钟前` / `昨天` / `12 天前`） |
| 状态覆盖 | 只有「已签到」 | 需重登（spare-global）/ 凭据过期（design-lab）/ 今日未签到（qa-bot、design-lab）/ 优先（jackey） |
| 旅行 | 每账号按 `index % 3` 造「旅行中/已结束/无 Buddy」 | **统一 `untraveled`**（默认态不渲染 chip，见 12.3） |
| 程序位当前账号 | CLI / IDE / VSC 全指向 jackey ⇒ 「使用中 4 个工具」 | CLI→ops-02、VSC→team-shared ⇒ jackey「使用中 **2** 个工具」（与原型同值） |

**改后实测**：两边卡片网格都是 **1164×555**；8 张卡的
`chips` / `指标行` / `状态标签` / 头像**配色与文字逐条一致**，单卡 **373×170**（与原型同值）。

### 12.3 顺带修掉的三处真缺陷

1. **旅行标签是一枚「只有图标、没有文字」的徽章** —— 原型 `.acct-chips` 里
   **没有任何无文字的 chip**，那枚空标签看起来就像坏掉了；而且它把 chips 行挤成两行
   （卡片 170 → 195px）。⇒ 短文字（`旅行中` / `已旅行`）进徽章，细节（地点·奖励·剩余）留在 tooltip。
2. **`travelChip` 的 `untraveled`（默认态）也渲染 chip** ⇒ 真实数据下**每张卡**都挂一枚
   「未旅行」灰标签。⇒ 默认态**不渲染**（只有真在旅行 / 刚结束 / 无 Buddy 才有信息量）。
3. **头像取色的哈希与原型不同源**：原型每步 `% 997`，我们用 `>>> 0`（保留 32 位）
   ⇒ 同一个名字算出**不同的颜色**（实测 qa-bot 原型是紫、我们是红）。⇒ 改为逐步 `% 997`。

### 12.4 仍然存在的三处差异（都不是「卡片做错了」）

| 差异 | 说明 |
|:---|:---|
| **卡片顺序** | 我们的「积分优先级」= **紧迫度**（临期程度 → 最早到期 → 临期额度）；原型 = **手动优先标记 + 积分降序**。规则不同 ⇒ 8 张卡的排列不同（内容仍逐张一致）。 |
| **第 5 枚程序位按钮 `JB`** | JetBrains CodeBuddy 插件 **2026-10-09 才落地**（`registry.ts` 里已从 `planned` 转为 `available`），原型早于它。这是**真实能力**，不是多画了一枚。 |
| **「当前」程序位的分布** | 我们的模型是「**每个程序位只有一个当前账号**」⇒ 只有 3 张卡亮绿。原型是**演示夹具**，把 `ide: true` 同时标在 4 个账号上（真实产品里不可能）。 |

### 12.5 一并修的数据管道缺陷

- `hydratedAccounts()` **无条件覆盖 `expiresAt`** 成未来时刻 ⇒ 种子里写的「已过期」
  被吃掉，「凭据已过期」这个状态**永远演示不出来**。改为「种子给了非 0 值就尊重它」。
- `creditExpiry` 的 `expireAt` 用 `futureAt(days, 23, 59)` ⇒ 卡片算出的天数是
  `ceil(3天13小时)` = **4 天**（原型写 3）。改为 `futureAt(days, 0, 0)` ⇒ 正好 `days` 天。
- 相对时间补 **「昨天」** 一档（24–48h 此前显示「1 天前」），
  `formatRelativeTime` 三处调用方（账号卡 / Trae 卡 / 网关来源条）共用同一口径。
- 清理孤立词条：`shared.demo.travel.cafe` / `gym`。

`tsc` 0 error · `check:scope` **280/0** · `check:api` 166 · `build` ✅

---

## 13. Trae 侧完全对齐（2026-10-12 第四轮）

用户：「**traework 和 workbuddy 完全对齐原型**」。

### 13.1 先量：Trae 差在哪

同一视口（1440）量两边：

| | 原型 `trae-cn` | 我们（改前） |
|:---|:---|:---|
| 账号数 | 5（jackey / code-lab / solo-test / idle-01 / snap-acc） | 3（主号 / 小号 A / 小号 B） |
| 卡片 | **373×170** | 373×**243**（chips 折成两三行） |
| chips | 使用中 N 个工具 / 状态 / N 积分 M 天后过期 / 优先 | JWT 状态 / 已签到 / 冷却·时间戳 / 分组 / 支持刷新 JWT（**5~6 枚**） |
| 指标行 | 积分 X/Y · **今日 N** · 相对时间 | 积分 X · 到期 MM/DD · 相对时间 |
| 页脚 | `Work` `Code`（纯文字、方角 8、高 26） | `TraeWork` `TraeCode`（胶囊、18px 图标） |

### 13.2 卡片：完全照原型的槽位与形状

**chips**（`trae-account-card.tsx`）改成原型 `.acct-chips` 的**固定四槽**：

```
使用中 N 个工具 → 状态 → X 积分 N 天后过期 → 优先
```

Trae 的真实信号**没有丢**，只是换进原型规定的槽位：
- JWT 过期 / `SessionDead` 冷却 ⇒ 第 2 槽「状态」（凭据已过期 / 需重新登录）
- 冷却的**时间与原因** ⇒ 该标签的 tooltip
- 分组 / 自动刷新 JWT ⇒ 详情抽屉（chips 行是固定四槽，不塞第五枚）

**指标行**：补 `积分 X/Y`（Y = 各包 `total` 之和）。
**程序位按钮**：改成与 WB 同形的 `.prog`（`[✓] 短名`、方角 8、高 26、字 11），
短名固定 `Work` / `Code`。

**结果**：Trae 卡片 **373×170**，与原型**同尺寸**；网格两边都是 **1164×373**。

### 13.3 演示数据逐账号对齐

`screenshot-demo.ts` + `shared.*.ts`：5 个账号（jackey / code-lab / solo-test / idle-01 / snap-acc），
名字 / 积分 / 临期 / 签到 / 最后使用全部照抄；分组照原型（常用 / 测试 / 闲置）。
每个账号拆两个积分包，使「剩余 880」与「临期 140」同时成立（单包做不到）。

### 13.4 顺带修掉的四处真缺陷

1. **`creditsExpireAt` 单位写错**：字段是**秒**（`formatExpiryDate` 会 `*1000`），
   演示夹具传的是**毫秒** ⇒ 卡片显示成五万年后的日期（实测「到期 03/25」）。
2. **包到期日按 23:59 取整**：`futureAt(N)` 默认 23:59 ⇒ 卡片算出的天数是
   `ceil(N天13小时)` = **N+1**（原型写 5 天，我们显示 6 天）。改 0 点后正好 N 天。
3. **`trae-account-card` 的「更多」按钮还是 32px**（WB 侧上一轮已改成 28）⇒ 页脚 43 而非 39。
4. **`trae-account-card` 的 h3/p 还留着 `leading-5`**（20px）⇒ 头部比原型高 3.5px。

### 13.5 门禁：把「代理信号」收紧成真正的反模式

`★ P4 programsFor 不许按 program 去重` 原判据是「函数体内出现 `.program` 即报警」——
那是**代理信号**。而卡片按钮的可见短名（`Work` / `Code`）**必须**从 `program` 取：
`variant`（`trae_cn` / `trae_code` …）在 `variant.rs` 里是 `TraeRegion × TraeProgram` 的
**朴素笛卡尔积命名、语义不唯一**，`label` 又是展示名（`TraeWork` / `Trae AI`）——
两者都会把 Code 判成 Work（实测按钮显示成 `Work Work`）。

⇒ 判据收紧为**真正的去重构造**（`new Set/Map(… .program)` 或 `.filter/some/find(… .program)`），
误报消失、真反模式仍被拦。

### 13.6 仍存在的三处差异 —— 都是**数据模型缺口**，不是卡片做错

原型对两个平台用**同一个** `accountCard()` + 同一套字段
（`name`/`email`/`credits`/`creditsTotal`/`expiring`/`expireInDays`/`todayTokens`/`priority`）。
Trae 侧缺三样：

| 缺什么 | 现状 | 影响 |
|:---|:---|:---|
| **email** | `RawAccount`（`trae/account.rs:35`）只有 `name`/`user_id`/`jwt`/`refresh_token`/时间戳/`remark`；OAuth 回调**见过** `email` 但**没落库** | 身份行只能显示 `UID · x`，不是掩码邮箱 |
| **逐账号今日 Token** | `token_stats.rs::get_statistics(days)` 是**滚动窗口**（`now - days*24h`），不是「本地日」；没有 WB 那样的 `token_daily` 日库 | 指标行第二格只能用 Trae 真有的「到期 MM/DD」 |
| **优先标记** | Trae 账号库没有 `priority` 字段 | 第 4 槽「优先」不渲染 |

**排序规则**（两平台同因）：我们的「积分优先级」= **紧迫度**（临期程度 → 最早到期 → 临期额度）；
原型 = **手动优先标记 + 积分降序**。规则不同 ⇒ 卡片排列不同（内容仍逐张一致）。

补齐前两项需要在 Rust 侧加字段 / 加命令（含 server 路由 + tauri 命令 + `api.ts` 四处 + 契约门禁）。

`tsc` 0 error · `check:scope` **280/0** · `check:api` 166 · `build` ✅

---

## 14. 「卡片还是和原型差很多」的真因：存量密度偏好没迁移（2026-10-12 第五轮）

用户贴了**真实 app** 的截图（jackey / 花 / Andev 三个账号）：程序位按钮挤在**头部右上**、
**没有页脚**，且按钮叠成 2~3 行。

### 14.1 复现：那是**紧凑卡**

同视口（1303px，与用户截图同宽）渲染演示数据、分别强制两种密度：

| 密度 | 卡片结构 | 与截图 |
|:---|:---|:---|
| `compact = true` | `[竖条] header section` —— **无页脚**，程序位在头部 | **完全一致** |
| `compact = false` | `[竖条] header chips metrics footer` | 原型形态 |

⇒ 用户看到的不是「卡片做错了」，而是**卡片停在紧凑形态**——而原型里**只有宽松那一种**。

### 14.2 根因：改了默认值，但没迁移存量值

`useCompactMode()` 的存储是**裸 `"1"` / `"0"`**。上一版把默认值从「开」改成「关」，
但老用户 localStorage 里躺着**旧默认写下的 `"1"`** ⇒ 他们永远停在紧凑形态，
**改默认值对他们等于没改**。

**修法**：存储改成 `{"v": 2, "compact": bool}`，**旧格式一律当作「未设置」** ⇒ 落回新默认值
（宽松）并就地升级；用户**在改版之后**主动切过的选择不受影响。

为什么不是「把旧值一律读成 `false`」：那会让「关」与「未设置」不可区分，
下次再改默认值时又会重蹈覆辙 —— **版本号才是能反复用的判据**。

实测三种存储值：

| 存储值 | 渲染 |
|:---|:---|
| 旧格式 `"1"` | **宽松卡**（迁移生效） |
| `{"v":2,"compact":true}` | 紧凑卡 |
| `{"v":2,"compact":false}` | 宽松卡 |

### 14.3 顺带修掉紧凑卡的布局缺陷

紧凑头部第 2 行原本是「状态标签 + 程序位」**共处一行**，且标签那层写的是
`min-[420px]:flex` —— 那是**视口**媒体查询、**不是卡片宽度**：
宽视口 + 窄卡片时标签照样显示，等于没有保护。

实测紧凑卡宽 324~410px，而「最多 4 枚标签（≈330px）+ 5 枚程序位按钮（≈154px）= **484px**」，
一行根本放不下 ⇒ 按钮被挤成 **2+2+1 三行**、还盖住标签（就是用户截图里那团）。

**修法**：标签与程序位**各占一行**（紧凑头部 = 名称 / 标签 / 程序位 三行）。
两平台同改（`account-card.tsx` + `trae-account-card.tsx`）。

### 14.4 门禁

新增 `★ 卡片密度偏好：存储必须带版本号（改默认值必须能被存量用户看到）`：
禁止再直接比对裸 `"1"`、要求存在 `STORAGE_VERSION` 且读取时校验版本。
**变异验证**：改回 `localStorage.getItem(...) === "1"` ⇒ 红且逐字复现；还原后 281/0。

`tsc` 0 error · `check:scope` **281/0** · `check:api` 166 · `build` ✅

---

## 15. 严格一致性审计（2026-10-12 第六轮）

用户要求「**严格检查是否一致**」。做法：同视口（1440）、两边都强制成原型形态（宽松），
取 **26 个元素 × 24 个计算属性**（`getComputedStyle` + `getBoundingClientRect`，
颜色经 canvas 归一以消除 `oklch` / `rgb` 写法差异）**逐键 diff**，两平台各跑一遍。

### 15.1 本轮修掉的 6 处**真差异**

| # | 元素 | 原型（`demo.css`） | 改前 | 改后 |
|:--|:---|:---|:---|:---|
| 1 | 程序位按钮（可切换） | `.prog`（`:326-332`）= **透明底** + `--border` 边框 + **字重 400** + `muted-fg` | `Button variant="outline"` 自带 `bg-background` / `border-input` / `font-medium` | 逐项压回（`bg-transparent border-border !font-normal text-muted-foreground`） |
| 2 | 徽章 `line-height` | `.badge`（`:244-248`）**没写** line-height ⇒ 继承 1.5（15.75px） | `leading-none`（10.5px）⇒ 文字基线差 2.6px | 去掉 `leading-none`（几何归基础样式） |
| 3 | 全选按钮 | `.btn.ghost` = `.btn` 的 1px 边框 + 透明色 ⇒ **46×28** | 无边框 ⇒ 44×28 | 补 `border border-transparent` |
| 4 | 「更多」按钮 | `.btn ghost sm` ⇒ **36×28** | `sm` 变体自带 `has-[>svg]:px-[8px]`（**与它自己的注释矛盾**）同特异性压过我们的 `px-[10px]` ⇒ 28×26 | `!px-[10px]` + 图标回到基础样式的 14px |
| 5 | 视图切换（激活态） | `.seg button.on` = **font-weight 550**（`:281`） | 500 | `data-[on=true]:font-[550]` |
| 6 | Trae「当前登录」徽章 | `.badge.info` = primary **16%** | 写死 `bg-primary/15` | `bg-primary/16` |

### 15.2 审计结果

- **逐键一致**（盒尺寸 + 全部 24 项）：工具栏、卡片、卡片头部、名称、身份行、指标行、
  指标数值、页脚、全选、当前程序位、可切换程序位 —— **两平台各一份**。
- 主题 token 亦逐值相同（`--border` = `oklch(1 0 0 / 10%)`、`--card` = `oklch(0.279 0.041 260.031)`）。

### 15.3 仍存在的差异（**已全部归类，无未知项**）

| 类别 | 项 | 说明 |
|:---|:---|:---|
| **有意为之** | 搜索框窄 20px | 我们的工具栏多一枚「⋯」溢出菜单（2026-10-12 用户批准），搜索框 `flex:1` 因此少分 20px |
| | Trae 无「添加账号」 | Trae 的账号来自客户端导入 / OAuth，没有「添加」这一动作 |
| | Trae 身份行 `UID · x` | Trae 账号库（`RawAccount`）**没有 email 字段** |
| | Trae 指标行第二格是「到期 MM/DD」 | Trae **没有逐账号「今日」口径**（`get_statistics` 是滚动窗口） |
| | Trae 无「优先」槽 | Trae 账号库没有 `priority` 字段 |
| | 程序位多一枚 `JB` | JetBrains 插件 **2026-10-09 才落地**（`registry.ts` 从 `planned` 转 `available`），原型早于它 |
| **实现等价** | `display: grid → flex`、`justifyContent/alignItems: normal → center` | 头像 / 徽章 / 程序位的居中写法不同，**渲染结果相同** |
| | seg 按钮 `gap: normal → 6px` | 按钮里只有图标，gap 不参与布局 |
| | `.btn.primary` 的 1px 透明边框 | 盒尺寸已相同（98×34），有边框的靠内边距让位 |
| | 边框色差异 | 全部落在 **`border-width: 0`** 的元素上（seg 按钮 / 徽章 / 芯片）⇒ **不可见** |
| **亚像素** | 带图标的徽章宽 ~1.1px | 文本度量取整差异 |
| **排序规则** | 卡片顺序 | 我们的「积分优先级」= **紧迫度**（临期程度 → 最早到期 → 临期额度）；原型 = **手动优先标记 + 积分降序**。规则不同 ⇒ 排列不同（**内容逐张一致**） |

`tsc` 0 error · `check:scope` **281/0** · `check:api` 166 · `build` ✅

---

## 16. 全站逐页审计（2026-10-12 第七轮）

用户要求「**整体一项一项严格检查是否一致，并修正**」。做法：把原型的**六个目的地**
（概览 / 账号 / 用量 / 会话 / 网关 / 设置）逐页 dump `main` 的结构（标签 + 类 + 盒尺寸 + 文本）
两两比对，再对可疑块逐属性核。

### 16.1 本轮修掉 5 处（其中 3 处是**系统性**的）

| # | 问题 | 原型 | 改前 | 影响面 |
|:--|:---|:---|:---|:---|
| 1 | **页头主操作按钮** | `.btn`（`:191-196`）= **高 34** / 内边距 0 13 / 圆角 10 / 字号 12.5 | `size="sm"`（**高 28**） | 概览 / 用量 / 会话三个页头 |
| 2 | **视图切换组** | `.seg`（`:275-281`）= `padding:2px; gap:2px; border-radius:10px`，按钮 `h26 / padding 0 11 / radius 8`，激活态 `bg-card + 字重 550` | `TabsList` = `h36 / padding 4 / radius 12 / 无 gap`，`TabsTrigger` = `h28 / padding 12 / radius 10 / 字重 500` | **共享组件**（用量页 / Trae 用量页 / 限流卡 / 技能卡 / 集成指南 5 处调用点） |
| 3 | **用量页切换组的额外外边距** | `.seg` 是 `.page` 的**直接子级**，间距只来自 `.page-head` 的 `margin-bottom:20px` | 多套一层 `mt-4 mb-4` ⇒ 与容器 `gap-5` 叠加成 **36px** | 用量页 |
| 4 | **卡片底色** | `.card { background: var(--card) }` —— **不透明** | `bg-card/70`（实测 `oklab(… / 0.7)`） | 统计卡 / Trae 积分总览 / Trae Token 页（4 处） |
| 5 | **统计卡行高** | `.stat .k` / `.stat .v` **都没写 line-height** ⇒ 继承 1.5（17.25 / 30） | `leading-4`（16）/ `leading-7`（28） | 统计行（概览 + 用量共用 `StatTiles`） |

**实测效果**：概览的统计行 **103 → 106**，与原型**逐值相同**；用量页切换组 **36 → 30** ✓。

### 16.2 逐页审计结果

| 目的地 | 页头 | 主要区块 |
|:---|:---|:---|
| **概览** | `1144×67` **✔** | 统计行 `106` **✔**（修后）；「待处理」卡 `235` vs 原型 `308` ✗；split `372` vs `348` ✗ |
| **账号** | `1144×67` **✔** | 工具栏 `56` **✔** · 卡片网格 `535` **✔**（逐卡 373×170 已在 §15 逐键核过） |
| **用量** | `1144×67` **✔** | 切换组 `30` **✔**（修后）；子视图内容结构不同（见下） |
| **会话** | `1144×67` **✔** | 列表 `100` **✔**；工具栏 `54` vs `52`（2px） |
| **网关** | `1144×67` **✔** | 内容 `1023` vs `772`（分区更多） |
| **设置** | `1144×67` **✔** | 内容 `803` vs `503`（分区更多） |

### 16.3 仍存的差异 —— 分两类

**A. 内容 / 功能差异（不是样式错）**
- 我们多一条**固定的「快速上手」提示**（`1440×74`，仅首次运行显示），原型无。
- 账号页多一张**「客户端限流台账」卡**（`251`），原型无。
- 网关 / 设置的分区**比原型多**（功能更多）⇒ 页面更长。
- 概览「待处理」卡：原型的行是**两行**（`.srow` 的 `.lab b` 标题 + `.lab span` 副标题，行高 65），
  我们是一行（47）⇒ **行内信息结构不同**，不是行高 token 错。

**B. 待逐块对齐（需要各自单独核）**
- 概览 split 块 `372` vs `348`（`split-main` `227` vs `209`）。
- 会话页工具栏 `54` vs `52`。
- 用量页子视图：原型把各块**平铺**在 `.page` 下，我们用一个大容器包住（外层 1144×1111）⇒
  需要逐个视图对齐。

`tsc` 0 error · `check:scope` **281/0** · `check:api` 166 · `build` ✅

## 17. 「⋯」溢出菜单并入「添加账号」抽屉（2026-10-12 第八轮）

**用户要求**：把工具栏那枚「⋯」合并到「添加账号」的右侧弹出栏内，**外面那枚菜单不用存在了**，
`workbuddy` 与 `traework` 交互保持一致；**已经存在的功能不重复搬**。

### 17.1 为什么原先有「⋯」，现在为什么能删

第 11 轮为了把工具栏压回原型的**一行 56px**，把本仓多出来的 8 个入口收进一枚溢出菜单
（原型工具栏右侧只有 `全选` + `+ 添加账号`）。代价是工具栏**永远比原型多一枚控件**。
本轮把这 8 个入口整体搬到「添加账号」抽屉 ⇒ 工具栏与原型**逐控件一致**，「⋯」删除。

### 17.2 落点（8 个入口的去向）

| 原「⋯」里的入口 | 去向 |
|:---|:---|
| 从本机导入 / 导入备份 | **不搬** —— 抽屉的前两张 `.opt` 卡本来就是它俩 |
| 自动签到 / 第二枚开关 / 卡片密度 | 「更多操作」分区的**开关行**（`Switch`） |
| 回收站 / 导出备份 / 刷新积分 | 「更多操作」分区的**动作行**（整行可点，与 `.opt` 同形 + `ArrowRight`） |
| （Trae 独有）粘贴 JWT | 抽屉第 4 张添加方式卡（`extraOptions`） |

### 17.3 实现要点

- `account-toolbar.tsx`：删 `children` / `secondaryActions` 两个平台插槽 + 整个 `DropdownMenu`
  ⇒ 工具栏只剩 搜索 / 排序 / 分组 / 视图 / `grow` / 全选 / 主操作。
- `account-workbench.tsx`：同步删 `toolbarExtra` / `secondaryActions`。
- `add-account-drawer.tsx`：新增 `extraOptions`（额外添加方式卡）与 `moreTools`（更多操作分区）。
  `DrawerTool` **两种形态二选一** —— 开关行走 `control`，动作行走 `onClick`（整行可点）。
  两平台**同一个组件、同一套结构**，差异只有数据。
- **Trae 侧补齐同一入口**：原先 `addAction` 直接开 OAuth 弹窗（按钮文案「OAuth 网页登录」），
  现在与 WB 一样开抽屉（文案统一成「添加账号」）—— 这是「交互一致」的**必要**改动，
  否则 Trae 没有抽屉可落这些入口。
- **词条搬迁**：抽屉是共享组件 ⇒ 文案从 `wbAccounts.addDrawer.*` 挪到 `shared.addDrawer.*`，
  两平台共用（否则两边各写一份同样的句子，必然漂移）。顺手删掉随之变成死词条的
  `*.page.demoDisabled` / `compactToLoose` / `compactToCompact` / Trae 的 `importLocal` 等
  （中英各若干），以及 `shared.toolbar.moreAria`。

### 17.4 核验

- **门禁**：两条前提被反转的老门禁整条重写（「工具栏 = 原型一行」改成**反向**钉「不得再有
  「⋯」/ 平台插槽」；「两平台控件集一致」改成钉「两页都无插槽 + 都用同一个抽屉」），
  新增一条「抽屉 = 原「⋯」内容的唯一落点（6 行 · 键序一致 · 分区有标题 · 文案同源）」。
  `check:scope` **283/0**（原 280/2 失败 → 重写后全绿）。
- **变异验证**：5 条变异（加回 `Ellipsis` / 加回 `secondaryActions` / 改 `moreTools` 键 /
  文案写回平台域 / 抽屉不渲染 `moreTools`）**全部逐字复现**，还原后 `sha256` 一致。
- **CDP 实测**（`logs/verify-add-drawer.mjs`，两平台各跑一遍，17/17 通过）：
  工具栏 `1128×56`、按钮集 = `排序方式 / 分组方式 / 卡片视图 / 表格视图 / 全选 / 添加账号`
  （**无「更多操作」**）；抽屉宽 `460`、标题「添加账号」、「更多操作」6 行（3 开关 + 3 动作）；
  Trae 多第 4 张卡「粘贴 JWT」。截图：`logs/add-drawer-wb-cn.png` / `logs/add-drawer-trae-cn.png`。
- ⚠️ **踩坑**：Pages 演示构建走 **HashRouter**（`pagesDemoHostingEnabled`），作用域在 **hash** 里；
  写成 `/s/trae-cn/accounts` 会静默落到默认作用域（渲染出 WorkBuddy）。
  另外演示构建里主操作被 `DemoAction` 置 `disabled`，而 React 合成事件**按 props 判断** disabled
  ⇒ 摘 DOM 属性 + `click()` 无效，必须直接取 `__reactProps$…` 调 `onClick` 才能开抽屉。

`tsc` 0 error · `check:scope` **283/0** · `check:api` 166 · `check:store-selectors` ✔ ·
`check:npm-platforms` ✔ · `build` ✅ · `build:demo` ✅

## 18. 「回收站 / 紧凑模式」放回工具栏（2026-10-12 第九轮）

**用户要求**：第 17 轮把 8 个入口全塞进抽屉后，用户点名「**回收站和紧凑模式放出来**」。

### 18.1 落点调整

| 入口 | 第 17 轮 | 本轮 |
|:---|:---|:---|
| 紧凑模式（卡片密度） | 抽屉「更多操作」的开关行 | **回工具栏**（视图切换之后的图标钮，仅卡片视图下渲染） |
| 回收站 | 抽屉「更多操作」的动作行 | **回工具栏**（视图切换之后的图标钮） |
| 自动签到 / 第二枚开关 / 导出备份 / 刷新 | 抽屉「更多操作」 | **不动**（仍在抽屉里，4 行） |

### 18.2 实现要点（为什么用**结构化 prop** 而不是恢复插槽）

第 17 轮刚删掉 `children` / `secondaryActions` 这种 **ReactNode 插槽**（它是「平台专属控件」
的载体，一放开就会重新长出平台分支）。本轮的两枚控件**两平台都有**，因此走**结构化 prop**：

```ts
compactToggle?: { compact: boolean; onToggle: () => void };
onOpenRecycleBin?: () => void;
```

几何（`size-[34px] rounded-lg`）与文案（`shared.toolbar.compactToLoose` / `compactToCompact`）
**都留在工具栏壳里** ⇒ 两平台不可能漂移。词条随之从 `shared.addDrawer.compactMode*`
挪回 `shared.toolbar.compactTo*`。

### 18.3 ⚠️ 36px 会把工具栏顶到 58（本轮实测踩到）

两枚图标钮最初沿用旧的 `size-9`（36px）⇒ 它成为行内最高子件，工具栏 `56 → 58`
（36 + 20 padding + 2 border）。改成 **`size-[34px]`** 后回到 `56`（与主操作 `.btn` 同高）。
门禁已钉：工具栏里出现 `size-9` 即红。

### 18.4 核验

- **门禁**：工具栏那条改名并补三条判据（两枚常驻控件必须有**结构化 prop** · 密度必须
  `effectiveView === "card"` 才渲染 · **不得出现 `size-9`**）；「两平台控件集一致」补
  「两页都必须注入这两枚」；抽屉那条改成 **4 行** + 新增「抽屉里**不得**再有紧凑模式 / 回收站」
  （同一个开关出现在两处必然打架）。
- **变异验证**：5 条（删某一页的 `compactToggle` / 去掉卡片视图限制 / 改回 `size-9` /
  抽屉里又冒出紧凑模式 / 抽屉少一个入口）**全部逐字复现**，还原后 `sha256` 一致。
- **CDP 实测**（`logs/verify-add-drawer.mjs`，27/27 通过）：两平台工具栏 `1128×56`、
  按钮集 = `排序方式/分组方式/卡片视图/表格视图/切换为紧凑模式/回收站/全选/添加账号`；
  **切到表格视图 ⇒ 密度按钮消失，切回卡片 ⇒ 回来**（决策 #5 实测）；
  抽屉 4 行（2 开关 + 2 动作），无「紧凑模式 / 回收站」。
- 截图：`logs/add-drawer-wb-cn.png` / `logs/add-drawer-trae-cn.png`。

`tsc` 0 error · `check:scope` **283/0** · `check:api` 166 · `build` ✅ · `build:demo` ✅


