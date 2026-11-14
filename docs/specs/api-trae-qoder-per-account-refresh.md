# 接口约定：Trae / Qoder 的「按账号 · 今日」只读重取

**日期**：2026-10-18
**提出方**：账号页（`AccountsPage` / `TraeAccountsPage` / `QoderAccountsPage`）
**状态**：**已实现**（2026-10-18；`get_trae_account_token_today` / `get_trae_checkin_status_map` /
`qoder_checkin_status_map` 三条命令 + 两端接线 + 抽屉指标块）
**影响面**：卡片「更多」菜单与详情抽屉指标块的刷新项（此前两平台各只有「积分 / 凭据状态」）

> **实现期对本文的两处修订**（实现与初稿不一致的地方，以本文为准）：
>
> 1. **`accountIds` 是逗号分隔串**，不是 `string[]`：`httpCall` 的 `queryString` 对数组
>    只会产出 `String(["a","b"])`，桌面端却天然拿到 `Vec<String>` ⇒ 两端形状分叉。
>    统一收一个串（后端 `parse_id_list` / `split_id_list`），前端传 `ids.join(",")`。
> 2. **Trae 的 `clientSource: "unavailable"`**：Trae 没有可解析的客户端会话落盘
>    （对话历史在 IDE 私有存储里），`clientIn/clientOut` 恒为 0 只为**形状同构**，
>    UI 必须把那一档渲染成「不可得」而不是真实 `0`。

---

## 1. 为什么需要新接口（而不是复用现有的）

卡片「更多」里有四项刷新：`积分` / `今日 Token` / `签到状态` / `凭据状态`。
WorkBuddy 四项都能刷；**Trae / Qoder 只开了两项**，因为缺「这个账号、今天」这一粒度的只读数据。

现有的三个批量接口都**不能**直接当单账号今日口径用：

| 现有命令 | 现状口径 | 为什么不够 |
|:--|:--|:--|
| `get_trae_token_statistics`（`GET /api/trae/token-stats`） | 区域批量，`rangeDays` 时间窗 | 是「区间合计」，不是「今天、这个账号」；卡片上也没有对应读数可更新 |
| `get_trae_checkin_status`（`GET /api/trae/checkin/status`） | 区域批量，**最近一次签到运行**的 `results` | `summaryIsToday` 为假时是旧结果；且没有逐账号的「今天签没签」布尔 |
| `qoder_checkin_status`（`GET /api/qoder/checkin/status`） | 区域批量 | 同上，且 Qoder 的签到是**写操作**（`qoder_checkin_claim`），读侧只有列表快照 |

> 结论：需要两条**按账号、今日口径、只读**的命令。字段名**照抄 WorkBuddy 的同类命令**
> （`get_account_token_today` / `get_checkin_status_map`），前端三平台就能共用同一套渲染。

---

## 2. 两条命令

### 2.1 今日 Token（按账号）

| 项 | 值 |
|:--|:--|
| 命令名 | `get_trae_account_token_today` / `get_qoder_account_token_today` |
| HTTP | `GET /api/trae/account/token-today`、`GET /api/qoder/account/token-today` |
| 参数 | `scope`（`trae-cn` / `trae-global` / `qoder-cn` / `qoder-global`，**fail-closed**）、`force?: bool`（缺省 `false`） |
| 返回 | `AccountTokenTodayResponse`（**与 WB 同构**，见 `src/lib/types.ts:85`） |

```ts
interface AccountTokenTodayResponse {
  date: string;                 // 本地日期键 "YYYY-MM-DD"
  accounts: { uid: string; clientIn: number; clientOut: number;
              gatewayIn: number; gatewayOut: number }[];   // 缺席 = 今日无记录/未知
  unknownClient: { in: number; out: number };
  gatewayTruncated: boolean;    // 网关日志今日部分可能被裁 ⇒ 只是下界
}
```

**语义边界（必须遵守，前端会照此渲染）**

1. **缺席 ≠ 0**：`accounts` 里没有这个 uid ⇒ 前端显示「—」。真实 `0` 必须显式给 `0`。
2. `force=false` 时后端可按 TTL 走缓存（与 WB 一致，避免每次进页面全量重扫）；用户点刷新传 `true`。
3. `gatewayTruncated=true` ⇒ 前端渲染成 `≥N`（不是精确值）。

### 2.2 签到状态（按账号）

| 项 | 值 |
|:--|:--|
| 命令名 | `get_trae_checkin_status_map` / `get_qoder_checkin_status_map` |
| HTTP | `GET /api/trae/checkin/status-map`、`GET /api/qoder/checkin/status-map` |
| 参数 | `scope`、`accountIds`（**逗号分隔**的 uid 串，批量，最多 50 个；空串直接返回 `{}`） |
| 返回 | `Record<uid, CheckinStatusEntry>`（**与 WB 同构**，见 `src/lib/api.ts:1457`） |

**语义边界**

1. **未知不进 map**（不要用 `false` 表示「不知道」）—— 前端对缺失项显示「—」。
2. **读失败 ≠ 未签到**：读失败回 `{ ok: false, todayCheckedIn: false, error: "…" }`，
   前端据 `ok` 区分「没签到」与「读不到」；`ok: true` 时 `todayCheckedIn` 才是事实。
3. 只读：绝不能在这里顺带触发签到（Qoder 的签到是一次写操作，走 `qoder_checkin_claim`）。
4. 若上游能给签到积分（`dailyCredit` / `streakDays` / `checkinDates`），**原样透传** ——
   抽屉的「签到」页已经在用这些字段（`CheckinStatusEntry` 的注释写了来源）。

---

## 3. 后端实现要点

1. **数据来源**
   - Trae Token：**只有网关日志**（`api_gateway_logs.json`）—— Trae 没有可解析的客户端会话落盘
     （见 `trae::token_stats` 模块头）⇒ `clientSource: "unavailable"`；
     Qoder Token：**不实现**（见 §5「明确不做」）。
2. **时区**：`date` 必须是**本地日期键**，与签到日志 / 积分统计事件同口径（WB 已如此）。
3. **裁剪标志**：网关日志按滚动窗口保留，今天的部分可能已被裁 ⇒ 必须回 `gatewayTruncated`。
   判据取「文件已到 `logKeep` 上限」（保守：只在真的可能被裁时报警）。
4. **性能**：批量签到状态一次调用返回全部账号（前端**禁止**逐账号循环调，见 `AccountsPage.tsx:143` 的注释）；
   单账号刷新时前端传长度为 1 的列表即可，不新增「单账号」变体。
   后端并发上限 4（串行问 50 个账号要十几秒，全并发又会把 50 个请求一次性打到上游）。
5. **幂等**：两条命令都是只读，可安全重试；不要因为 `force=true` 触发任何写操作。
   `force` 在 Trae 侧**恒等**（读的是本地文件，无 TTL 缓存）—— 参数仅为契约保留。
6. **进页面时不要全量拉签到状态**（那要逐账号打上游）。账号视图自带的 `checkedToday`
   （本地台账）足够画初值；用户点「刷新签到状态」才去问上游，那一次是真值。

---

## 4. 前端接线（已落地）

| 文件 | 改动 |
|:--|:--|
| `src/lib/api.ts` | `ROUTES` 加 3 条（`get_trae_account_token_today` / `get_trae_checkin_status_map` / `qoder_checkin_status_map`）、`call()` 封装、`DEMO_READ_COMMANDS` 登记 |
| `src/lib/types.ts` | `AccountTokenTodayResponse` 加 `clientSource`（边界 5） |
| `src/pages/TraeAccountsPage.tsx` | `TRAE_REFRESH_KINDS` 加 `"tokens"` / `"checkin"`；`onRefreshPart` 两条分支接新命令；抽屉指标块从 3 块 → **4 块**（多一块「今日 Token」，副文本写明「仅经本网关的调用」） |
| `src/pages/QoderAccountsPage.tsx` | `QODER_REFRESH_KINDS` 加 `"checkin"`（**不加** `tokens`）；抽屉签到块接上「读不到 / 本区域未开放」两档 |
| `scripts/check-api-contract.cjs` | 三条命令登记进 `SCOPE_PARAMETERIZED`（防「参数化只做一半」） |

> ⚠️ `check:api` 是**硬门禁**：`ROUTES` / `call` / 后端 `server` 三张表任缺一条都会红 ——
> 本次三处（前端 / server / invoke）在同一次提交里补齐。

---

## 5. 明确不做

- **Qoder 的「今日 Token」**：Qoder 的计量单位是 **credits**，不是 token；`qoder_usage` 返回的是逐日 credits。
  除非上游提供 token 级用量，否则这条命令**不实现**，前端菜单里 Qoder 也不出现该项
  （摆一个点了只重读缓存的菜单项比不摆更坏）。
- **不新增「单账号」变体**：批量接口传 1 个 id 即可，避免两套口径。
- **不在读接口里顺手写缓存文件**：刷新是「重读」，不是「续期」；续期走 `trae_refresh_jwt` / `qoder_refresh_account`。

---

## 6. 验收（前端会加这两条断言）

1. **缺席 ≠ 0**：mock 里让某账号不出现在 `accounts` ⇒ 卡片与抽屉显示「—」，不是 `0`。
2. **签到未知 ≠ 未签到**：`status-map` 缺该 uid ⇒ 显示「—」；只有 `todayCheckedIn: false` 才显示「未签到」。

> 两条都对应本仓已有的老问题（`AccountTokenTodayResponse` 的注释里写了四类边界），
> 新接口照抄形状即可自动满足。
