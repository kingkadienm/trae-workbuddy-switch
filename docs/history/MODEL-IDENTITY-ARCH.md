# 同名模型的多来源接入方案（WorkBuddy 客户端 / 多平台 / 多版本）

> **状态：只读历史（不再更新，2026-10-17 归档）** —— 已被 `docs/specs/UNIFIED-GATEWAY-ARCH.md` 取代（同日改走命名空间方案）。
> 现行文档见 [`docs/README.md`](../README.md)（文档地图）。
> 状态：**已被 `UNIFIED-GATEWAY-ARCH.md` 取代（2026-10-14 同日）** —— 用户选择「合并两个产品线 + 模型 id 带命名空间」，
> 即本文件 §4 的 L2 从「可选终局」升为「主线的第 2 步」（命名空间解析器），L0 / L1 降为过渡手段。
> **读那份，这份只留作 L0–L3 取舍的推导记录。** 本文件中仍然有效的独立结论：§2 现状事实、§6 L3 端口方案的否决理由。
> 日期：2026-10-14。作者：AI 助手。依据：**本仓源码实测**，凡未核实处显式标注「未核实」。
> 需求原话：「traework 和 workbuddy 的部分模型有重叠，国际版和国内版也有重叠。如果我使用网关，怎么添加多个不同平台和不同版本的同个模型呢？比如 workbuddy 的客户端想添加多个 `deepseek-v4.1-flash` 模型会被识别已经存在、会被覆盖。」

---

## 1. 结论先给

**这不是「客户端缺个去重开关」，而是「模型 id 里不含来源」的结构性问题。** 修法只有一条：**让网关对外暴露的 id 唯一**，或**让客户端按 provider 分组**。二选一，不能绕过。

推荐组合（按优先级）：

| 层 | 手段 | 代价 | 解决哪些组合 |
|:--|:--|:--|:--|
| **L0** | 多 Key × 多 provider（**零改动**） | 0 | 客户端按 `provider+模型id` 分组的：WB↔Trae、WB-CN↔WB-Global、TraeWork↔TraeCode 全部 |
| **L1** | 打开既有开关 `allow_model_region_prefix` | 0（改一个 JSON 字段） | **仅 WB** 的 CN↔Global（跨域前缀） |
| **L2** | 网关侧**模型别名（namespace）** | 一次完整改造 | **全部组合、全部客户端**，且**可被 `/v1/models` 自动发现** |
| **L3** | 每域一个监听端口 | 重 | 全部，但会打破既有设计（见 §6） |

**我的建议：先做 L0 验证你的客户端到底能不能分开，能就停手；不能才做 L2。** L1 只是个临时补丁，别当终局（它只覆盖 WB、且前缀名不进 `/v1/models`）。

---

## 2. 现状事实（全部可在本仓核实）

1. **两个产品线 = 两个网关、两个端口、两套 Key 库**：WB `127.0.0.1:57891`，Trae `127.0.0.1:7864`。端口**不能合并** —— 两条路由的 `/v1/models`、`/v1/chat/completions` **逐字相同**，axum 同路径 `merge` 会**运行期 panic**（`crates/buddy-switch-server/src/gateway_host.rs` 模块头已写明）。
2. **Key 决定来源，模型名不决定**：
   - WB Key 绑 **region**（`cn` / `global`）—— `routes/models.rs` 用 `record.region` 取目录，`routes/chat.rs` 用 `region` 选账号池；
   - Trae Key 绑 **variant**（4 个程序位 `trae_work` / `trae_cn` / `global` / `global_trae_code`）。
3. **模型 id 就是上游名字，不含平台/区域信息。** 这就是重叠的根：
   - WB CN 兜底目录含 `deepseek-v4.1-flash`（`core/modules/catalog.rs:160`）；
   - WB Global 兜底目录**也含** `deepseek-v4.1-flash`（同文件 `:187`，两者 context_window 还不一样：1M vs 300k）；
   - Trae 的清单同样来自上游 `name` 字段（`trae/model_list.rs`），与 WB 名义重叠。
4. **网关不校验模型名。** WB 侧把 `model` 字段改写后直接转发（`routes/chat.rs`），Trae 侧走 `payload::model_config` **派生**（`name` + `__dev`）。⇒ 别名层只是「名字 → 名字 + 目标域」，**成本很低，不需要动上游层**。
5. **已有一个默认关闭的机制**：`allow_model_region_prefix`（`gateway/state.rs:85`，落盘 `~/.buddy-switch/gateway_config.json`，键名 snake_case）。
   - 关闭（默认）：**同域**前缀（`cn:` 当 Key 是 CN）被剥离；**跨域**前缀返回**可诊断的 400**，并明确告诉你开关名和出路（`gateway/model_route.rs::rejection_message`）。
   - 开启：`global:` 前缀可跨域路由。
   - ⚠️ **只存在于 WB**：`model_route` 只被 `routes/{chat,messages,responses}.rs` 引用，Trae 侧没有等价物。
   - ⚠️ **前端没有这个开关的 UI**（全仓 `src/` 无 `region_prefix` 字样）⇒ L1 只能手改 JSON。
6. **`/v1/models` 不列前缀名**，只列裸 id ⇒ 客户端**无法自动发现** `cn:deepseek-v4.1-flash`，必须手抄。
7. **模型禁用（`disabled_models`）按 `(region, 模型id)` 存**，且是**两个出口共用的唯一判断点**（`is_disabled`）：`/v1/models` 过滤 + 调用路径硬拒（403 `model_disabled`）。改别名必须一并想清楚「按哪个 id 禁用」（见 §4.3）。

---

## 3. L0：多 Key × 多 provider（零改动，先试这个）

**原理**：来源由 **端口 + Key** 唯一确定，不靠模型名。同一个 id 在不同 provider 下**是两个不同的条目**。

| 你想并存的 | 怎么建 | 客户端里怎么配 |
|:--|:--|:--|
| WB 国内版 | 「API 服务」页建 Key，归属 **国内版** | provider A：`http://127.0.0.1:57891/v1` + Key A |
| WB 国际版 | 同页再建一把，归属 **国际版** | provider B：**同一个 URL** + Key B |
| TraeWork 国内 | Trae 页建 Key，归属 `trae_work` | provider C：`http://127.0.0.1:7864/v1` + Key C |
| TraeCode 国际 | 同页建 Key，归属 `global_trae_code` | provider D：同 C 的 URL + Key D |

**适用条件（必须确认）**：客户端把两个 provider 分别记账。**支持**：Codex（`config.toml` 可写多个 `[model_providers.*]`，本仓 `docs/guides/GATEWAY-CLIENT-INTEGRATION.md` §3.1）、Claude Code（多份 `settings.json` / 多套环境变量）、dsh（多 provider，§3.4）、opencode。
**不支持（会去重）**：把模型写成**一份扁平列表**的客户端 —— 典型是 **CodeBuddy `~/.codebuddy/models.json`**（`models[]` + `availableModels[]` 都是按 id 的数组，同 id 必然打架，§3.5.1），以及只让你填**一个** Base URL 的客户端。

### 3.1 需要你回答的一件事

**你现在用的是哪个客户端？** 这是决定走 L0 还是 L2 的唯一变量。若是 CodeBuddy `models.json` 那类扁平列表的，L0 无效，直接看 L2。

---

## 4. L2：网关侧模型别名（推荐终局）

### 4.1 数据与形状

**独立文件，不进 `GatewayConfig`**（与 `disabled_models.rs` / `account_strategy.rs` 同构，理由同它们的模块头：`GatewayConfig` 的键集被契约测试**精确钉死**；且别名是**用户偏好**不是运行配置）。

- WB：`~/.buddy-switch/gateway_model_aliases.json`
- Trae：`~/.buddy-switch/trae/api_gateway_model_aliases.json`

```jsonc
{
  "regions": {
    "cn": {
      "wb-cn-flash": { "model": "deepseek-v4.1-flash" },              // 目标域省略 = 跟随 Key 的域
      "wb-flash-cheap": { "model": "deepseek-v4.1-flash", "region": "global" }
    },
    "global": { "wb-gl-flash": { "model": "deepseek-v4.1-flash" } }
  },
  "variants": { "trae_work": { "trae-work-flash": { "model": "deepseek-v4.1-flash" } } }
}
```

- 键用 **`String`**（**不是** `Region`/`TraeVariant` 枚举）：未知键不能让整份反序列化失败 —— 这条是 `disabled_models.rs:49` 记下的脚枪，照抄。
- **别名由用户显式命名，网关不自动加后缀。** 自动生成（如 `wb-cn-deepseek-v4.1-flash`）会在上游改名时静默失效，且用户无法表达「我就想叫这个名」。

### 4.2 解析点（唯一，且顺序定死）

新增 `gateway/model_alias.rs`，在三个入口 **`model_route` 之前**调用一次：

```
原始模型名 → ①整串精确匹配别名表（命中 ⇒ 得真实 id + 目标域）
           → ②未命中 ⇒ 走既有 model_route 前缀规则
           → ③都不是 ⇒ 原样透传（现状不变）
```

- **先别名后前缀**：别名是整串精确匹配（可含 `:`），前缀是切第一个 `:` —— 顺序反了会让名叫 `cn-x` 的别名被误切。写成一条硬规则并加护栏。
- 目标域（`region` / `variant`）的越域校验**复用既有兜底**：选出的账号必须属于目标域，否则 `region_mismatch`。**不新写一套越域逻辑。**
- Trae 侧同名实现落在 `trae/routes.rs`，**两平台同一件事同一做法**（硬约束）—— 差异只走数据（`variants` vs `regions`）与那个 `function_for(variant)` 调用。

### 4.3 三处必须一起定（否则是「假开关」重演）

1. **`/v1/models`（出口 ①）是否列别名？** 建议**列**，并加 `aliased_of` 字段标注来源、**同时继续列裸 id**（不破坏既有接入）。理由：只认 id 的客户端正是本需求的受害者，不给它自动发现就等于没解决。
2. **禁用（出口 ②）以哪个 id 为键？** 建议以**对外的那个 id**（开了别名就是别名）为键。理由：用户禁用的是他**在 `/v1/models` 里看见的那一条**。⇒ `is_disabled` 仍是唯一判断点，两个出口同源，不许各判一套。
3. **请求日志记什么？** 记 `model` = **对外 id**（客户端填的那个），另加 `resolved_model` = 真实 id。否则排查时看不出「我填的 `wb-cn-flash` 到底打给了哪个域、哪个模型」。

### 4.4 冲突规则（**这就是用户要的「不再被覆盖」**）

- 同一别名在同一 region / variant 内**唯一**。重复保存 ⇒ **明确报错**，**不静默覆盖**。
- 别名**不得与任何真实模型 id 相同**（保存时查该域目录）⇒ 否则「谁优先」会变成隐式行为。
- 别名冲突检测要在**保存时**做，不能只在解析时「后者胜」。

### 4.5 前端

- 落点：API 服务页 → **「模型」子导航页内新增一个「别名」区块**（`ModelList` 里，或与它并排的卡）。**不新增子导航项** —— `GATEWAY_SUBNAV` 是两平台共用的一张表，加项要两个页面一起改，收益不抵成本。
- 两平台**同一组件**，差异只走数据（`regions` / `variants` 的轴标签）与插槽。
- ⚠️ **8 控件的工具栏是账号页的，别搬过来**；模型页保持现状骨架。
- 每个目的地只有一处页头（子视图禁再渲染标题）。

### 4.6 影响面清单（按仓的「四处同步」纪律）

| 层 | 落点 |
|:--|:--|
| 数据 | `gateway/model_alias.rs`（新）+ 两平台各一个 JSON |
| 解析 | `routes/{chat,messages,responses}.rs`、`trae/routes.rs` |
| 出口 ① | `routes/models.rs`、`trae/payload.rs::models_response_for` |
| 出口 ② | 复用 `disabled_models::is_disabled`（**不得复制逻辑**） |
| 日志 | `RequestMeta`（WB）/ Trae 日志结构，加 `resolved_model` |
| 管理面 | **四处**：`server/api.rs` `.route`、`src-tauri/commands.rs`、`src-tauri/lib.rs::generate_handler!`、`src/lib/api.ts` 的 `ROUTES` + `call()` ⇒ 跑 `npm run check:api` |
| 前端 | 模型页新区块 + `src/lib/types.ts` + 词条（⚠️ 词条一律 `//` 行注释，**禁**裸 `/*`：门禁 `stripComments` 会吞掉后续全部内容） |
| 门禁 | `check:api` / `check:scope` / 契约测试（`GatewayConfig` 键集**不受影响**，因为走独立文件） |
| 后台任务 | 别名**不需要**周期任务 ⇒ **不要**往两侧 `BackgroundTask` 表里加东西 |

---

## 5. L1：打开 `allow_model_region_prefix`（临时补丁，可选）

**怎么做**：编辑 `~/.buddy-switch/gateway_config.json`，把 `allow_model_region_prefix` 置 `true`，然后在「API 服务」页点「刷新」重读配置（配置是读快照的），必要时重启网关。之后客户端里手填：

- `cn:deepseek-v4.1-flash` —— 走国内版账号池
- `global:deepseek-v4.1-flash` —— 走国际版账号池

**为什么只当临时**：① 只 WB，Trae 侧完全没有；② 这两个带前缀的 id **不在 `/v1/models` 里**（§2.6），客户端自动发现拿不到，必须手抄，且客户端会认为「模型不存在」；③ **安全语义变了** —— 原本「Key 的域是硬边界」，开启后客户端可自行越域（`model_route.rs` 模块头把这条写成**有意的安全取舍**）。**要开就得你点头。**

---

## 6. L3：每域一个监听端口（不推荐）

技术上可行（`GatewayHost<P>` 已经在做独立监听），但要为每个 region / variant 各起一个 router 与端口，客户端配置变成 6 个 URL，而网关的既有设计是**每产品线一个端口**。**只有**在「客户端只允许一个 Base URL **且**必须自动发现模型」时才值得。当前不建议。

---

## 7. 需要你拍板的 5 件事

1. **你用的是哪个客户端？** —— 决定 L0 是否够用（若是 CodeBuddy `models.json` 这类扁平列表，L0 直接出局）。
2. **要不要做 L2（别名）？** 我建议做，它是唯一能同时覆盖「4 个平台/版本组合 × 任意客户端」的方案。
3. **别名要不要跨域**（即别名里写 `"region": "global"` 而 Key 是 CN）？如果不允许，别名就纯粹是「改名」，安全性不变，实现也更小。
4. **禁用按「对外 id」还是「真实 id」为键？** 我建议对外 id（用户看见哪个就禁哪个）。
5. **Trae 侧要不要同一期做？** 我建议同一期 —— 「两平台同一件事同一做法」是硬约束，分两批做必然漂移（本仓已因此踩过坑）。

---

## 8. 我不做 / 不承诺的

- **不做「网关自动给模型加区域后缀」**：自动命名会在上游改名时静默失效（§4.1）。
- **不合并两个网关端口**：会触发 axum 运行期 panic（§2.1）。
- **不碰 `/v1/responses` 的 Trae 缺口**：Trae 侧本来就没有这个端点，与本需求无关。
- **不编造客户端配置片段**：L0 的片段只给已被本仓 `docs/guides/GATEWAY-CLIENT-INTEGRATION.md` 核实过的那几个（Codex / Claude Code / dsh / CodeBuddy CLI）；其余以各客户端官方文档为准。
- **未核实项**：你那个客户端到底按什么键去重（`provider+id` 还是裸 `id`）—— 需要你说出客户端名字，或贴一次它报「已存在」的界面/配置。
