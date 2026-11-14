# 升级前说明弹窗（版本内容）

> 状态：现行（2026-10-17 落地） · 原型：`docs/prototype/index.html?open=update`
> 实现：`src/components/update-install-dialog.tsx` / `release-notes-view.tsx` / `lib/release-notes.ts` /
> `lib/bundled-notes.ts` / `crates/buddy-switch-core/src/modules/update.rs` / `scripts/gen-release-notes.mjs`
> 相关：`scripts/gen-update-json.sh`、`.github/workflows/build.yml`、`docs/releases/`

---

## 1. 要解决的问题

用户原话：

> 目前每次升级都不知道这个版本的变化内容是什么，帮我补充个弹窗显示版本内容，升级前显示相应的版本信息

现状（改造前）有三处硬伤，缺一不可地导致「不知道改了什么」：

| # | 现状 | 后果 |
|:--|:--|:--|
| 1 | 侧栏 ↑ 只挂一个「有新版本」角标，点进去是**抽屉里挤出来的三行 bullet** | 说明沦为背景信息，用户没地方读长内容 |
| 2 | 点「立即更新」**立刻开始下载安装**（`UpdateInstallDialog` 挂载即跑 `downloadAndInstall`） | 不可逆动作发生在「读懂」之前 |
| 3 | 版本内容没有数据源（`gen-update-json.sh` 写的 `"notes": ""`） | 就算 UI 想做也拿不到内容 |

---

## 2. 结论（要做什么）

**一枚弹窗、四个阶段、先说明后动作。** 说明态是默认态，下载动作只能由用户点「立即升级」触发。

```
available ──「立即升级」──▶ downloading ──进度 100%──▶ ready ──「立即重启」──▶ 重启
    │                            │
    └──「稍后升级」/ Esc / 遮罩 ──┘（downloading 期间禁止关闭）
                                  └──失败──▶ error ──「重试」/「打开下载页」
```

不新增第二个弹层、不再用抽屉：抽屉是「看某个对象的细节」的容器，而这里是**一次等待用户决策的模态流程**（下载中必须占住界面、不能并发去点别的）。

---

## 3. 交互规格

### 3.1 状态表

| 阶段 | 标题 | 正文 | 页脚 |
|:--|:--|:--|:--|
| `available` | 发现新版本 **vX** | 必读（若有）→ 本版更新分组 → 更早版本（折叠） | 查看完整发布说明 · 稍后升级 · **立即升级** |
| `downloading` | 正在升级到 **vX** | 进度条 + 「期间不要退出应用」+ 说明内容仍可读 | 提示「下载完成后会提示你重启」+ 禁用「下载中」 |
| `ready` | **vX** 已就绪 | 「下载完成，重启后生效」（必读区块**不再出现**）+ 说明内容 | ✓ 已下载 · 稍后重启 · **立即重启** |
| `error` | 无法完成更新检查 | 只有「为什么失败 + 怎么办」，**不摆**昨天的说明与包大小 | 打开下载页 · 关闭 · **重试** |

### 3.2 版式与判据

| 决策 | 判据 |
|:--|:--|
| 600px 级居中弹窗，**页头 / 可滚正文 / 固定页脚**三段 | 说明可能很长；动作被长正文顶出屏幕 = 主操作消失 |
| **破坏性变更置顶**（`.upd-alert`，警示色 + `role="alert"`） | 端口 / 数据格式这类漏看一次就是「升级完直接用不了」，混在长列表里必被划走 |
| 分组用**左侧细线**而不是卡片堆叠 | 11 条以上内容时卡片会变成一堵墙；细线把「组」和「条」的层级压到一个视觉重量 |
| 头部写「当前 vA · 发布于 … · 27.4 MB」 | 用户判断的是「这次要不要现在装」，不是「有没有新版本」——版本号 + 日期 + 体积三件套才够 |
| 「更早的 N 个版本」默认**折叠** | 落后多版时才需要；默认展开会把本版说明推走 |
| 版本说明按**纯文本**渲染（`esc()` 后才进 innerHTML） | 内容来自发布资产（外部输入），不解析 markdown、不 `innerHTML` 原串 |
| 拿不到说明 → 显示「这次发布没有附带说明」 | 空标题假装「本版没有更新」比空着更坏 |
| 下载中：关闭按钮禁用 + Esc 无效 + 点遮罩无效 | 关掉 = 安装半途而废（Tauri updater 会中断） |
| 打开即聚焦主操作；失败态聚焦「重试」 | 键盘用户不该先 Tab 一圈找出口 |
| 进度**只刷两个节点**（宽度 + 百分比），不整块重画 | 整块 `innerHTML` 重写会把正文滚动位置打回顶部（本仓已出过同类事故） |

### 3.3 入口（两个，同一实现）

1. 侧栏版本行的 ↑ 按钮（有更新时才出现，沿用现状）；
2. 顶栏 ⚙「应用设置」抽屉 →「关于与更新」→「查看更新内容」。设置里查得到，才不至于让功能只活在一个角落图标里。

### 3.4 弹窗出现的时机

- **新版本第一次被发现时主动弹一次**（桌面端、且「自动检查更新」开着、且这一版没看过）。
  只挂角标**已被证明无效** —— 想知道的人不会去点，于是永远不知道改了什么（用户原话）。
- 检查时机（三处，同一个 `checkForUpdate`）：① 挂载时一次；② 每 30 分钟（`AUTO_CHECK_UPDATE_INTERVAL_MS`）；
  ③ **窗口重新可见 / 重新获得焦点时补一次**（`visibilitychange` + `focus`）。③ 是必要的：
  隐藏窗口里 `setInterval` 会被 WebView 节流，回到应用时结论可能是几十分钟前的陈值。
  三处都**不传 `force`** —— 节流交给后端成功结果缓存（`update.rs::CACHE_TTL_SECS`，5 分钟，
  且**必须短于轮询间隔**，见 2026-10-17 那次「6 小时缓存盖住 30 分钟轮询」的事故）。
- 关掉弹窗（含「稍后升级」）即记 `seenVersion`，同一版本不再拦人：
  「稍后升级」是正常决策，不是没看完，第二天再弹同一份内容是骚扰。
- 关掉之后角标还在，随时可以从角标 / 设置里再打开 —— 入口不是一次性的。

---

## 4. 数据契约

```ts
interface ReleaseNotes {
  version: string;          // 2026.10.171833（与 APP_VERSION 同格式）
  publishedAt?: string;     // ISO；前端按当前语言 + 本地时区显示
  size?: number;            // 字节；缺失就不显示体积，不猜
  releaseUrl?: string;
  breaking?: { title: string; desc?: string }[];  // 空数组 = 不渲染该区块
  sections?: { title: string; items: string[] }[];
  history?: { version: string; publishedAt?: string; sections: { title: string; items: string[] }[] }[];
}
```

`UpdateInfo.notes` 承载它；`null` = 没拿到（前端显示「这次发布没有附带说明」，**不**假装本版没有更新）。

### 4.1 来源（已拍板：A + B）

| 方案 | 做法 | 结论 |
|:--|:--|:--|
| **A. 发布资产 `release-notes.json`**（主源） | 发布时 `gen-release-notes.mjs --json` 产出，随 Release 上传；`update_check` 顺带抓 `releases/latest/download/release-notes.json` | ✅ 采用。结构稳定、不吃 GitHub API 配额（60 次/小时），与 manifest 同源 |
| **B. 打包内置 `src/data/release-notes.json`**（兜底） | 构建时 `npm run notes:json` 生成，随前端打包 | ✅ 采用，但**只在版本号对得上时**才用（等于 latest 或等于 current）。拿旧版说明冒充新版比不显示更坏 |
| C. updater manifest 的 `notes` | `gen-update-json.sh` 留空 | ❌ 只能是一整段纯文本，做不出分组 / 必读置顶 |
| D. GitHub Releases API 的 `body` | 直接取 Markdown | ❌ 配额 + Markdown→HTML 的注入面 |

> **红线**：前端不解析 markdown、不把 release body 原串塞进 `innerHTML`。分组在**发布侧**做完。
> 后端还会再洗一遍（`update::sanitize_release_notes`）：只保留白名单键、丢掉未知字段与超长条目、
> 并校验版本号等于本次检查到的 `latest`。

### 4.2 版本内容从哪来（发布侧）

`docs/releases/<tag>.md` 是唯一人工入口，优先级：

1. `docs/releases/<tag>.json`（人工结构化，要精细控制时才写）；
2. `docs/releases/<tag>.md` → 按 `## 标题` + `- 条目` 解析：标题含「必读 / 破坏性」的组进 `breaking`，
   **一组一条**（标题就是这一项的名字，组内 bullet 是它的说明）；其余组进 `sections`；
3. 提交本身（Conventional Commits 前缀分组，与自动正文同一套口径）。

`history` 由 `docs/releases/*.md` 全量解析（**只收比本次更早的版本**）—— 供「更早的 N 个版本」用。

### 4.3 「更早的 N 个版本」怎么切片

`history` 是全量数据，**切片在前端**（`lib/release-notes.ts::olderNotes`）：
保留 `current < v < latest` 的版本，最多 5 个。判据是「只有客户端知道自己装的是哪一版」——
落后 1 版不需要归档，落后 8 版需要知道漏了什么。

---

## 5. 落点（已实现，2026-10-17）

| 层 | 文件 | 改了什么 |
|:--|:--|:--|
| 前端 | `src/components/update-install-dialog.tsx` | 四阶段（`available` / `checking` / `downloading` / `ready` + `latest` / `error`）；**删掉打开即下载**，下载只由 `startInstall` 触发；下载中禁止关闭；失败态不摆旧数据 |
| 前端 | `src/components/release-notes-view.tsx`（新） | 必读置顶 + 分组 + 更早版本折叠；纯文本插值，无 Markdown 解析器 |
| 前端 | `src/lib/release-notes.ts`（新） | 白名单归一化（长度上限 / 数量上限）、版本比较、`olderNotes` 切片、日期与体积格式化 |
| 前端 | `src/lib/bundled-notes.ts`（新） | 内置兜底的**版本对齐**采用规则 |
| 前端 | `src/lib/update-preference.ts` | `seenVersion`（localStorage）+ `shouldAutoShowNotes`（纯函数，门禁可断言） |
| 前端 | `src/App.tsx` | 检查结果归一化 + 每版首次自动弹一次 |
| 前端 | `src/components/app-settings.tsx` | 入口文案改「查看更新内容」（点开是说明，不是直接下载） |
| 前端 | `src/components/ui/dialog.tsx` | 新增 `width="wide"`（620px）与 `flush`（三段式：页头 / 可滚正文 / 固定动作条） |
| 后端 | `crates/buddy-switch-core/src/modules/update.rs` | `release_notes_urls` / `sanitize_release_notes` / `fetch_release_notes`；`update_check` 主/兜底两路都带 `notes`；新增 2 个单测 |
| 脚本 | `scripts/gen-release-notes.mjs` | `--json`：人工稿解析 / 提交分组 / 历史版本；tag 默认值回落 Cargo 版本 |
| 脚本 | `scripts/gen-update-json.sh` | `notes` 保持空串并注明原因（结构化的走 `release-notes.json`） |
| 脚本 | `package.json` | `npm run notes:json` |
| CI | `.github/workflows/build.yml` | 版本戳后重生成内置说明；发布时把 `release-notes.json` 作为资产上传 |
| 门禁 | `scripts/check-scope-registry.cjs` | +5 条：effect 不许自动下载 / 说明态真的渲染 / 纯文本渲染 / 后端白名单 + 版本对齐 / 内置说明版本一致 |
| 原型 | `docs/prototype/*` | 四阶段弹窗 + 11 条行为断言 + 2 个变异（`updauto` / `updorder`） |

---

## 6. 决策记录（2026-10-17 拍板）

| 问题 | 结论 |
|:--|:--|
| 版本内容来源 | A（发布资产）+ B（内置兜底，版本对得上才用） |
| 「更早的 N 个版本」 | 保留；`history` 全量下发，前端切片（最多 5 个） |
| 首次发现新版本是否自动弹 | **弹一次**。角标单独存在已被证明无效；关掉即记 `seenVersion`，同版不再打扰 |
| 「跳过此版本」 | 不做。用户不装就继续有角标（与改造前一致），不引入第三种「已读」状态 |

---

## 7. 明确不做的

- **增量更新 / 部分功能热更**：说明再清楚也不改变「整包替换」的事实。
- **在弹窗里显示提交列表**：提交标题写给维护者，不是写给用户的。发布侧只放「一句话说清结果」。
- **把说明做成第二个抽屉**：同一份内容两套容器，必然漂移。
- **在弹窗里渲染 Markdown**：外部输入进解析器 = 自己开一个注入面。

---

## 8. 已知局限

- **Rust 侧已验证**（2026-10-17）：`cargo fmt --check` 干净、`cargo check --workspace` 0 告警、
  `release_notes` 两个单测通过。⚠️ 仓库另有 143 条**既有** clippy lint（`update.rs` 0 条），
  CI 的 Clippy 步骤显式 `RUSTFLAGS: ""` 豁免，不在本次范围。
- **「更早的版本」依赖 `docs/releases/*.md` 的存量**：目前仓库里只有 1 篇，所以现在基本看不到折叠区；
  每版补一篇人工稿，它才会长出来。
- **体积（`size`）暂由发布侧决定是否提供**：`latest.json` 的 manifest 不带包大小，
  只有下载开始后 updater 才报 `contentLength`。说明里没有 `size` 就不显示，不猜。
