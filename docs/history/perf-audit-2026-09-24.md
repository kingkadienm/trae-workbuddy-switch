# 性能审计报告 — Buddy Switch

> **状态：只读历史（不再更新，2026-10-17 归档）** —— 性能审计报告；代码注释仍引它作为「为什么这么改」的证据。
> 现行文档见 [`docs/README.md`](../README.md)（文档地图）。
> 审计日期：2026-09-24 · 范围：`src/`（React 前端）、`crates/buddy-switch-core`、`crates/buddy-switch-gateway`、`crates/buddy-switch-server`、`src-tauri`
> 方法：静态读码 + 本机实测。**每条结论都标注「已核实」的依据（文件:行号 / 实测数字）**；无法确证的归入 §5「待实测」。
> 约束：所有修复方案均以**不改变可观察行为**为前提，并逐条列出必须保留的语义与建议护栏。

> ## ⚠️ 已复核，请以复核版为准
>
> 本报告经**第二遍逐条复核**（查 git 引入提交 + 注释上下文 + 仓库内对照实现 + 测试），结论有实质修正。
> **复核结论见 [`history/perf-audit-2026-09-24-review.md`](perf-audit-2026-09-24-review.md)**，其中：
>
> - **★ 撤回**：P1-4「sticky 内存泄漏」**判错** —— `get`/`bind`/`unbind`/`gc` 生产代码零调用，
>   表恒为空，**不存在泄漏**；真相是「整套粘性机制未接线」。详见复核报告。
> - **下调严重度**：P0-1（调用路径是冷的，且已有 3s 缓存 + spawn_blocking 两道缓解）；
>   P0-2（桌面端本地加载，代价是解析而非下载，收益需先量化）。
> - **改判为「有意为之，不得擅动」**：M3（`freshMs=0` 是文档化的 SWR 语义）；
>   P1-6 的 2s 轮询本身（用户可开关的「日志实时跟随」功能）；P0-3 的「每次重同步」。
> - **需先向作者确认**：P0-4（注释把「不缓存」写成设计属性，但未给理由）。
> - **需分级处理**：P1-3（55 个同步命令中，只有做慢 IO 的那些该改，微秒级配置读取不必改）。
> - **证据强化**：P0-1 / P1-2 / P1-3 / P1-6 的「是遗漏而非有意」判定，均靠**仓库内已有的对照实现**确立。
>
> 下文保留原始分析（含被修正的表述），**判断请以复核报告为准**。

---

## 0. 结论速览

按「收益 ÷ 风险」排序，最值得做的六件事：

| # | 问题 | 量级 | 风险 | 复核后 |
|---|---|---|---|---|
| 1 | 进程检测每次起 N 个 `tasklist` 子进程 | 单次 208ms，Global 区域一轮 ≈ 564ms | 低（改动局限在 `process.rs`） | 真问题，但**路径是冷的** ⇒ 降为 P1 |
| 2 | 首屏单 chunk **1.40 MiB**，零路由懒加载 | 冷启动解析 1.4MB JS | 低 | 真问题，**影响需先量化** |
| 3 | Trae 网关**每个请求**重建整个账号池 | 3 次磁盘读 + N 次哈希派生 | 中（需保住「写即可见」语义） | 真问题；**重同步本身是有意设计** |
| 4 | token 统计每次**全量重扫 + 全量 JSON 解析** | 随日志量线性增长 | 中（需等价性护栏） | ✅ **已实施 2026-10-10（两轮）**：首扫 226s→79s；**重启后 127s→1.2s**（落盘缓存，见 P0-4 正文） |
| 5 | 静态资源**无任何缓存头** | 每次刷新重传 1.4MB | 低（但 index.html 必须 `no-cache`） | 真问题（从未考虑过） |
| 6 | WorkBuddy 网关每请求 **5 次全局写锁** | 全局串行化 | 中 | 真问题；**底层设计有意** |

**已核实且表现良好的部分**（不要动）：上游 `reqwest::Client` 全局复用（含连接池 `pool_max_idle_per_host(20)`）；`stores/resources.ts` 的 SWR 快照（同键去重 + 序号防旧写回 + 上限 32，未见缓存穿透）；`trae/pool.rs::pick` 选号是 O(n) 单遍无锁；`ApiServicePage` 已用 `Promise.all` 并行 7 请求；`use-cached-resource.ts` 的 `freshMs=0`（**有意为之的 SWR 语义**）。

---

## 1. 实测基线（本机，2026-09-24）

进程枚举各方案单次耗时（`Measure-Command`，取单次）：

| 方案 | 耗时 | 说明 |
|---|---|---|
| `tasklist /NH`（全量） | **208 ms** | 一次拿全部进程 |
| `tasklist /FI "IMAGENAME eq X" /FO CSV /NH` | **141 ms** | 当前实现用的形式 |
| `Get-Process`（WinAPI） | **33 ms** | 快 6.3×，且不产生子进程 |
| 启动 `powershell -NoProfile -Command` | **329 ms** | 最贵，仅弹窗/一次性场景可用 |

其他基线：

- `dist/assets/index-*.js` = **1,464,712 字节（1.40 MiB）**，CSS 86,561 字节。全部页面与 recharts 在同一 chunk。
- `src-tauri/src/commands.rs`：**81 个** `#[tauri::command]`，其中 **26 个 async / 55 个同步**。
- `spawn_blocking` 出现次数：`buddy-switch-core` **0**、`buddy-switch-gateway` **0**、`buddy-switch-server/api.rs` **15**。
- 前端 `React.memo` / `memo(` 使用次数：**0**。

---

## 2. P0 — 收益最大，优先处理

### P0-1 进程枚举：每次检测起 N 个 `tasklist` 子进程

> **复核定性（详见复核报告）**：**确认为遗漏，非有意为之** —— 同一团队已在
> `trae/platform.rs:1079-1083` 做过同样优化并写明理由（「后者在装了多个渠道时要起 4 个子进程，这里只起 1 个」），
> 由 `19287bb` 引入；`process.rs` 这段来自初始提交 `461827e`，属**改 Trae 侧时漏改 WorkBuddy 侧**。
> **但严重度下调为 P1**：调用路径是**冷的**（仅状态探测 + 切换/结束流程），
> 且服务端已有 3s 缓存（`server/api.rs:28-46`）、Tauri 侧 `get_status` 已 async + spawn_blocking
> （`commands.rs:88-94`，注释写明就是为这个子进程）。**最有价值的是 `wait_windows_pids_gone`**，
> 因为它直接决定切换要等多久 —— 优先级应排在 `windows_workbuddy_process_rows_for` 之前。

**证据（已核实）**
- `crates/buddy-switch-core/src/modules/process.rs:519-525` — `windows_tasklist_image_rows(image)` 每次调用起一个 `tasklist` 子进程。
- `process.rs:530-540` — `windows_workbuddy_process_rows_for` **对每个映像名各调一次**：CN 区域 2 个名 → 2 次；**Global 区域 4 个名（`WorkBuddyAI` / `CodeBuddyAI` / `WorkBuddy AI` / `CodeBuddy AI`，见 `process.rs:65-75`）→ 4 次**。
- `process.rs:543-551` — `is_windows_pid_running(pid)` **每个 PID 一次** `tasklist`。
- `process.rs:554-570` — `wait_windows_pids_gone` 每 500ms 对**每个 PID** 再各起一次，直到超时。

**影响（含量化）**
Global 区域一次 `windows_workbuddy_process_rows_for` ≈ 4 × 141ms ≈ **564 ms**。等待进程退出时，若 3 个 PID 存活，每轮轮询 ≈ 423ms，轮询本身成了耗时主体。
缓解现状：`crates/buddy-switch-server/src/api.rs:28-46` 有 3 秒 `RUNNING_CACHE`，但**只覆盖服务端这一条路径**；Tauri 侧与 `wait_windows_pids_gone` 无缓存。

**修复方案（推荐两步走）**

*第一步（低风险，先做）*：把「按名过滤」改成「取一次全量 → 内存过滤」。
- 新增 `windows_tasklist_all_rows()`：`tasklist /FO CSV /NH` 一次拿全量。
- `windows_workbuddy_process_rows_for` 改为「全量 → `filter_windows_workbuddy_rows_for`」，从 4 次子进程降到 **1 次**（省 ~423ms）。
- `wait_windows_pids_gone` 改为「每轮取一次全量 → 用 `HashSet<u32>` 比对存活」，从 N 次/轮降到 **1 次/轮**。

*第二步（收益更大，单独一轮）*：换 `sysinfo` crate 或 Win32 `CreateToolhelp32Snapshot`，33ms 且无子进程，彻底消除黑窗风险。

**⚠️ 不破坏功能的关键约束**
1. `process.rs:549` 的 `None => true`（**查询失败视为进程仍在运行**）是保守设计，批量改造后**必须保留**同一语义 —— 否则查询失败会被误判为「进程已退出」，切换流程可能在客户端还活着时就动手。
2. 全量 `tasklist` 输出**多出「会话名 / 会话# / 内存使用」等列**，`parse_tasklist_csv`（`process.rs:360`）是按列索引解析的，必须核对列位一致性，别让内存列里的逗号把解析搞错。
3. `filter_windows_workbuddy_rows_for` 的精确映像名匹配语义不能变（现有护栏 `process.rs:1616-1627`）。

**建议护栏**
- 用真实全量 CSV 做夹具的新单测。
- **对照测试**：断言「全量 + 内存过滤」的结果与「逐个 `/FI` 查询」的结果**逐条相等**（这正是本项目的「阳性/阴性对照」纪律）。
- 断言 `None => true` 分支仍为 `true`。

---

### P0-2 首屏单 chunk 1.40 MiB，零路由懒加载

> **复核定性（详见复核报告）**：**确认为未做，非有意为之** —— `App.tsx` 中 lazy/Suspense/懒加载/chunk
> 相关字样**零匹配**；全仓性能相关提交只有 `e8c90a7 perf: Trae 分区切换不再整页重载`（做的是**数据层** SWR），
> 打包层从未涉及。
> **但影响需诚实下调**：桌面端从内嵌资源/本地磁盘加载、WebUI 走 `127.0.0.1`，**都不跨网络**
> ⇒ 1.40 MiB 的代价主要是**解析（数十 ms 量级）**，不是下载。
> recharts 在产物中的**确切占比未实测**（`node_modules/recharts` 5.1MB 不代表产物占比）。
> ⇒ **建议先量化再投入**：先拆一次 chunk 或跑可视化分析拿到 recharts / radix / 其余 的真实字节数，
> 再决定是否值得引入懒加载的复杂度（它牵动路由层 `Suspense` 与嵌入资源护栏，不是零成本）。

**证据（已核实）**
- `src/App.tsx:8-17` — 10 个页面全部静态 `import`。
- `dist/assets/index-CCFynCPV.js` 实测 1,464,712 字节；`vite.config.ts:24-54` 无 `build.rollupOptions.output.manualChunks`。
- recharts 被 `components/ui/chart.tsx:2`（`import * as RechartsPrimitive`）与 4 个统计页引入，全部进首屏。

**影响**：冷启动必须解析 1.4MB JS。打开「账号」页也会连带加载 4 个统计页的图表库 —— 用户从未访问的页面在付代价。

**修复方案**
1. 每个 `Route` 改 `const P = lazy(() => import("@/pages/P"))`，在 `<Routes>` 外层加**一个** `<Suspense fallback={...}>`。
2. `manualChunks` 把 `recharts` / `lucide-react` / `@radix-ui/*` 拆成独立 vendor chunk（recharts 尤其大）。
3. 顺带确认 `components/ui/chart.tsx` 是否真被使用，未用可删。

**⚠️ 不破坏功能的关键约束**
1. `App.tsx` 用 `BrowserRouter`，深链是常态 —— `<Suspense>` 必须包在**路由层**，不能只包单个页面，否则切路由会整页闪骨架。
2. **演示/截图模式已核实不受影响**：`src/lib/screenshot-demo.ts` 是**请求层 mock**，不依赖 DOM 同步挂载，懒加载后路由导航仍能拿到假数据。
3. `dist/` 与 `dist-demo/` 的 base 差异（`vite.config.ts:11-21`）不能碰；现有护栏 `api::tests::embedded_index_html_references_only_embedded_assets` 会校验 index.html 只引用已嵌入资源 —— **改了 chunk 划分后必须跑它**。
4. 改完前端**必须重编宿主**（`rust-embed` 编译期嵌入 `dist`），且重编前先停掉在跑的 serve，否则 `LNK1104`。

---

#### ✅ P0-2 已实施（2026-09-28，B2）—— 实测收益与三处与原方案的偏差

**实测收益**（`dist/` 产物，未 gzip）：

| 指标 | 改动前 | 改动后 |
|---|---:|---:|
| 首屏（`/` 路由）静态 JS 闭包 | **1,464,712 B** | **786,223 B** |
| 全部 JS 合计 | 1,464,712 B | 1,482,894 B（拆包开销 +18 kB，+1.2%） |
| 被推迟到其它路由 | 0 | **696,671 B** |
| 首屏变化 | — | **−678,489 B（−46.3%）** |

recharts 主 chunk `chart-*.js` = 385,793 B（gzip 107 kB），**只在统计页加载**。

**★ 三处与原方案不同（都是有意的）**

1. **`Suspense` 放在 `Layout` 的 `<Outlet />` 里，不是「`<Routes>` 外层」。**
   包住 `<Routes>` 会把**侧栏也一起换成 fallback** —— 每次切页侧栏闪一下，看起来像整页重载。
   放在 `<Outlet />` 里，外壳（侧栏 / 产品切换 / 状态圆点）保持不动，只有内容区显示骨架。
2. **没有引入 `manualChunks`。** 路由级 `lazy()` 之后，Rollup **自动**把 `chart` / `ComposedChart` /
   `Line` / 各页面拆成了独立 chunk（实测见上表）。再加 `manualChunks` 只会增加配置面，
   没有额外收益。原方案第 2 条**不必做**。
3. **默认路由 `/` 也懒加载。** 桌面端与 WebUI 都从内嵌资源 / `127.0.0.1` 取，
   多一次「入口 chunk → 页面 chunk」往返是毫秒级，换来「其余 9 个页面不进首屏」。

**★ 新增护栏（原方案漏掉的一条）**

`embedded_index_html_references_only_embedded_assets` **只校验 `index.html` 的根绝对引用**；
懒加载 chunk 全在入口 JS 的 `__vite__mapDeps` 清单里，`index.html` 里看不见
⇒ 那条护栏**对懒加载 chunk 零覆盖**。新增
`api::tests::embedded_entry_chunk_references_only_embedded_lazy_chunks`：
解析入口 chunk 的引号资源名并逐个断言在 embed 中，且**先断言集合非空**（不许空转）。
证伪：从 `dist/assets/` 移走 `chart-DDcOHMGW.js` ⇒ 立刻红，并逐字点名缺失文件。

**复现方式**

```bash
# 体积量化
npx vite build --config scripts/bundle-report.config.ts     # → dist-analyze/，量完删
# 浏览器验收（11 项断言：首屏不含统计页 chunk、切页后才加载、路由渲染、无白屏）
node scripts/preview-dist.mjs dist 4174 &                   # 需常驻后台
node ~/.workbuddy-ai/skills/webui-cdp-verify/scripts/cdp-drive.mjs \
     http://127.0.0.1:4174/ scripts/scenarios/b2-lazy-routes.scenario.mjs logs
```
验收场景已固化在 `scripts/scenarios/b2-lazy-routes.scenario.mjs`（**已跟踪**；截图落 `logs/`，该目录被 gitignore）。
脚本注释里记了两个真实踩坑：① 点侧栏后 `location.pathname` **同步**变、chunk **异步**拉，
断言必须等 chunk 到位，否则会看到「新增 chunk 落后一次导航」；② `chart-*.js` 由
**TokenStatsPage** 静态引入（不是积分页），首跑把它当积分页 chunk 断言 ⇒ **假失败，脚本错不是应用错**。


---

### P0-3 Trae 网关：每个请求重建整个账号池

**证据（已核实）**
- `crates/buddy-switch-gateway/src/trae/routes.rs:382-388` — `attempt_once` 每次选号前都 `pool.sync_for(variant)`；**换号重试会重复执行**。
- `trae/pool.rs:239-278` — `sync_for` 每次读 **3 份 JSON**（`load_remaining_for` / `load_cooldowns_for` / `account::entries_for_region`），并对**每个账号**执行 `device::derive(&uid)` + `rand_hex_salted(64, "mach", Some(&uid))`（`pool.rs:253-258`），重建整个 `Vec<TraePoolEntry>`。
- 注释 `routes.rs:385` 明确说明「每次选号前重新同步」是**刻意的正确性设计**（另一入口可能刚写冷却/刷新积分）。

**影响**：每个请求（含每次换号）3 次磁盘读 + N 次哈希派生 + 全量 Vec 分配。并发下成为吞吐瓶颈。

**修复方案 —— 不要简单加 TTL（会破坏上述正确性语义）**

正确做法是把 `sync_for` 的**三项成本拆开，各自用不影响语义的方式消除**：

1. **设备派生缓存（纯函数，最安全）**：`device::derive(uid)` 与 `rand_hex_salted(64, "mach", Some(&uid))` 都**只是 uid 的纯函数**（`pool.rs:237` 注释已确认「设备标识不需要分家」）。按 `uid` 缓存结果即可 —— 键空间 = 账号数，天然有界。这一项直接消掉 N 次哈希派生。
2. **文件读按 mtime 缓存**：3 份 JSON 改成「先 `metadata()` 比对 `(mtime, size)`，未变则复用上次解析结果」。**这保留了「另一个入口一写就立刻可见」的语义**（写必然改 mtime），同时消除重复 parse。这才是符合原设计的优化。
3. `apply_error` 写回后**主动标脏**，下次 `sync_for` 强制重读（写路径自己知道数据变了）。

**⚠️ 不破坏功能的关键约束**
1. 账号库按**区域**分家（`pool.rs:246-249`）—— 缓存键必须含 `variant.region()`，否则 CN/Global 串台（这是本项目踩过的坑）。
2. 缓存必须**可失效**，且能被测试重置（参照 `HomeOverrideGuard` 的做法）。
3. 必须保留 `sync_for` 的「以入参校正 `self.variant`」行为（`pool.rs:242`），否则 `apply_error` 会写错区域。

**建议护栏**
- 「同一 uid 两次 `device::derive` 结果相同」。
- 「改动 cooldown 文件后，下一次 `sync_for` 能看到新值」（防缓存过度）。
- 「CN / Global 两池互不污染」。

---

#### ❌ P0-3 已实测（2026-09-28，B3）—— **结论：不值得实施，建议关闭**

本节自己把「哈希派生 vs 文件 parse 谁是大头」列为**未实测**（见 §5 第 5 条），
并明确「未实测前不得当作『已确认』」。现已实测（隔离副本 + 独立 `CARGO_TARGET_DIR`，
临时基准 `cargo test --test tmp-bench-sync`，300 次取均值）：

| 账号数 | `sync_for` 单次（debug） | 单次（**release**） |
|---:|---:|---:|
| 1 | 152.3 µs | 108.5 µs |
| 5 | 233.1 µs | 133.9 µs |
| 20 | 534.3 µs | 199.1 µs |
| 50 | 1120.5 µs | **330.8 µs** |

拆解（release）：**固定成本 ≈ 112 µs**（3 份 JSON 的读盘 + 解析），**每账号 ≈ 4.4 µs**
（= `device::derive` 的 3 次 SHA-256 + `rand_hex_salted` 的 1 次 SHA-256 + 32 次 `format!` + 结构体分配）。

**为什么不做**：`sync_for` 走的是**转发一次 LLM 请求**的路径，那个请求本身是**秒级**的。
50 账号（远超真实场景）下 0.33 ms 的 CPU ≈ 该请求耗时的 **0.01%~0.03%**。
而按本节自己列的约束，加缓存要付出的是**正确性风险**：缓存键必须含 region（否则 CN/Global 串台）、
mtime 精度不足（须并入 size）、`apply_error` 写回后要主动标脏、还要能被测试重置。
**用「真实的失效 bug 风险」换「测不出来的 0.33 ms」，不划算。**

**这与 P0-2 的处理逻辑完全一致**（那条也是「先量化再投入」）—— 只是量化结果相反：
P0-2 量出 −46% 首屏，所以做；P0-3 量出 0.33 ms，所以不做。

**什么情况下应该改结论**：① 单账号池规模上到数百（此时线性项才开始显形）；
② `sync_for` 从「每请求一次」变成「每次选号/重试都调」且重试次数放大（`routes.rs:382-388`
在换号重试里会重复调，但重试次数由 `max_rotate` 限制，量级仍小）；
③ 出现真实 profile 证据表明该函数在火焰图里占比显著。

**保留的资产**：临时基准未进仓库（它依赖隔离副本的播种代码）。若日后要复查，
按「隔离副本」配方重建后用同样方式量即可 —— 配方已写进 `.workbuddy-ai/memory/2026-09-28.md`。


---

### P0-4 token 统计：每次全量重扫 + 全量 JSON 解析

> **复核定性（详见复核报告）**：**⚠️ 需先向作者确认，不得直接按缺陷处理。**
> `token_stats.rs:1048` 把「**实时聚合，不缓存、不落库**」写成了**设计属性**（断言式表述），
> 但**未给出理由**，且来自初始提交、无增量历史可追。
> - 若是**刻意**（大概率是为避免「落库聚合值与真实日志不一致」这类陈旧状态）⇒ **可加内存缓存、但不可落盘**，
>   且必须明确失效条件。
> - 若只是**描述现状** ⇒ 那就是未做的优化。
>
> **可以先做、且无争议的那一半**：`ide_workspace_meta` 被调在会话循环体内（`token_stats.rs:719`），
> 同一份 `index.json` 被读 N 次 —— 这是**纯冗余**，与「是否缓存聚合结果」无关，可独立先改。

**证据（已核实）**
- `crates/buddy-switch-core/src/modules/token_stats.rs:439-507` — `source()` 先 `files()` 递归 `read_dir` 收集全部 `.jsonl`（`:440-441`），再对**每个文件** `File::open` + `BufReader::lines()`，**逐行** `serde_json::from_str::<Value>(&line)`（`:468`）。
- `token_stats.rs:1048-1077` 注释自认 `region=all` 是**实时聚合、不缓存、不落库**。
- `token_stats.rs:612-642` / `:719` — `ide_workspace_meta()` 每次 `read_to_string` 整个 workspace `index.json` 再遍历找 id，而它被调在**每个会话 path 的循环体内** ⇒ 同 workspace 下 N 个会话把同一份索引**读解析 N 次**。

**影响**：`~/.workbuddy/projects` 等日志目录随使用无限增长，每次打开统计页都全量重扫重建整棵 `Value`；IDE 源部分叠加 O(会话 × 索引) 重复解析。

**修复方案**
1. **按 `(path, mtime, size)` 缓存单文件聚合结果**（`SourceCollector` 的 per-session 部分），缓存键含 region。
2. `ide_workspace_meta` 的索引解析**提到循环外**：按 workspace 目录建一次 `HashMap<conv_id, (title, model)>`。
3. 追加式 jsonl 可记录 offset 做**增量解析**（收益更大，但复杂度高，可作第二步）。
4. 用精简 `#[derive(Deserialize)]` 结构体替代 `Value`（仅取需要的字段）—— 收益明显，但**风险最高**（代码里用 `value.get("aiTitle")` 等动态取值 + `record_project` 可能读多种字段名），**必须先把被读字段全部枚举清楚**，建议单独一轮。

**⚠️ 不破坏功能的关键约束**
1. **等价性是唯一验收标准**：缓存前后输出必须**逐字节相同**。
2. **mtime 精度**：同秒内多次写入可能 mtime 不变 ⇒ 缓存键必须含 `size`，或提供手动失效入口。
3. 标题/摘要的读取顺序语义（`token_stats.rs:472-481`：`aiTitle` 优先于 `summary`，且**不受时间 cutoff 影响**）必须原样保留。
4. 缓存必须能被测试重置，且按 region 分家。

**建议护栏**：新增「同一份日志连续解析两次，输出 `assert_eq!`」+「缓存命中路径与冷路径输出相同」的等价性测试。

---

#### ✅ P0-4「无争议的那一半」已实施（2026-09-28，B4）

**改动**：`ide_workspace_meta`（每会话一次 `read_to_string` + `serde_json::from_str`）
→ 拆成 `workspace_index_path` + `workspace_meta_index` + `WorkspaceMetaCache`，
把 workspace `index.json` 的解析**提到会话循环外**、按路径缓存查表。

**语义等价性**（逐条对照，全部保留）：
- 索引缺失 / 读失败 / 坏 JSON / 无 `conversations` ⇒ 空表 ⇒ 回落 `(None, "未知模型")`（逐字相同）；
- 标题优先级 `name` → `title`；模型优先级 `selectedModelId` → `modelId` → `model` → `"未知模型"`；
- **同名 id 取首次出现**：改造前是「线性扫描返回第一个匹配」，故用
  `entry().or_insert()` 而非 `insert()`（后者会变成「最后一个赢」）；
- 无 `id` / `id` 为空串的条目**永远匹配不上**（调用方传的是会话**目录名**，
  空目录名先被 `.filter(|name| !name.is_empty())` 换成 `"未知会话"`）⇒ 直接跳过。

**★ 可观测缝**：`WorkspaceMetaCache` 带一个 `#[cfg(test)] parses: usize` 计数。
没有它，「只解析一次」就只能靠计时或读盘计数**间接推断**，两者都不可靠。
这是**直接**断言，且发布构建零成本。

**验证**：
- 新增 3 条用例 —— `workspace_meta_cache_parses_each_index_once`（3 会话 ⇒ `parses == 1`
  且各拿到自己的标题/模型）、`workspace_meta_cache_falls_back_exactly_like_the_linear_scan`
  （索引缺失 / 无该会话 / 坏 JSON 三种回落）、`workspace_meta_index_keeps_the_first_duplicate_id`。
- **证伪**：把 `lookup` 改回「每次重新解析」⇒ `workspace_meta_cache_parses_each_index_once`
  立刻红，报 **`left: 3` vs `right: 1`** —— 计数缝量的正是这件事，且坐实了改造前确实是 N 次。
- `token_stats::` 全套 **32 passed / 0 failed**，无编译警告。

**仍未做**：追加式 jsonl 的 offset 增量解析、以及用精简 `#[derive(Deserialize)]`
替代 `Value`（风险最高，需先枚举全部被读字段）。

---

#### ✅ P0-4 主体已实施（2026-10-10）——「不缓存」按现状处理，先加**内存**缓存

**前置结论：作者已确认。** 用户明确要求「token 查询太慢了，需要优化」⇒
「实时聚合、不缓存、不落库」按**描述现状**处理，不再是「待确认的设计属性」。
评审给的边界照办：**可加内存缓存、不可落盘**。
> ⚠️ 当天稍晚这条边界被**第二轮反转**为「可落盘」—— 见下面「落盘缓存」一节，
> 那里给出了反转的理由（内容指纹键让陈旧在结构上不可能）。

**★ 决定性发现（决定了整个方案）**：真实流量**只有 `cutoff = None` 一种**。
`TokenStatsPage` 传 `undefined`；`overview-metrics.ts` 传 `TREND_DAYS = 14`，
而 `get_statistics_for_filter` 只把 `7 | 30 | 90` 映射成窗口，**其余一律 `None`**。
⇒ 把 `cutoff` 放进缓存键做精确比对后，**命中即逐字节等价**，不存在「窗口随时间漂移
导致陈旧」的问题；`days` 恰好取 7/30/90 的调用（当前无调用方）必然未命中，行为与改造前一致。

**改动**（`crates/buddy-switch-core/src/modules/token_stats.rs`）
1. `source()` 的「全源共用一个累加器 + 逐文件循环」拆成
   [`parse_file`]（**每文件**一个 `SourceCollector`）+ [`SourceCollector::absorb`]（**按
   `paths.sort()` 顺序**合并）。
2. **并行**：`scan_paths()` 先查缓存，未命中的文件用 `std::thread::scope` +
   原子游标按 `available_parallelism()`（上限 32）分片解析，结果写回**按索引定位**的槽位
   ⇒ 合并顺序与串行逐字一致。
3. **进程内缓存**：`HashMap<PathBuf, (mtime_ms, size, cutoff) → Arc<SourceCollector>>`，
   按 `(mtime, size)` 做内容指纹。（当天稍晚追加了**落盘**层，见下一节。）

**★ 并行化最容易踩的坑（已加护栏）**：`push_session` 的 `key` 带**跨文件**去重后缀
（`项目 · 会话 · 2`），计数器属于**整个 source**。若各文件各自从 1 计数再直接搬运，
两个同名会话会**都拿到无后缀 key**。故 `absorb` 只取原始四元组、由 `self` 重新 `push_session`。

**实测（本机 32 逻辑核；`~/.workbuddy-ai/projects` = 12356 个 jsonl / 35GB）**

| 场景 | 改造前 | 改造后 | 倍数 |
| --- | --- | --- | --- |
| global 冷（首扫） | 226.4 s | **72.1 s** | 3.1× |
| global 热（二次查询） | 127.4 s | **0.50 s** | 255× |
| cn 冷 | 8.2 s | **1.99 s** | 4.1× |
| cn 热 | 2.1 s | **0.02 s** | 105× |

复现：`cargo run --release -p buddy-switch-core --example token_scan_bench -- <region>`
（每例自动跑冷/热两轮；`TOKEN_BENCH_DUMP=<目录>` 可把完整 JSON 落盘供 diff）。

**等价性验证（本项目纪律：等价性是唯一验收标准）**
1. 新增 `multi_file_aggregation_matches_single_collector_semantics`：2 文件 / 2 项目 /
   3 个整点 / 共享模型 / 只带标题的行，逐项断言 `summary`（含 `cacheRead`）、`models`、
   `projects`、`sessions`（含标题归属与排序）、`daily`、`dailyByModel`、`hours`、
   `coverageStartAt/EndAt`、`parseErrors`、`filesScanned`。
2. **★ 该断言在「改前」实现上同样通过**：用 `git show HEAD:<path>` 取出旧实现、
   注入同一条 `#[test]`（连同属性）后单跑 —— `1 passed`。⇒ 它编码的是**旧语义**，
   不是把新代码的输出抄成断言。
   ⚠️ 第一次注入时漏了 `#[test]` 属性 ⇒ 函数成死代码、**`0 passed` 却是绿的**；
   这正是「断言的敌人不是红、是永远绿」的现场。
3. **变异检验（证明护栏会红）**：
   - 缓存永不失效（忽略 mtime/size/cutoff）⇒ `scan_cache_is_equivalent_...` 红，
     报「改写后必须重新解析」；
   - 缓存永不命中 ⇒ 同一用例红，报「缓存命中不得再解析」；
   - 合并顺序倒置（`absorb` 反序）⇒ `parallel_scan_keeps_cross_file_session_key_dedup`
     红，报「按文件顺序拿到 1 / 2 后缀」。
4. `token_stats::` 全套 **36 passed / 0 failed**；`core / server / gateway` `--all-targets`
   零警告。

**★ 缓存失效在**活数据**上确实生效**：同一进程连跑两轮，第二轮 `records` 比第一轮多 1 ——
期间被 WorkBuddy 写过的那个文件 `(mtime, size)` 变了 ⇒ 未命中 ⇒ 重新解析。
（这也是为什么**不能**用「两次运行记录数相等」当等价性证据：日志目录是活的。）

**未做 / 已知边界**
- `ide_source()`（CodeBuddy IDE 的 `index.json`）仍是**串行 + 不缓存**：本机该目录
  `index.json` 数为 **0**，故未纳入本轮；有 IDE 历史的重度用户仍会慢，需单独一轮。
- 追加式 jsonl 的 **offset 增量解析**未做（首次扫描仍要读完 35GB）。
- 并发冷请求无 single-flight：两个请求同时冷启动会各扫一遍。

---

#### ✅ P0-4 第二轮：**落盘缓存**（2026-10-10）——冷启动 79s → 1.2s

**为什么还要做第二轮**：用户反馈「冷 72s 还是太久」。先量化（新增
`token_scan_bench read` 模式，只读盘不解析）：

| 量 | 结果 |
| --- | --- |
| 纯读盘 34.5GB / 12356 文件（32 线程） | **65.0 s（543 MB/s）** |
| 同上「热」轮（第二次） | **64.0 s** —— 与冷**一样** |
| 读 + 解析 + 聚合（全量） | 72.1 s |
| 单个 31MB 文件重复读（.NET 原生） | 1680 MB/s |

⇒ **解析只占 ~7s，90% 是 I/O；而且页缓存完全不帮忙**（35GB 塞不进 31.7GB 内存）。
磁盘本身不慢（单文件 1680MB/s、C: 余量 743GB），543MB/s 的聚合吞吐来自**每文件开销**
（12356 次 open + Defender 实时扫描；本机 Defender 实时防护为**开启**状态）。

**结论**：再优化 CPU 侧（预筛、`serde` 结构体、offset 增量）**收益上限只有 7s**，
不值得承担等价性风险。**唯一出路是不重读** ⇒ 把逐文件解析结果**落盘**。

**改动**（`crates/buddy-switch-core/src/modules/token_stats.rs`）
- 新增 `PersistedScanCache` / `PersistedFileEntry` / `PersistedSession`，
  写到 `~/.buddy-switch/token-scan-cache.json`（`store_dir()`，`atomic_write`）。
- `Totals` 用**定长数组** `[input, output, read, write, records]` 落盘：12356 个文件 ×
  每个几十个 `Totals` ⇒ 若带 `"input":` 之类键名，光键名就要几百 MB。
  **实测缓存文件 10.5MB**。
- `HashMap` 落盘前**按 key 排序**：不是为了正确性，是为了同一份数据每次产生相同字节
  （否则 `HashMap` 随机序让文件每次都变，无法 diff 且白写盘）。
- `ensure_cache_loaded()` 进程内只装一次；`read_persisted_cache(path)` 抽成**显式路径的纯函数**，
  让用例直接喂临时文件，不依赖进程级标记（并行用例会踩）。
- **只有真的解析过才写盘**（`CACHE_DIRTY`）；热路径一个字节都不写。
- 落盘前用 `path.exists()` 清掉已删除文件的条目（纯清理，不影响正确性）。
- **失败一律静默回落**：文件不存在 / 坏 JSON / 版本不符 ⇒ 当空缓存，重扫。
  `SCAN_CACHE_VERSION` 不匹配整体丢弃 —— 宁可重扫，不可读出错值。

**★ 关于「不落盘」这条边界的反转**：原注释把「实时聚合，不缓存、不落库」写成设计属性；
评审给的边界是「可加内存缓存、但不可落盘」，**理由是怕落库聚合值与真实日志不一致**。
该理由**对内容指纹键不成立**：条目只在 `(path, mtime, size, cutoff)` **逐字段相等**时
才被使用 ⇒ 文件一改必然未命中，**陈旧在结构上不可能**。故落盘安全。

**实测（两次独立进程 = 模拟重启）**

| 场景 | 耗时 |
| --- | --- |
| 第 1 次进程（真扫 35GB + 落盘） | **79.1 s** |
| 第 2 次进程（从盘装载 + 只重解析变化的文件） | **1.32 s** |
| 第 3 次进程 | **1.19 s** |

34.5GB 不可能 1.2s 读完（需 29GB/s）⇒ 缓存确实生效。三轮 `records` 分别为
2315587 / 2315592 / 2315593 —— 数据是活的，**只有变化的文件被重新解析**，
这同时反证了「缓存不会吞掉新写入」。

**验证**
- `persisted_entry_round_trips_every_field`：字段全非空的夹具（含标题行、坏行、
  `cacheRead`、coverage、多模型）经 JSON 往返后合并结果**逐字段相同**。
- `persisted_cache_file_round_trips_and_rejects_a_foreign_version`：写盘→读回等价；
  **异版本与坏 JSON 都必须整体拒绝**。
- **变异检验**：去掉版本检查 / 标题不落盘 / `parseErrors` 不落盘 ⇒ 两条用例**分别变红**。
- `token_stats::` 全套 **38 passed / 0 failed**。



---

### P0-5 静态资源无任何缓存头

**证据（已核实）**
- `crates/buddy-switch-server/src/api.rs:1855-1884` — `static_handler` 只设 `Content-Type`，**没有 `Cache-Control` / `ETag` / `Last-Modified`**。
- `api.rs:1877` — `Body::from(f.data.into_owned())` 每次把整个嵌入文件拷成新 `Vec`。

**影响**：浏览器每次刷新都**全量重传** 1.40MB JS + 86KB CSS。这是 webui 路径下最直接、最容易被用户感知的浪费。

**修复方案**
- 带内容哈希的 `assets/*` → `Cache-Control: public, max-age=31536000, immutable`。
- `index.html` → `Cache-Control: no-cache`（**必须**，否则更新后仍加载旧 chunk 引用，直接白屏）+ `ETag`。
- `into_owned()` 的拷贝收益有限（1.4MB memcpy ≈ 0.2ms 量级），**优先级低**，不必为它引入复杂度。

**⚠️ 不破坏功能的关键约束**
1. **`index.html` 绝不能长缓存** —— SPA 深链回退也走 `index.html`（`api.rs:1858-1871`），缓存它会同时破坏「更新生效」和「深链」两件事。
2. SPA 回退时 `Content-Type` 必须按**实际被服务的资源名**推导（`api.rs:1860-1865` 的注释说明了为什么），加缓存头时别顺手改成按请求路径推导。
3. 哈希资源与 `index.html` 要**分别判定**，别一刀切。

---

## 3. P1 — 值得做，中等改动

### P1-1 WorkBuddy 网关：每请求 5 次全局写锁

**证据（已核实）** `crates/buddy-switch-gateway/src/routes/relay.rs` — 单个请求内对 `state.pool.write().await` 加锁：`:84`（同步账号）、`:121`（acquire）、`:147`（release）、`:157`（note_success）、`:176`（apply_upstream_error）。`Pool` 位于 `Arc<RwLock<Pool>>`（`state.rs:248`），**全局写锁把所有请求的选号串行化**。

**修复**：① 账号同步从「每请求」改为**节流/事件驱动**（间隔或按账号库 mtime）；② `acquire`/`release` 这类小状态更新改为原子量或分片锁；③ `select_account` 走读锁。**约束**：`sync_accounts` 的「增量 upsert、不删除既有账号以保留治理历史」语义（`relay.rs:81`）必须保留。

---

#### B5 实施结果（2026-09-28）—— 实测后**改写**了本条的优先级

复核时逐行读了 `relay.rs:82-190` 的 5 处 `pool.write()`，并**实测了各临界区的性质**：

| 锁点 | 临界区内做什么 | 是否含 IO |
|---|---|---|
| `relay.rs:83-86` | `load_accounts_for` 在**锁外**先读盘，锁内只有 `sync_accounts`（内存合并） | ❌ |
| `relay.rs:121` `acquire` | 内存置在途标记 | ❌ |
| `relay.rs:147` `release` | 内存清标记 | ❌ |
| `relay.rs:157` `note_success` | 内存计数 / EMA（只置 `self.dirty`） | ❌ |
| `relay.rs:176` `apply_upstream_error` | 内存冷却 / 熔断（只置 `self.dirty`） | ❌ |

⇒ 这些临界区都是**几百纳秒级的内存操作**，「每请求 5 次写锁」本身**不构成瓶颈**；
本条原判的严重度**下调**。真正严重的是另外两处（下）。

##### ★★ 发现一（**正确性缺陷**，非性能）：池治理状态**从不落盘**

`Pool::flush_if_dirty` 存在、类型全对、单测全绿（`persistence_round_trip_*` 等），
但**生产上没有任何调用点** —— 唯一的包装 `GatewayState::persist_pool` 自身**零调用者**。
而 `GatewayState::load` 在启动时调 `pool.load(&state_file, …)` 读的正是它写的那个文件
⇒ **该文件永远是旧的**。

后果不是崩溃而是**静默遗忘**：冷却 / 熔断 / 成功率 EMA / 余额读数 / `credits_refreshed_ms`
全部只在内存里，**每次重启从零开始**（例：刚判定 `SessionDead` 的账号，重启后立刻又被选中、再撞一次同样的墙）。

★ 这与 `BackgroundTask` 注册表文档里记的 `set_credits` 事故是**同一类**
（「函数存在、类型全对、构建全绿、测试全绿，但生产上没人调用」）——
**同一个坑在本仓库第二次出现**，只是换了个函数。

**已修**：新增 `BackgroundTask::PoolPersist` 登记进注册表（周期 30 s，`flush_if_dirty` 只在有改动时真写），
并补两条护栏：`background_task_registry_includes_pool_persist`（注册表守卫）+
`persist_pool_writes_governance_state_to_disk`（端到端：改池 → `persist_pool` → 文件里真出现该账号）。
两条都做了变异验证（移除注册项 / 让 `persist_pool` 直接 return ⇒ 均精确变红）。

##### ★★ 发现二（**真·阻塞**）：`refresh_once` 持**全局写锁跨网络取数**

`credits_refresh.rs` 改造前：

```ignore
let mut pool = state.pool.write().await;
refresh_with(&mut pool, &fetcher, now_ms, interval_ms).await   // 内部逐账号 await 网络
```

写锁**跨整个取数过程**持有 ⇒ 后果不是死锁而是**排队**：启动时所有账号都「从未取过余额」，
第一批 relay 请求必须等完整一轮取数（账号数 × 单次上游耗时）才能选到号；之后每 30 分钟再来一次。
这与本模块开头那条硬约束（「绝不在请求路径上同步拉余额」）是**同一件事的另一面**：
取数虽不在请求路径上，但它**把请求路径堵住了**。

**已修**：拆成三段 —— 读锁算目标 → **无锁**取数 → 短暂写锁写回。
`refresh_with`（单测注入点）与新的 `refresh_pool`（生产）**共用**
`refresh_targets` / `apply_refresh` / `parse_credits`，**只差锁的持有时机**，
因此不存在「测的路径不是跑的路径」；另有 `refresh_pool_and_refresh_with_agree` 钉住两者产出相同。

护栏 `refresh_pool_does_not_hold_the_lock_across_fetches`：注入的 fetcher 在 `fetch` 时
对同一把锁 `try_read()`，拿不到即说明锁被占着 —— **直接观测，不靠计时**。
变异（改回「先取写锁再取数」）⇒ 红且报 `left: 1` vs `right: 0`。

##### 仍未做（**已评估，建议不做**）

- **账号同步节流**（原方案 ①）：`sync_accounts` 在锁内是**内存合并**，且账号数是几十量级；
  加节流会引入「另一入口刚改账号库、这边看不到」的陈旧窗口 —— 与 P0-3 同款取舍，**收益测不出来、风险真实**。
- **`acquire`/`release` 改原子量 / 分片锁**（原方案 ②）：临界区已是纳秒级，改造成本与出错面远大于收益。
- **`select_account` 走读锁**（原方案 ③）：它确实只读，但每次选号都紧跟一次写（`acquire`），
  读写分开只是把一次锁变成两次，无净收益。


### P1-2 core / gateway 在 async 里直接做阻塞 IO

**证据（已核实）** `spawn_blocking` 在 `buddy-switch-core` 与 `buddy-switch-gateway` 中**各 0 处**，而 `buddy-switch-server/api.rs` 有 15 处。已定位的阻塞点：`trae/handlers.rs:503-513`（async 中调 `list_account_views_for`）、`trae/checkin.rs:360-374`（async 中调 `entries_for` / `list_account_views_for` / `load_cooldowns_for` / `load_remaining_for`，循环内还有 `device::ensure_for_variant` 阻塞 fs）。

**影响**：阻塞 tokio worker 线程；多账号并发签到/刷新时线程池被占满，整体吞吐下降。
**修复**：把同步 IO 包进 `tokio::task::spawn_blocking`。**约束**：`spawn_blocking` 要求 `'static`，需把数据 `Arc` 化；注意别把「持锁跨 await」引入进来。

#### B5 对 P1-2 的评估（2026-09-28）—— **建议不做，理由如下**

本条定位的两处（`trae/handlers.rs:503-513`、`trae/checkin.rs:360-374`）都在 **Trae 侧的后台/交互路径**
（签到、余额刷新、账号视图列表），**不在 relay 热路径上**。`spawn_blocking` 的收益是
「不让同步 IO 占住 tokio worker 线程」，而它的代价是真实的：
`'static` 约束会迫使 `Arc` 化一批数据、并新增「跨 `spawn_blocking` 边界」的错误处理与
`await` 点 —— 而本仓库刚刚（见 P1-1 发现二）踩过「持锁跨 await」这个坑。

判据：**收益要有证据**。目前没有任何 profile 证据表明这些路径造成过线程池饥饿
（本应用是**个人本地网关**，并发度是「几个请求」而不是「几百并发」）。
⇒ 与 P0-3 同款结论：**在有 profile 证据之前不做**。若日后真要做，先补一个
「并发 N 个签到请求 + 观测 tokio worker 是否被占满」的度量，而不是直接改。


### P1-3 Tauri：55 / 81 个命令是同步的

> **复核定性（详见复核报告）**：**确认为遗漏，且判据是明文约定** —— `commands.rs:90-92` 与 `:150-151`
> 自己写着判据：「**会起子进程**」或「**慢 IO**」必须 async + spawn_blocking，理由是「避免阻塞主线程造成页面卡顿」。
> **⇒ 55 个不能一刀切，必须分级**：
> - **该改**（慢 IO）：`get_trae_logs`(`:1306+` 全量读日志)、`list_sessions`(`:483`)、`import_local`(`:262`)、
>   `get_trae_accounts`(`:1275`)、`get_trae_variants`(`:1260` 含进程探测)、`check_auth_permission`(`:338` 含写盘)。
> - **不必改**（微秒级配置读取，改 async 反而多一次线程调度）：`get_switch_config`、`get_schedule_config`、
>   `get_auto_checkin_config` 等。
>
> 原报告「55 个同步」的表述易被读成「55 个都该改」，此处更正。

**证据（已核实）** 81 个命令中 26 个 async、**55 个同步**。同文件 `:88-94`（`get_status`）、`:152-157`、`:171-175`、`:633`、`:1333` 已按上述约定走 `spawn_blocking` —— 说明做慢 IO 却仍同步的那些是**遗漏而非设计**。

**影响**：文件 IO 跑在 Tauri 主线程，账号页频繁刷新时可能卡 UI。
**修复**：按 `:633` 的既有模式改 `async` + `spawn_blocking`，**按上表分级**。**约束**：**命令名与参数名不能变**（`scripts/check-api-contract.cjs` 校验「`api.ts` ROUTES ←→ Tauri invoke ←→ server 路由」三方一致 + 「单 `Value` 参数命令必须恰好传该键」）—— 改完**必须跑门禁**，且注意门禁的「（提示）」行。

### P1-4 粘性会话表：★ **本条原判有误，已撤回**（详见复核报告）

**原判**：「`gc` 从不被调度 ⇒ `bindings` 无界增长 ⇒ 长跑内存泄漏」。

**复核事实**：`crates/buddy-switch-gateway/src/sticky.rs` 的公开方法 `get` / `bind` / `unbind` / `gc`，
在全仓（排除自身）**一个都没被调用**；唯一外部引用是 `routes/status.rs:29` 的 `state.sticky.read().await.len()`
⇒ 该表**恒为空**，`sticky_sessions` 永远是 `0`。

**更正后的结论**：**不存在内存泄漏**（无增长）。真实情况是**整套粘性会话机制未接线** —— 属未完成/预留功能。
按本项目「不保留死代码」纪律，应明确去留：**接上写路径**，或**连 `sticky_ttl_ms` 配置项与 status 字段一并移除**。

**若决定接上**：`server/main.rs:33-57` 的「后台任务注册表」纪律要求 —— 任何后台循环**必须登记进
`BackgroundTask` 枚举**（注释原文：「登记即执行——不要在本表之外直接 `tokio::spawn` 后台循环」），
且已有测试守住该表。**约束**：GC 不得驱逐未过期绑定（单测 `gc_drops_only_expired_entries` 已锁定）。

#### ✅ B8 已接线（2026-09-28）—— 复核先发现**接线的键是错的**，修好后接上

准备接线时逐行读了键的派生，发现一个**会让「接上」变成假动作**的问题：

`sticky.rs` 的键是 `sticky_key(region, model, conversation_request_id)`，
而 `conversation_request_id` 由 `relay::prepare_body` 这样解析：

```rust
session_headers::resolve_conversation_request_id(inbound_request_id, None, turn_key)
//                                                               ^^^^ session_key 传的是 None
```

`resolve_conversation_request_id` 的优先级是
① 入站头 → ② **会话键**派生（「同一会话跨轮稳定」）→ ③ 轮键派生 → ④ 随机。
因为 **② 的 `session_key` 恒为 `None`**，实际只会在 ③（按「最后一条 user 文本」派生）与 ④ 之间落。

⇒ **`conversation_request_id` 实际是「轮」级标识，每轮都换。**
用它当粘性键 ⇒ 每轮都是新键 ⇒ 绑定永远命中不了、`sticky_sessions` 恒为 0。
**「把 `bind` 接进 relay」这个动作本身不会产生任何效果。**

而模块开头写明的用途是**跨轮**的：「同一会话的上下文会落到不同账号上，上游侧表现为
『对话突然失忆』；且跨账号的缓存前缀命中率归零」。

**⇒ 正确的键是 `RelayRequest::conversation_id`（客户端会话 id）**，而不是轮主键。

##### 改用它牵出一个**真实的取舍**（已确认按方案 1 实施）

`conversation_id` 是 `Option<String>`（来自请求体 `metadata.conversation_id`），**可能缺失**；
而轮主键是 `String`（必有）。这大概就是原作者选轮主键的原因 —— 但那让粘性**永不命中**。

更关键的是：粘性**会绕过 `pick` 的四因子加权**（积分比例 / 快过期占比 / 闲置补偿 / 成功率）
去钉住一个账号。也就是说：

| | 粘性开（现已采用） | 粘性关 |
|---|---|---|
| 同会话上下文 | 固定在同一账号（上游不「失忆」、缓存前缀命中） | 可能落到不同账号 |
| 积分利用 | 绑定的账号可能积分少却一直被用（直到不可用才解绑） | 按四因子选，额度利用更优 |

这是**产品取舍**（拿「额度利用」换「会话一致性与缓存命中」），不是纯缺陷修复。
`sticky_ttl_ms` 与 status 的 `sticky_sessions` 是已对外暴露的配置/契约。

**缓解**：取舍的代价被两条设计压到最小 ——
① 只对**带了会话 id** 的请求生效（没带的一律走原逻辑，行为不变）；
② 绑定的账号一旦不可用（冷却 / 在途占满 / 模型级限流 / 账号已删）**立刻放弃**并落回四因子选号。

##### ✅ 已接线（2026-09-28，按「键改用 `conversation_id`」方案）

**做了什么**

1. **键改用会话级标识**：`sticky_key(region, model, session_key)` 的第三参从
   `conversation_request_id` 改为**客户端会话 id**；参数名与文档一并改，
   把「为什么不能用轮主键」写进函数文档（避免后人又改回去）。
2. **`conversation_id` 缺失 ⇒ 整段跳过粘性**（`sticky_key_of` 返回 `None`）：
   **不退化成轮主键** —— 那只会造一个永不命中的键，让 `sticky_sessions` 看起来非 0 却毫无作用。
   绝大多数既有调用点不带 `metadata.conversation_id`，它们的行为与接线前**逐字相同**。
3. **可用性判据同源**：新增 `Pool::is_usable_for` ⇒ `pick::is_usable` ⇒ `pick::entry_usable`，
   与 `pick_account` 的候选过滤**共用同一个函数**（`entry_usable`）。
   若另写一套且更宽松，粘性会把请求钉到 `pick` 认为不可用的账号上 —— 且**不报错**。
4. **绑定 / 解绑时机**：只在**成功之后** `bind`；失败（进 `tried`）时立刻 `unbind`。
   （即便漏了解绑也不会失败 —— 冷却中的账号会被 `is_usable_for` 挡掉 —— 解绑只是省一次无用尝试。）
5. **GC 走写入路径摊销**，**不新增后台循环**（因此不触碰 `BackgroundTask` 注册表纪律）：
   `bind` 里 `len >= gc_at` 才清扫，清扫后按当前规模**翻倍上抬**阈值。
   ★ 关键分支：**清扫释放 0 条时阈值必须上抬** —— 否则「条目大多未过期」的正常长跑场景下
   每次 `bind` 都会全表扫描，那才是真的性能问题。反向（清扫清空整表）则回落下限。

**★ 最重要的一条性质：粘性绝不导致失败**

`sticky_choice` 被抽成**纯函数**，`Some(uid)` = 用它、`None` = 放弃并落回原逻辑。
**它无法表达「失败」**：uid 为空 / 本请求已试过 / 此刻不可用，任一条不满足即放弃。
抽成纯函数就是为了让这条性质能被**直接**测到。

**护栏与证伪**（gateway lib **330 → 339**，全绿）

| 护栏 | 守住什么 |
|---|---|
| `sticky_key_is_none_without_a_conversation_id` | **不破坏既有使用**：无会话 id（含空串/纯空白）⇒ 粘性整段跳过 |
| `sticky_key_carries_region_and_model` | 区域与模型必须进键（跨域不串、切模型不粘） |
| `sticky_choice_falls_back_instead_of_pinning_an_unusable_account` | ★★ 不可用/已试过/空 uid ⇒ 一律放弃 |
| `sticky_choice_probes_exactly_the_preferred_uid` | 只问被选中的那个 uid |
| `is_usable_agrees_with_pick_on_each_state` | ★ 判据与 `pick` 同源（行为级印证） |
| `is_usable_honours_model_scoped_cooldown` | 模型级冷却也要被看到（粘性传的是模型名） |
| `is_usable_treats_expired_model_cooldown_as_available` | ★ 已过期的模型冷却不得判为不可用 —— `pick` 会先 prune 而 `is_usable` 不 prune，判据必须自带过期判断 |
| `bind_amortizes_gc_so_long_runs_stay_bounded` | 接上写路径后表**必须有界** |
| `gc_threshold_is_raised_only_when_a_sweep_frees_nothing` | ★ 摊销的关键分支（见上） |
| `default_falls_back_to_thirty_minutes` | `new(0)` / `Default` 都落到 30 分钟 |

**证伪**：把 `sticky_choice` 改成忽略 `usable`（= 会钉死不可用账号）⇒ 两条用例红，
报「账号不可用时必须放弃粘性，绝不能钉死」。还原后按内容校验（变异标记 0 + 关键调用存在）。

**未覆盖（如实说明）**：`relay()` 的**端到端**粘性行为（「绑定后下一轮真的选到同一账号」）
没有自动化用例 —— 它需要构造 `GatewayState` + mock 上游，本 crate 没有这层脚手架
（现有 e2e 是 **Trae 侧**的 `trae_gateway_e2e.rs`，不覆盖 WorkBuddy relay）。
粘性的**决策逻辑**已被纯函数用例覆盖，**接线本身**靠编译器 + 全量回归保证。


### P1-5 official_usage 缓存命中时深拷贝整个大 payload

**证据（已核实）** `crates/buddy-switch-core/src/modules/official_usage.rs:237-249` 命中缓存即 `cached.clone()`；`:251-256` 写入再 clone 一次。注释 `:63-64` 述 payload 含全账号 `requests[]` 明细，约 600KB/账号。

**修复**：缓存改 `Arc<Value>`，命中时只克隆 `Arc`（引用计数 +1），消除每次统计请求的整块深拷贝。**约束**：调用方若需要 `&mut`，改在拿到 `Arc` 后再 `Arc::try_unwrap` 或按需 clone 一次。

### P1-6 前端：整 store 订阅 + 无门控的 2 秒轮询

> **复核定性（详见复核报告）—— 本条必须拆成两条，结论相反：**
>
> **(a) 2 秒轮询本身 = 有意为之，不得擅动。** 它是**用户可开关的功能**：
> `TraeSettingsPage.tsx:133` `const [autoRefresh, setAutoRefresh] = useState(true)`，界面上有开关；
> 由 `e8c90a7 perf: Trae 分区切换不再整页重载` 引入，且带说明意图的注释
> （「自动刷新只在『没有未提交的输入』时跑：否则用户正在输入关键字…看起来像在抖」）。
> ⇒ 这是有意的「日志实时跟随」，**2s 是产品选择，擅自拉长间隔会改变用户可见行为**。
>
> **(b) 缺 `visibilitychange` 门控 = 真遗漏。** 仓库里**已有两套正确实现可对照**：
> `use-workbuddy-status-refresh.ts:79-82`（visibilitychange + focus + blur）、
> `use-credit-auto-refresh.ts:69-77`（visibilitychange + Tauri `main-window-visible` 事件）；
> 而 `TraeSettingsPage.tsx:163-167` 与 `AccountsPage.tsx:427-439` **都没有**。
> ⇒ 「窗口不可见时暂停轮询」**不改变任何用户可见行为**（用户看不到时本就无需刷新），
> 且现成模式可直接复用（连 Tauri 的窗口可见事件通道都备好了）⇒ **安全的遗漏修复**。
> **但间隔数值不要动。**

**证据（已核实）**
- `src/pages/CreditStatsPage.tsx:1475` — `useAccountsStore()` **未传 selector**，订阅整个 store。而该 store 每 30 分钟刷新积分、每 60 秒刷新状态，且 `loadCredits` 按账号逐个 `setState` ⇒ 每次都会让这个 1900+ 行、含多个 recharts 图表的页面**整页重渲染**。
- `src/pages/TraeSettingsPage.tsx:165` — `setInterval(() => void refresh(), 2000)`，`refresh()` 走 `force: true`（`use-cached-resource.ts:62-65`）⇒ 每 2 秒强制发一次 `get_trae_logs`，且**没有 `visibilitychange` 门控**（窗口最小化/切后台仍持续请求）。
- `src/pages/AccountsPage.tsx:427-439` — 60 秒旅行状态轮询同样**无门控**。

**修复**：① `CreditStatsPage` 拆成多条带 selector 的订阅（或 `useShallow` 合并为稳定对象）；② 给 `TraeSettingsPage` 与 `AccountsPage` 的轮询**加可见性门控**（照抄上面两套现成实现），**间隔保持原值不变**。

### P1-7 出站请求体多轮 parse / serialize

**证据（已核实）** `crates/buddy-switch-gateway/src/outbound/mod.rs:75` `serde_json::from_str::<Value>` → 改写 → `:98` `.to_string()`；上游路径 `core/modules/upstream.rs` 另有一轮。大 prompt（工具定义多）时 CPU 显著。
**修复**：合并为**单次解析 → 原地改写 → 单次序列化**；无改写需求时**直接透传原始字节**。
**约束**：`outbound/mod.rs:351+` 有一批断言产出 JSON 结构的测试（含 `prompt_cache_key`、effort 调整、tool_call 序列），**必须全绿**。

### P1-8 `dir_stats` 每次概览递归整个快照树

**证据（已核实）** `crates/buddy-switch-core/src/modules/trae/profile.rs:1000-1010` 递归 `dir_stats`；`:1035` 对每个槽位调用；`overview_for`（`:1847` 附近）每次调用。
**影响**：快照含 `User/globalStorage` 等大树，每次概览遍历全部账号快照的完整文件树。
**修复**：按槽位 mtime 缓存 size/count，或列表视图只读顶层 mtime。

---

## 4. P2 — 低风险顺手清理

> **⚠️ 复核提醒（详见复核报告）**：下表中**两条已改判**——
> - 「缓存默认每次挂载重校验（`freshMs` 默认 0）」：**有意为之，不是缺陷**。`use-cached-resource.ts:9-18`
>   的文档表把「已有快照 → 立刻渲染真数据 + 后台重校验」写成核心语义，即 **stale-while-revalidate**，
>   是「切分区不闪骨架」的实现手段。设 `freshMs` 属**调优取舍**（拿「少一次请求」换「最长 30s 陈旧」），
>   **应由产品决定，不该由性能审计单方面改**。
> - 「账号列表排序在 render 体内 / 零 `React.memo`」：**证据不足，降级为待实测**。全仓 `React.memo` 0 处
>   说明**不是刻意规避**（无相关约定/注释），但也**没有实测证据**表明重渲染构成瓶颈
>   （账号数通常为个位数到几十）。**需先用 React Profiler 确认，再动。**

| 项 | 位置（已核实） | 修复 |
|---|---|---|
| 账号列表排序在 render 体内 ⚠️待实测 | `src/pages/AccountsPage.tsx:714-739`（IIFE，无 `useMemo`） | 先用 Profiler 确认，再包 `useMemo` |
| 全仓零 `React.memo` ⚠️待实测 | `src/` 计 0 处；`components/account-card.tsx:246` 为普通组件 | 同上，先量化再定 |
| 缓存 `freshMs` 默认 0 ⚠️**有意为之** | `src/lib/use-cached-resource.ts:55` | **不是缺陷**，仅产品层面的调优取舍 |
| 账号页 N+1 请求 | `AccountsPage.tsx:94-115`（每账号一次 `getCheckinStatus`）、`:118-139`（每账号一次 `getTravelStatus`）、`:431` 每 60s 全量重放 | 后端加批量接口；短期先拉长间隔 + 加可见性门控 |
| 缓存默认每次挂载重校验 ⚠️**已并入上表，此行为原判，已撤回** | — | 见上表与复核报告 |
| `existing_windows_drives` 26 次 `Path::exists` | `process.rs:573-577` | 改 Win32 `GetLogicalDrives` 位图或加缓存 |
| Vec 去重 O(n²) | `process.rs:434-437`、`:462-464`、`credit_usage.rs:266-267`、`:305-306` | 改 `HashSet` |
| `p95_latency` 每次 clone + sort | `trae/token_stats.rs:72-82` | 排序一次复用，或近似分位 |
| 大图直接进 bundle | `src/assets/donate-wechat.png` 136KB、`donate-alipay.jpg` 129KB、`workbuddy-official-icon.png` 136KB、`codebuddy-cn-ide-icon.png` 108KB | 转 webp；`donate-*` 改 `import()` 懒加载 |
| 全量字体包 | `src/main.tsx:3` 引入整个 `@fontsource-variable/bricolage-grotesque`（仅侧栏标题一处用，`App.tsx:437`） | 改 `.../latin.css` 或按 weight 子集引入 |
| SSE 解析器二次复杂度 | `trae/sse.rs:97-99`：`buffer.iter().position()` 每次从 0 扫 + 命中后 `drain(..=index).collect()` 每行一次 memmove | 用游标偏移 / `VecDeque` 代替反复 `drain`，保留尾部残行 |
| 出站体每次 attempt 复制 | `trae/routes.rs:485` `.body(body.to_vec())` | 用 `Bytes` 共享而非 `to_vec` |
| 托盘菜单全量重建 + 读盘 | `src-tauri/src/tray.rs:393-400`（每次重建整个菜单）、`:405` 调 `checkin::all_accounts_checked_in_today()` | 复用 `Menu`，只 `set_text`/`set_checked` 可变项；`checked_in` 结果加短缓存 |
| 命令式驱动日志全量读 | `trae/logs.rs:104-113`、`:139-160` | 按 mtime 缓存解析结果 |
| 配置路径每次读 env | `config.rs:74-79`、`:127-146`、`:217-219` | `OnceLock` 缓存（测试可重置） |

**已核对无问题、请勿「优化」**：`App.tsx:185`（60s 侧栏圆点）与 `App.tsx:311`（30 分钟检查更新）均**已有 `active` 门控**，属合理设计；`switch-account-dialog.tsx:105` 的 600ms 轮询仅在 webui 分支且生命周期极短，收益有限。

---

#### B7 实施结果（2026-09-28）—— 一条真问题 + 六条**实测后否决**

##### ✅ 真问题（**比审计描述的更严重**）：webui 下账号页是 **N×N** 次上游查询

审计写的是「N+1 请求」。实测逐行读代码后发现是 **N×N**：

- **webui** 的 `get_checkin_status` 是**整端点** —— 它把**全部账号**各查一次上游后一起返回；
- **桌面端**的同名命令是**单账号**的。
- 而账号页原本对每个账号调一次 `api.getCheckinStatus` ⇒ webui 下**每次调用都拉全量**
  ⇒ **N 个账号 = N × N 次上游签到查询**（20 个账号 = **400 次**）。旅行状态同款。

**已修**：在 `api.ts` 新增 `getCheckinStatusMap` / `getTravelStatusMap` 两个**批量入口**，
把「webui 一次、桌面端逐个」这条**通道差异收口在 api 层**（调用方不必也无法自己判断）。
账号页改用它们。

**验证（请求计数，不是读代码）**：新增 `scripts/mock-webui-backend.py`（会统计每端点调用次数）
+ `scripts/scenarios/b7-accounts-nplus1.scenario.mjs`：

| 断言 | 修复后 | 旧实现 |
|---|---|---|
| `GET /api/checkin/status` | **1** | 3（= 账号数） |
| `GET /api/travel/status` | **1** | 3 |
| `GET /api/accounts` | 2（CN + Global 各一次，**不随账号数增长**） | 2 |

**证伪**：把 `fetchTodayCheckinMap` 改回逐账号调用并重建 ⇒ 计数立刻变 **3**，场景变红。
截图人工核对：3 张卡片各自显示正确的「未签到 · 未旅行」⇒ 批量结果**落对了卡片**，不只是请求变少。

★ 两个**验收工具自身的坑**（都真实踩到，已写进脚本注释）：
1. **mock 不读请求体 ⇒ HTTP/1.1 keep-alive 协议失步**：未读的 body 被当成下一个请求行，
   浏览器把后续请求报成「缺少 `Access-Control-Allow-Origin`」的 CORS 失败。
   症状是**同样的端点时通时不通**、计数不可信。修法是处理前先把 body 读干净。
2. **不要用 `location.reload()` 重置计数**：它会打断 CDP 的页面上下文，后续 `evaluate`
   落在旧 context 上（实测：reload 后卡片永远等不到）。正确做法是**每次跑之前重启 mock**。

##### ❌ 实测后**否决**的六条（都写了理由，避免后人重复分析）

| 项 | 否决理由 |
|---|---|
| 全量字体包 | **审计的前提不准**：字体用在 **7 处数字展示**（不是「一处」）。且 `unicode-range` 让浏览器**只下载 latin 子集（41 kB）**，另两个子集只嵌不传（68 kB 里 27 kB 永不传输）。要省得手写 `@font-face` —— 维护成本换 0 用户可见收益。 |
| `donate-*` 改 `import()` 懒加载 | **已经是按需的**：URL 在模块作用域不影响传输时机，`<img>` 只在弹窗打开时才渲染 ⇒ 只在那时下载。改 `import()` 只挪一个字符串，零收益。 |
| 4 处 Vec 去重 O(n²) | n = 账号数 / 进程数（几十量级），O(n²) 是**微秒级**。改 `HashSet` 要引入第二个结构并保持同步，可读性反而下降。 |
| SSE 解析器「二次复杂度」 | `drain(..=index)` 的 memmove 确实与剩余长度相关，但**buffer 每次 `feed` 后只剩残行**（有界于 chunk 大小，非流总长）⇒ 单次 `feed` 内 L 只有几条，非真二次。 |
| `config.rs` 路径函数加 `OnceLock` | **会破坏测试隔离机制**：那些无参路径函数**每次重读进程级 `BUDDY_SWITCH_HOME`** 正是 `HomeOverrideGuard` 能生效的前提（本项目已为此踩过四次假失败）。审计不知这层依赖。 |
| 图标已修（见下） | — |

##### ✅ 顺手修掉的真浪费：图标按**实际渲染尺寸**缩图

`workbuddy-official-icon.png` 与 `codebuddy-cn-ide-icon.png` 都是 **512×512**，而它们最大只渲染到
**56 px** / **22 px**（`WorkBuddyMark` / `CodeBuddyCnIdeMark`），且**始终可见**（侧栏 + 账号卡）⇒ 首屏下载。

| 文件 | 前 | 后 | 省 |
|---|---:|---:|---:|
| `workbuddy-official-icon.png` | 135.8 kB (512²) | **22.2 kB (192²)** | −84% |
| `codebuddy-cn-ide-icon.png` | 107.8 kB (512²) | **10.1 kB (96²)** | −91% |
| 合计 | **243.6 kB** | **32.3 kB** | **−211 kB** |

尺寸取值：最大渲染 56 px × 3 DPR = 168 px ⇒ 192² 留足余量；22 px × 3 = 66 ⇒ 96²。
**视觉验收**：`scripts/scenarios/b7-icon-quality.scenario.mjs` 把 `<img>` 临时放大到 200 px 后截图
（15 px 的原尺寸看不出质量），并断言 `naturalWidth` 为 192/96（证明新资源生效、旧 512 已消失）。
截图人工核对：边缘干净、透明通道完好。


---

## 5. 待实测确认 → **B0 已执行（2026-09-28），结论如下**

> **B0 已完成**。量化脚本已固化为 `scripts/bundle-report.config.ts`
> （`npx vite build --config scripts/bundle-report.config.ts`，产物落 `dist-analyze/`，量完即删）。

### 5.1 P0-2 recharts 占比：**已量出，结论是「值得做」**

按包拆 chunk 后（minified / gzip）：

| chunk | 体积 | gzip | 占比 | 说明 |
|---|---:|---:|---:|---|
| `index`（应用代码） | 597.87 kB | 146.78 kB | 40.9% | 全部页面 + 组件 |
| **`pkg-recharts`** | **407.73 kB** | **109.16 kB** | **27.9%** | recharts **及其传递依赖**（`d3-*` / `victory-vendor` / `lodash` / `react-smooth` / `react-is` …） |
| `pkg-react` | 193.07 kB | 60.58 kB | 13.2% | react / react-dom / scheduler |
| `pkg-other` | 124.87 kB | 38.79 kB | 8.5% | 其余零散依赖 |
| `pkg-radix` | 86.69 kB | 26.77 kB | 5.9% | `@radix-ui/*` |
| `pkg-app` | 38.70 kB | 14.04 kB | 2.6% | zustand + react-router |
| `pkg-lucide` | 17.61 kB | 6.13 kB | 1.2% | 图标 |
| **合计** | **1466.54 kB** | — | 100% | 与审计原文 1,464,712 B 一致 ✓ |

★ **关键**：只把 `recharts` 本身算作 264.92 kB 会**低估一半** —— 它的传递依赖散在 `pkg-other` 里。
按「家族」合并后是 **407.73 kB / 27.9%**，而它**只被 4 个统计页用到**
（`TokenStatsPage` / `CreditStatsPage` / `TraeTokenStatsPage` / `TraeCreditsPage`）。
⇒ **B2（路由懒加载 + manualChunks）确认值得做**，收益上限 ≈ 首屏 JS −28%（约 110 kB gzip）。

### 5.2 P0-4 token 统计：**数据规模已量出，成本确凿**

| 输入 | 实测（本机 2026-09-28） |
|---|---|
| `~/.workbuddy/projects`（CN） | **472** 个 jsonl / **889 MB** |
| `~/.workbuddy-ai/projects`（Global） | **480** 个 jsonl / **1.9 GB** |
| 合计 | **952 个文件 / ≈2.8 GB** |
| 纯 I/O 下界（`cat` 全部文件，热缓存） | **2.42 s** |

⇒ `region=all` 的「实时聚合、不缓存」意味着**每个请求**都要过 ≈2.8 GB（I/O 下界 2.4 s，
**外加 JSON 解析**，实际远高于此）。**成本确凿，缓存应当做**。
但「不缓存、不落库」被写成设计属性（`token_stats.rs:1048`）且无理由 ⇒ **仍建议加内存缓存、
绝不落盘**，并在实现时给出明确失效条件（按文件 `(mtime, size)` 判定）。
`CodeBuddyExtension/Data` 本机仅 **24 KB** ⇒ 该数据源不是瓶颈。

> ⚠️ **两处已被 2026-10-10 的实际结果推翻，留作对照：**
> 1. **「2.8 GB / I/O 下界 2.4 s」严重低估** —— 日志目录随使用增长，
>    到 10-10 已达 **34.5 GB / 12356 文件**，实测纯读盘 **65 s**。**别再用这组数字做容量假设。**
> 2. **「绝不落盘」被反转** —— 理由是「怕落库聚合值与真实日志不一致」，而该理由
>    对 `(path, mtime, size, cutoff)` **内容指纹键**不成立（文件一改必然未命中）。
>    落盘后冷启动从 79 s 降到 1.2 s，见下一节。

### 5.3 P1-8 `dir_stats`：**本机为 ~0，降级**

`~/.buddy-switch/trae/profiles*` 三个快照目录**文件数均为 0**（总 30 KB）。
⇒ 本机不存在该成本；只有「用户确实做过大量快照」时才有量级。**从 B7 降级为「有实测数据再做」**。

### 5.4 仍未实测（保持待确认）

1. **P1-3 分级后各命令的真实耗时**：需命令耗时日志确认哪些同步命令确实慢。
2. **P0-3 的设备派生成本占比**：需 profile 确认「哈希派生」与「文件 parse」谁是大头。
3. **前端 `memo` / 排序的收益**：需 React Profiler 确认（账号数通常为个位数到几十）。

---

## 6. 分批实施路线图

每批**可独立提交、独立验证**，批次内改动互不依赖。**已按复核结论调整**（见批注）。

| 批次 | 内容 | 风险 | 验收方式 |
|---|---|---|---|
| **B0（新增）** | 纯量化，不改代码：拆 chunk 看 recharts 占比；真机计时 token 统计与 `dir_stats`；确认 P0-4 的「不缓存」意图 | 无 | 拿到数字后再决定 B2/B4 是否值得做 |
| **B1** | P0-1 第一步（全量 tasklist，**优先 `wait_windows_pids_gone`**）+ P0-5（缓存头）+ P1-6b（前端**只加可见性门控**，不动间隔）+ P1-6c（store selector） | 低 | 对照测试 + 前端手测 + `check:api` |
| **B2** | P0-2（懒加载 + manualChunks）—— **依赖 B0 的量化结论** | 低 | 构建产物体积对比 + 截图核对 + 重编宿主 + 嵌入护栏 |
| **B3** | P0-3（设备派生缓存 + mtime 缓存） | 中 | CN/Global 隔离 + 写后可见 三组护栏 |
| **B4** | P0-4 中**无争议的那半条**（`ide_workspace_meta` 提出循环外）+ P1-2（`api.rs:1582` 与邻居对齐） | 低 | 等价性测试（逐字节相同）+ 现有测试全绿 |
| **B5** | P1-1（网关锁粒度）+ P1-2 其余 + P1-5（`Arc` 缓存） | 中高 | 网关 e2e + 并发压测 |
| **B6** | P1-3（**仅慢 IO 的那些**命令异步化） | 中 | `check:api` 门禁 + 桌面端手测 |
| **B7** | P2 清单（顺手清理；`memo`/排序**需先有 Profiler 证据**） | 低 | 现有测试全绿 |
| **B8** | P1-4：决定粘性机制**去留**（接上 / 移除）—— 与性能无关，属清理 | 中 | 若接上需登记后台任务表 |
| **B6** | P1-3（Tauri 命令异步化，55 个，可分批） | 中 | `check-api` 门禁 + 桌面端手测 |
| **B7** | P2 清单（顺手清理） | 低 | 现有测试全绿 |

**通用纪律（本项目已固化，务必遵守）**
- cargo 一律串行 `-j 1` + `CARGO_INCREMENTAL=0`；构建/测试用 Bash。
- 改前端后**必须重编宿主**，且重编前**先停掉在跑的 serve**（否则 `LNK1104` 是锁冲突不是代码错误）。
- 反向验证的注入窗口尽量短，残留变异标记提交前必须为 0。
- 若多会话并行改同一仓库，先看 `mtime` 与改动集合是否超出自己这一轮，**别 `git checkout` 别人的文件**。

---

## 7. 「不破坏功能」约束汇总（改之前先读这一节）

| 修复 | 绝不能改的语义 |
|---|---|
| P0-1 进程枚举 | `None => true`（查询失败视为运行中）；精确映像名匹配；全量 CSV 列位 |
| P0-2 懒加载 | `Suspense` 必须在路由层；`dist`/`dist-demo` base 差异；嵌入资源护栏 |
| P0-3 网关池 | 账号库按 region 分家；`sync_for` 校正 `self.variant`；写后立即可见 |
| P0-4 统计缓存 | 输出逐字节等价；`aiTitle` 优先且不受 cutoff 影响；缓存可重置 |
| P0-5 缓存头 | `index.html` 必须 `no-cache`；回退时 MIME 按实际资源名推导 |
| P1-1 网关锁 | 增量 upsert、不删账号（保留治理历史） |
| P1-3 Tauri 命令 | 命令名与参数名不变（`check-api-contract.cjs` 门禁） |
| P1-4 粘性机制（原判「GC 泄漏」**已撤回**） | 若接上写路径：GC 不驱逐未过期绑定；后台循环**必须登记进 `BackgroundTask` 枚举**（`server/main.rs:33-57`「登记即执行」） |
| P1-5 Arc 缓存 | 调用方需要 `&mut` 时的处理路径 |
| **P1-6a 2s 轮询（有意为之）** | **间隔数值不得改动**（用户可开关的「日志实时跟随」功能）；只可加可见性门控 |
| **M3 `freshMs=0`（有意为之）** | **不得按缺陷修改**；改 `freshMs` 属产品层面的调优取舍 |

---

*本报告为纯静态分析 + 本机实测，未修改任何项目文件。*
