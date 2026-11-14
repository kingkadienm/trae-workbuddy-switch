# 用量页网关明细（issue #63）实施方案

> 状态：**待拍板**（2026-10-21）。拍板后先改 `docs/prototype/`，原型复核通过才动代码。
> ⚠️ **UI 口径已由 [`USAGE-GATEWAY-UI-REDESIGN.md`](USAGE-GATEWAY-UI-REDESIGN.md) 取代**：本文的「模型 tab 加含网关开关」作废（改为页面级「账本」参数 + 拆表）；数据契约、落盘位置、升级播种、验收用例仍然有效，并按新 UI 扩到 model / account / endpoint 三个维度。
> 关联：issue #63、`docs/specs/P3-ARCH.md`（用量四视图）、`crates/buddy-switch-core/src/modules/token_daily.rs`、`crates/buddy-switch-gateway/src/logging.rs`。

## 结论

1. **趋势视图的「区间总 Token 不求和」是对的，不改口径**。错的是**标签**（叫「总」却只含客户端）+ **没有第二个明细出口**。
2. **模型视图可以并入网关**，但数据必须**落盘成维度**，不能读 `gateway_logs.json`（环形 200 条，30 天窗口下几乎全空 ⇒ 假功能）。
3. **会话视图永远并入不了网关**：网关请求没有 sessionId。不伪造、不猜归属，只加一行说明。
4. 网关页请求日志**加小计条**（按模型 / 按账号），数据现成，零后端改动。
5. 存储位置选 **扩展 `token_daily` 的 `DayRecord`**，与现有 `gatewayIn/Out` 同轮、同水位线 ⇒ **按模型合计恒等于「网关 Token」卡**。这是选型的关键理由：不给用户第二套互相打架的数字。

---

## 1. 诊断（与 issue 一致，补两条源码层结论）

| 现象 | 根因 |
|:--|:--|
| 区间总 Token 与网关 Token 并排、加起来对不上 | `total` 只累加 `dailyModelSeries` 的 client 柱；网关走 `get_gateway_token_trend` 独立小图 |
| 模型 / 会话视图一条网关调用都没有 | 模型 / 会话视图只吃 `token_stats`（扫 `projects/*.jsonl`）；`token_daily` 的 gateway 桶只有 `input`/`output`，**没有 model 维度** |
| 网关明细离开请求日志页就没了 | `gateway_logs.json` 是 FIFO 环形窗口（`log_keep` 缺省 200），只支撑「最近」，撑不起按天/按模型聚合 |

补充两条 issue 没写的：

- **同一请求会被记两次**：客户端走网关时，客户端自己的 jsonl 里也有一条。所以「客户端 + 网关」求和**天然含重复计数**，只能作参考值，不能作账单。现有代码注释拒绝求和是对的，本方案不推翻它。
- **`parse_gateway_rows` 会丢掉 `account` 为空的行**（`filter(|a| !a.is_empty())`）。所以「无可用账号 503」「账号未解析的 429」这类请求既不在 token 桶里，也不该单独出现在模型维度里 —— 新维度必须**沿用同一个过滤器**，否则两个数字开始分叉。

## 2. 硬事实：会话维度并入不了

网关请求日志的字段集是钉死的（`logging.rs::log_entry_key_set_is_pinned`）：

```
ts / endpoint / method / region / account / accountUid / model / status / latencyMs / totalMs / promptTokens / completionTokens / stream
```

**没有 sessionId，也没有对话 id**。网关是「一个请求一个请求转发」的，它不知道这个请求属于哪个会话。

所以对「会话视图看不到网关」只有一个诚实答案：**看不了**。三个选项：

- ❌ 按 (账号, 模型, 天) 造伪会话 —— 那是编数据，会让「哪个会话最贵」变成假答案；
- ❌ 按账号聚合冒充会话 —— 换个名字骗人；
- ✅ **会话视图保持客户端口径 + 一行说明 + 指向网关页**（推荐）。

`docs/specs/P3-ARCH.md` 契约 8 关于「气泡数 = 会话数」的语义因此不被破坏。

## 3. 选型

| | A. 前端直读 `gateway_logs` | B. 网关写路径新增维度落盘 | **C. 扩展 `token_daily`（推荐）** |
|:--|:--|:--|:--|
| 数据完整性 | 只有最近 200 条；30 天窗口 ≈ 全空 | 完整（每请求即记） | 受环形窗口约束（与现有网关桶**同一条**边界） |
| 跨天历史 | 无 | 有（新文件从上线起累积） | 有（90 天，与 gateway 趋势同源） |
| 与「网关 Token」卡是否自洽 | 否（两个数字对不上） | **否**（写路径独立计数，与 `token_daily` 的水位线口径会漂） | **是**（同一轮、同一水位线、同一过滤） |
| 是否改网关写路径 | 否 | **是**（打破「只读上游」约束） | 否 |
| 改动面 | 最小 | 大（新文件 + 落盘节流 + 两宿主接线） | 中（一个模块 + 一条命令） |
| 结论 | 只够做「今天」 | 未来若要「精确到请求」，再升级到它 | **本期采用** |

**为什么不选 B**：B 的数据更全，但它会和 `token_daily` 的网关桶**各算一套**。用户看到「网关 18.5M」和「按模型合计 18.9M」时会先怀疑软件坏了。自洽性比完整性更值钱，且 C 的缺失面（2 分钟内 >200 条会丢）与现状**完全一致**，不是新增缺陷。

## 4. 数据契约

### 4.1 落盘（`token_daily.json`，`TOKEN_DAILY_VERSION` 2 → 3）

```jsonc
"2026-10-21": {
  "accounts": { "uid-a": { "client": {...}, "gateway": { "input": 18421567, "output": 0 } } },
  "unknownClient": null,
  "gatewayTruncated": false,
  "gatewayWatermarkTs": 1761031800000,
  // 新增
  "gatewayModels": {
    "deepseek-v4.1-flash": { "input": 18421567, "output": 0, "requests": 74, "failed": 0 },
    "__unknown__":          { "input": 0,        "output": 0, "requests": 3,  "failed": 3 }
  },
  "gatewayAccounts": { "uid-a": { "input": 12186150, "output": 0, "requests": 43, "failed": 0 } },
  "gatewayEndpoints": { "/v1/chat/completions": { "requests": 75, "failed": 5 } },
  "gatewayModelsPartial": false
}
```

- `gatewayModels`：键是模型名（`model` 为空 ⇒ 哨兵 `__unknown__`，渲染「未标注模型」）。
- **`gatewayAccounts` / `gatewayEndpoints`**：UI 重设计（`USAGE-GATEWAY-UI-REDESIGN.md` §2.2）要求会话 tab 在网关账本下换成「按账号 / 按端点」，且跨 7 天 ⇒ 必须落盘，环形窗口撑不住。三者**在同一次聚合里顺手产出**，成本不变。
- `requests` / `failed`（`status >= 400`）：成本 0，让「429 × 5」能落到模型视图。
- `gatewayModelsPartial`：升级过渡标记。见 §5.2 的播种规则。
- 版本号 3；旧文件缺字段 ⇒ `default` 空 ⇒ 走播种，不丢已有桶。

### 4.2 命令 `get_gateway_token_models`（GET `/api/account/token-models?region=`）

```jsonc
{
  "date": "2026-10-21",
  "retentionDays": 90,
  "partialDays": ["2026-10-20"],          // 模型拆分不完整的日（升级过渡）
  "days": [                                // 只含落库日，升序；缺日 = 未记录，不是 0
    { "key": "2026-10-21", "truncated": false,
      "models": [ { "model": "deepseek-v4.1-flash", "input": 18421567, "output": 0,
                    "total": 18421567, "requests": 74, "failed": 0 } ] }
  ]
}
```

与 `get_gateway_token_trend` **同源同形状**（`days` + `truncated` + `retentionDays`），前端复用同一套覆盖率计算。

### 4.3 命令 `get_usage_reconcile`（GET `/api/account/usage-reconcile?region=&days=`）

回答「客户端与网关重叠多少」（`USAGE-GATEWAY-UI-REDESIGN.md` §4）。**只读、不落盘**。

```jsonc
{
  "window": { "from": "2026-10-15", "to": "2026-10-21" },
  "clientRows": 117, "gatewayRows": 126,
  "matched": 9, "matchedTokens": 350000,
  "bounds": { "lower": 28500000, "upper": 35010000 },   // max(C,G) .. C+G
  "estimate": 34660000,                                 // C + G − matchedTokens
  "logCoverage": { "from": "2026-10-21T17:01:00+08:00", "to": "…18:10:00+08:00",
                   "covered": 126, "total": 126 },
  "verdict": "independent",                             // independent | subset | mixed
  "unmatchedSamples": [ { "ts": 1761031452000, "model": "step-5-preview", "tokens": 42100, "why": "model_missing" } ]
}
```

- 入参 = 窗口（**外加一个配置信号**：网关代理是否开启，用于 `verdict` 的双信号判定，见 UI 文档 §4.4.1）；读 `gateway_logs.json`（≤ `log_keep` 条）+ **窗口内**客户端逐消息记录 `(ts, model, input, output)`（复用 `token_stats` 的解析，只保留窗口内的元组）。
- 匹配：同模型 + `|Δt| ≤ 120s` + prompt/completion 各差 ≤ 1% + 一对一贪心（最近 Δt 优先）。**账号不参与匹配** —— 网关切号会让两端账号对不上（UI 文档 §4.4.1）。纯函数 `reconcile(client_rows, gateway_rows, proxy_enabled) -> ReconcileReport`，可单测。
- `verdict` 双信号：匹配率 + `proxy_enabled`。`< 10%` ⇒ `independent`；`> 90%` **且** 代理开启 ⇒ `subset`；其余 `mixed`（匹配率高但代理没开 ⇒ 更可能是别的客户端在跑同样模型）。
- **不落盘**：对账结果随日志窗口滚动而变，写进 `token_daily` 会让历史数字漂移（UI 文档 §4.7）。

### 4.4 前端类型

```ts
export interface GatewayModelUsage { model: string; input: number; output: number; total: number; requests: number; failed: number }
export interface GatewayUsageDimDay { key: string; truncated: boolean; models: GatewayModelUsage[] }
export interface GatewayUsageDim { date: string; retentionDays: number; partialDays: string[]; days: GatewayUsageDimDay[] }
```

## 5. 后端改动

### 5.1 `crates/buddy-switch-core/src/modules/token_daily.rs`

1. `GatewayRow` 加 `model: String`、`status: u16`；`parse_gateway_rows` 填充（`model` 空 ⇒ `__unknown__`）。
2. 新增 `struct ModelBucket { input, output, requests, failed }`、`fn bucket_gateway_models(rows, region, day, since) -> BTreeMap<String, ModelBucket>`（**过滤器与 `bucket_gateway` 逐字一致**：region + day + `ts > since`）；`bucket_gateway_accounts` / `bucket_gateway_endpoints` 同形（endpoint 桶只要 `requests` / `failed`）。
3. `gateway_round` 的返回扩成：

```rust
struct GatewayRound {
    accounts: GatewayWrite,              // 现有 Add / Correct
    models: GatewayWrite<ModelBucket>,   // 同一水位线决定 Add / Correct
    next_watermark: Option<i64>,
    models_partial: bool,                // 播种且该日已有账号桶 ⇒ true
}
```

4. `record_gateway_today` 落 `gateway_models`（`Add` 相加 / `Correct` 逐模型取大）与 `gateway_models_partial`。
5. 新增 `pub fn gateway_token_models(region: Region) -> Value`（只读、不触发记录、不扫盘），与 `gateway_token_trend` 并列。
6. 版本常量 → 3。

### 5.2 升级播种规则（必须显式，否则升级当天数字会断层）

- 水位线**缺失**（v1 旧记录）⇒ accounts 与 models 都走 `Correct`（现有行为）；
- 水位线**存在**但 `gatewayModels` 为空（v2 → v3 升级过渡）⇒ **models 单独走一次 `Correct`**（取当前窗口读数）；
  - 若该日账号桶已有值（真·历史数据）⇒ 置 `gatewayModelsPartial = true`，前端披露「该日模型拆分不完整」；
  - 若账号桶为空（新的一天）⇒ 不置标记（整窗即全量）。

### 5.3 宿主接线（三处缺一不可）

| 位置 | 改动 |
|:--|:--|
| `src-tauri/src/commands.rs` | `get_gateway_token_models(region)` 命令 |
| `src-tauri/src/lib.rs` | `generate_handler!` 注册 |
| `crates/buddy-switch-server/src/api.rs` | `.route("/api/account/token-models", get(...))` + handler |

`cargo check -p buddy-switch-rust` 必须跑（前端门禁抓不到宿主编译错）。

## 6. 前端改动

### 6.1 `src/lib/usage-charts.ts`（纯函数，可被门禁求值）

```ts
export function gatewayModelTotals(dim, window): Map<string, { total: number; requests: number; failed: number }>
export function mergeModelSlices(client: ModelSlice[], dim, window):
  { slices: ModelSlice[]; coverage: GatewayCoverage; gatewayTotal: number; gatewayRequests: number; gatewayFailed: number }
export function gatewayDimCoverage(dim, window): GatewayCoverage   // 复用 gatewayCoverage 的判据
```

规则（写进注释，别让人后来改）：

- 同名模型**合并**（`value` 相加，`spark` 逐日相加）；
- 网关独有模型**进榜**（它确实是模型用量），`spark` 只取网关逐日；
- `total === 0` 的模型仍过滤掉（与 `modelSlices` 同判据）；
- 未记录日不参与（缺日 = 未记录，不是 0）。

### 6.2 `src/pages/TokenStatsPage.tsx`

| 视图 | 改动 |
|:--|:--|
| 趋势 | ① tile「区间总 Token」→ **「客户端 Token」**（`wbStats.usage.trend.total`），title 补「不含网关」；② 网关小图图注追加 `客户端 + 网关 = 21.3M（参考值 · 可能重复计数）` —— **新增独立函数 `combinedReferenceCaption`**，不动 `gatewayTrendCaption` 的签名（现有门禁按名断言）；③ 网关 tile 的 hint 从「仅网关日志」→「覆盖 N/M 天」 |
| 模型 | 顶部加分段控件 **仅客户端 / 含网关**（默认仅客户端，**不静默改变老用户数字**）；含网关时 tile / 环形 / 排行全部按合并值算，图注披露 `网关覆盖 N/M 天 · 429 × 5 · 同一请求可能两处各记一次`；排行每项第二行小字 `网关 74 条` |
| 会话 | 空态与有数据两种情况都加一行说明：`网关调用没有会话 ID，无法归入会话视图 —— 按模型看请切到「模型」并打开「含网关」` |

数据获取：与 `getGatewayTokenTrend` **并行**取（`Promise.all`），单区失败只让该区缺席（`catch(() => null)`），失败 ⇒ 开关置灰 + 说明，不弹错。

### 6.3 网关页请求日志小计（零后端改动）

- 新文件 `src/lib/gateway-log-summary.ts`：`summarizeLogs(logs)` → `{ total, span: {from,to}, byModel: [...], byAccount: [...], failed }`（纯函数，可单测）。
- `src/components/gateway/request-log.tsx`：表格**上方**一行两列小计（按模型 / 按账号，各取前 5 + 「其余 N」），标题注明 `最近 N 条 · 17:01–18:10（受 log_keep 限制）`。
- 数据来自 `useGatewayStore` 已加载的 `logs`（`loadLogs` 已按 `ts` 降序），**不新增请求**。

### 6.4 其他必须同步的地方

| 文件 | 为什么 |
|:--|:--|
| `src/lib/api.ts` | `ROUTES`（method/path）+ `DEMO_READONLY_COMMANDS`（**不登记 ⇒ `build:demo` 报错**）+ `call()` + `getGatewayTokenModels()` |
| `src/lib/screenshot-demo.ts` | `case "get_gateway_token_models"` 演示数据（覆盖「有量 / 缺日 / partial」三态） |
| `src/locales/domains/wbStats.zh.ts` / `.en.ts` | 新键**两份都要加**（缺 en 会回落成键名） |
| `src/pages/TraeTokenStatsPage.tsx` | **不动**：Trae 侧数据源形状不同，本次只做 WB |

## 7. 原型（阶段一交付物，先做这个）

`docs/prototype/`：

1. `demo-data.js`：每个 scope 的 `ext` 加 `gateway: { days: [{label, value}], models: [{name, values}], requests, failed, coverage: {recorded, total} }`。
2. `demo-app.js`：
   - `state.usageModelsGateway = false`；
   - 趋势 tab：tile 1 改「客户端 Token」、加「网关 Token」tile（hint 覆盖 N/M 天）、加网关单段柱小图（图注含合并参考值）；
   - 模型 tab：顶部 `seg`「仅客户端 / 含网关」，打开后 tile / 环形 / 排行按合并值重算，图注披露覆盖率与 429；
   - 会话 tab：说明条；
   - 网关页 `logs` 分区：表格上方小计条（按模型 / 按账号）。
3. 新增 `docs/prototype/verify-gateway-usage-detail.cjs`（JSDOM，手跑）：开关切换后模型数与总量**真的变**、会话说明存在、小计条合计 === 表内可见行合计、`区间总 Token` 文案已改。
4. 跑现有门禁：`node docs/prototype/verify-demo.cjs`、`verify-usage-window.cjs`、`npm run check:scope`（它读 `demo-app.js`，改坏会红）。

**原型复核点（要你确认的四个决定）**：

- 模型视图开关默认 **关**（保持现状）还是 **开**（让网关默认可见）？
- 合并参考值放**图注**还是**独立 tile**？
- 会话视图是**只加说明**，还是要一个「按账号」的替代视角？
- 网关页小计取 **前 5 + 其余**，还是**全列**（`log_keep` 上限 200 条，全列也不长）？

## 8. 诚实边界（四条，必须落到 UI 文案里）

1. **缺日 = 未记录，不是 0**（App 没跑 = 后台任务没记）。
2. **`truncated` 日只是下界**（日志窗口滑过当天）。
3. **模型合计 = 网关 Token 卡**（同源同轮）；但**客户端 + 网关 ≠ 账单**（同一请求可能两处各记一次）。
4. **`gatewayModelsPartial` 日**：模型拆分不完整（升级过渡）。

## 9. 验收（可证伪，缺一条不算完）

**Rust（`token_daily.rs`）**

- `gateway_model_dimension_sums_to_the_gateway_bucket`：同一份日志跑一轮，`Σ gatewayModels.total === Σ accounts[].gateway`（**选型 C 的存在理由，红了就说明两条口径分叉了**）。
- `gateway_model_round_is_idempotent`：同窗重跑，模型量不变。
- `gateway_model_dimension_survives_log_rotation`：窗口滚掉旧条目后，已记的模型量不倒退。
- `legacy_v2_day_seeds_models_once_and_marks_partial`：真文件 + `HomeOverrideGuard`。
- `unknown_model_falls_into_sentinel`：`model` 为空 ⇒ `__unknown__`，不丢量。

**Rust（`reconcile.rs`，纯函数）**

- `reconcile_matches_one_to_one`：一条网关记录**最多**抵一条客户端记录（重复 token 的两条客户端记录不会都算重叠）。
- `reconcile_ignores_account`：同一条请求两端账号不同（网关切号）**仍必须匹配上** —— 拿账号当键会把 `subset` 误判成 `independent`。
- `reconcile_leaves_retry_rows_unmatched`：1 条客户端 × 2 条网关（内部降级重试）⇒ 只匹配 1 条，另一条留在网关侧（通常 0 token）。
- `verdict_requires_proxy_signal`：匹配率 95% 但 `proxy_enabled = false` ⇒ 不判 `subset`（那是别的客户端在跑同样模型）。
- `reconcile_rejects_time_gap_over_120s` / `reconcile_rejects_token_drift_over_1pct`：边界两侧各一条用例。
- `bounds_hold_for_any_overlap`：随机 overlap ∈ [0, min(C,G)] 下 `max(C,G) ≤ estimate ≤ C+G`。
- `verdict_thresholds`：匹配率 0.05 / 0.5 / 0.95 分别产出 `independent` / `mixed` / `subset`。
- `reconcile_is_read_only`：跑完 `token_daily.json` 与 `gateway_logs.json` 的 mtime / 内容不变。

**TS**

- `bookModelRows(book)`：只读单本账，`Σ rows.total === 该账本合计`（**不做跨账本合并** —— UI 文档 §4.7 第 2 条）。
- `summarizeLogs` 的 `Σ byModel.tokens === Σ rows.tokens`（含 `null` 字段按 0）。
- `combinedTile(recon)`：`value === recon.estimate`，`hint` 里的算式三项与 `recon` 字段逐项相等（防止卡片与抽屉说两套话）。

**门禁**

```
cargo test -p buddy-switch-core token_daily
cargo check -p buddy-switch-rust
npm run build            # tsc + check:api + check:docker + check:store-selectors + check:scope + …
node docs/prototype/verify-gateway-usage-detail.cjs
```

## 10. 分期

| 期 | 内容 | 改动面 |
|:--|:--|:--|
| **P0** | 改名 + tooltip + 合并参考值图注 + 网关页小计条 | 纯前端，**不动契约** |
| **P1** | 模型维度落盘 + `get_gateway_token_models` + 模型视图「含网关」 | core 一个模块 + 三处宿主接线 + 模型视图 |
| P2（可选） | 按账号 30 天维度、模型视图的失败率筛选 | 复用 P1 管线 |

**不做**：会话视图并入网关；把两块求和写进「区间总 Token」；读 `gateway_logs.json` 做跨天聚合；改网关写路径；给 Trae / Qoder 做同款（另开 issue）。

## 11. 风险

| 风险 | 处置 |
|:--|:--|
| 升级当天模型拆分与网关卡不一致 | `gatewayModelsPartial` + 图注披露（§5.2） |
| `token_daily.json` 变胖 | 90 天 × 模型数（实测量级 ~10）≈ 千行级；仍走 `atomic_write`，无性能问题 |
| 用户把「客户端 + 网关」当账单 | 只出现在图注、标「参考值 · 可能重复计数」；不新增独立 tile（除非原型复核要求） |
| 演示站崩 | 新命令必须登记 `DEMO_READONLY_COMMANDS` + `screenshot-demo.ts` case |
