# P14 任务分解：会话跨客户端 + 关联表升级（**取证版**）

> **状态**：设计（2026-10-13）。上游 = `P13-REFERENCE-PARITY.md` §4.2（本文件是它的**落地拆分**，不重复其论证）。
> **本文件的增量**：P13 §7.2 点名「开工前必须先读」的 4 个大文件**已读**，本节把结论钉成事实；
> 并据此**纠正 D37 的一处假设**（见 §0.2）。
> **格式**：沿用 `P5-P12-TASKS.md`（目标 / 涉及文件 / 依赖 / 验收项 / 风险点 / 回滚）。
> **口径**：以当前工作区代码为准。所有「参考实现」的证据都带 `文件:行号`，可复查。

---

## 0. 取证结论（P13 §7.2 第 1 件事的收口）

### 0.1 已确认的事实

| # | 事实 | 证据 | 对本仓的含义 |
|:--:|:---|:---|:---|
| F1 | 关联表**显式持久化成员列表** | `session_link.rs:244-252` `LinkGroup { id, variant, created_at, members, pair_bases }` | 参考**没有「账本」这一层**；成员表本身就是一个事实源 |
| F2 | 成员有**三态**（不是「在/不在」两态） | `session_link.rs:257-264` `MemberState::{Active, Stale, Superseded}`；`:266-269` 稳定字符串有单测守住 | 「失效但保留记录、不自动复活」是**独立语义**，不是布尔位 |
| F3 | 基线是**逐成员对**的，不是组级一个 | `session_link.rs:319-323` `PairBase { member_ids: [String;2], baseline_ref, normalization_version }`；测试 `:2237 pair_bases_are_per_member_pair_and_updates_are_directed` | 3 成员组 = 3 对基线；「相对上次一致点」是**每条边**各有各的 |
| F4 | 基线本体存**摘要向量** | `session_link.rs:328-336` `BaselineRecord { record_count, total_digest, line_digests: Vec<String> }` | 记录级摘要 + 总摘要，**全量**（不是前 N 条截断） |
| F5 | 乐观令牌是**落文件的凭据**，不是内存随机数 | `session_link.rs:1432 save_preview_token` / `:1451 load_preview_token` / `:1462 prune_preview_tokens`；测试 `:3152 preview_token_round_trips_and_rejects_foreign_ids` | 令牌可跨进程存活；`prune` 保留 N 份、且**不误删非令牌文件**（测试 `:3186-3195`） |
| F6 | 令牌绑定**组结构指纹** | `session_link.rs:1330-1343` `PreviewBinding { group_id, group_fingerprint, source, target, baseline_ref, baseline_total_digest, baseline_record_count, verdict }`；`:1369-1372` 注释：「组内任何成员替换或基线变化都会改变指纹」 | 「令牌失效」的判据 = **组结构变了**，不是「文件 mtime 变了」 |
| F7 | 令牌绑定**具体的一对 source→target** | 同上 `source: PreviewMemberBinding` / `target: PreviewMemberBinding` | 一次令牌只授权一条边 ⇒ 批量提交要么逐边签发、要么换设计 |
| F8 | 内容身份用 **sha2**，全量 | `crates/wb-switch-core/Cargo.toml:19`（注释：「不能改用 `DefaultHasher`：跨进程与跨版本都不稳定」） | 与 F4 同源：**跨进程稳定**是硬需求 |
| F9 | 插件会话是**记录级 JSON**，不是行式 | `vscode_session.rs:16-19`：工作区 `index.json {conversations, current}` → 会话目录 `index.json {messages, requests}` + `messages/<32 位小写 hex>.json`；`:827` 文件名只认 32 位 hex | 本仓 `append_delta_between_files`（按行 append）**完全不可复用** |
| F10 | 写回是**整体重写 + 备份回退** | `vscode_session.rs:732`「进入本次操作前的 `index.json` 原始字节，用于回退」；`:775-781` 用 `atomic_write` 写回；`:1104-1117`「重映射后的 `index.json` + `messages/*` + 其余文件原样复制」 | 与「按行 append」是两种写模型 |
| F11 | 运行态**硬拒绝**写会话 | `vscode_session_sync.rs:790`、`codebuddy_ide_session_sync.rs:74` `flavor.is_running()` → `Err("检测到 … 正在运行，请先完全退出后再同步会话，否则写入会被 … 覆盖")` | 拒绝理由是「会被客户端覆盖」，不是「文件占用」 |
| F12 | 三套命名空间 = 三份文件，测试断言不串 | `session.rs:152-154`；`codebuddy_ide_session_sync.rs:315,321-322`（断言另两份文件**不存在**） | 与 P13 T03 要求一致 |
| F13 | 备份用 **SQLite 在线备份 API** | `crates/wb-switch-core/Cargo.toml:16` `rusqlite = { version = "0.32", features = ["bundled", "backup"] }`；`session_backup.rs` 另有 `BackupState::{Allocating, Protected, CleanupPending}`（`:44-66`）与生命周期记录（`:31,308,518`） | P13 B3 的判断成立；但参考的面**远大于**「在线备份」——它有完整的**事务生命周期** |
| F14 | 悬浮窗标题**明文进 DOM，无模糊/占位** | `SessionCard.tsx` 的 `desktop-preview-text` 直接渲染 `{text}` + `title={text}`；`vendor/agent-companion/src/desktop/components/*.tsx` grep `blur\|masked` **零命中** | 与本仓 P9 红线**互斥**；D40 若选明文，须显式改门禁（见 §3 R3） |

### 0.2 ★ 纠正 D37 的一处假设

P13 的 **D37** 写：

> 「引入，但表里只存『基线 + 预览令牌』，**成员仍由 copy ledger 派生**」——理由是「参考怎么解**未取证**，不赌」。

**现在取证了（F1）：参考的关联表显式存 `members`，参考根本没有账本层。**
⇒ 「账本存成员 + 关联表存基线/令牌」**是本仓自己的设计**，不是「对齐参考」。
**这条仍然可以选**（它保住本仓 `session-replicas.ts:71,214` 的「ledger 是唯一事实源、`partial` 恒 true」不变量，从结构上消灭双事实源），
但**必须如实标注为本仓自主决策**，不能再以「对齐参考」为理由。

**建议**：维持 D37 的方案（账本派生成员），**但把理由改写为**：
「本仓已有 `partial: true` 的诚实标注机制，引入第二份成员表会让『组说 3 个成员、账本只有 2 条边』成为可能；
基线/令牌是**只增不改**的附属信息，不承担成员归属。」

---

## 1. 交付切片（4 个，各自可独立上线 / 独立回滚）

> **顺序是硬的**：S1 → S2 → S3 → S4。理由：S1 定契约，S2 依赖 S1 的指纹，S3 独立但与 S2 共用备份面，S4 依赖 S2 的基线语义。

| 切片 | 目标一句话 | 成本 | 风险 | 前置 |
|:--:|:---|:--:|:--:|:---|
| **S1** | **乐观令牌**：提交前复核「你看到的组」还是「现在的组」 | M | 低 | 无 |
| **S2** | **基线对**：把「增量」从「相对对方当前内容」改成「相对上次一致点」 | M | **中**（动复制写入路径） | S1 |
| **S3** | **会话级备份 / 恢复面**：SQLite 在线备份 API + 列出 / 还原 | M | 低 | 无 |
| **S4** | **多客户端会话命名空间**：VS Code 插件 → CodeBuddy IDE（记录级 diff） | **XL** | **高** | S2 |

---

## 2. 逐切片

### S1 · 乐观令牌（`previewToken`）

**目标**：`list_session_replicas` 为每个组签发一个**组结构指纹**；`sync` / `unify` 提交时必须带回它；
指纹不匹配 ⇒ **结构化拒绝，一个字节都不写**，前端把主按钮置为失效态并提示「已被其他操作修改，请刷新」。

**设计要点（借 F6/F7，不照抄 F5 的落文件形态）**

- 指纹内容 = 组内成员 `(uid, sessionId)` 的有序集合（S2 落地后追加 `pair_bases` 的 `baseline_ref` 集合）。
  **顺序无关**（对齐 `:1369-1372` 的注释）；用**稳定 hash**，不用 `DefaultHasher`（F8 的教训）。
- **不落文件**：本仓令牌可以随 `list_session_replicas` 的响应返回、由前端原样带回。
  ⚠️ 参考之所以落文件（F5），是因为它的令牌要跨「预览 → 二次确认弹窗 → 提交」三段、且要防前端伪造。
  本仓的提交是**同一进程内的连续动作**，且后端**本来就会独立复核内容**（`append_delta_between_files` 的
  `common < shorter` 拒绝）⇒ 落文件是**过度设计**。**这条差异要在文件头注释里写明**。
- **粒度**：按**组**签发（不是按边）。本仓的同步任务是 `{sourceCid, targetCid}` 列表，一个组的任务是**一起**算出来的
  ⇒ 组级令牌已经能挡住「你预览后组变了」。⚠️ 若将来要支持「同一组跨多次提交」，再按边拆。

**涉及文件（★ 2026-10-13 已落地，见下方「实现回写」——下表已按**实际**改法修正）**

| 文件 | 改什么 |
|:---|:---|
| `crates/buddy-switch-core/src/modules/session.rs` | 新增 `fnv1a`（从 `hash_line` 抽出，行为不变）+ `pub fn group_fingerprint(&[(uid,cid)]) -> String`；把 `replica_groups_from_ledger` 拆出 `replica_group_members_from_ledger` 作**共同底座**，组 JSON 增 `previewToken`；新增 `GroupGuard` / `group_guard_index` / `item_has_preview_token` / `item_group_key` / `verify_preview_token`；`sync_session_replicas_for` / `unify_session_replicas_for` 前置复核，逐条结果**回显 `groupKey`** |
| `src-tauri/src/commands.rs` | **一行都不用改** —— `items: Vec<Value>` 是透传，令牌随 item 走 |
| `crates/buddy-switch-server/src/api.rs` | 同上，**一行都不用改** |
| `scripts/check-api-contract.cjs` | **不用改**（命令面签名未变；已跑，通过） |
| `src/lib/types.ts` | `SessionReplicaGroup` 增 `previewToken: string`；`Sync/UnifySessionReplicaItem` 增 `groupKey` + `previewToken`；两个 Result 增 `groupKey?: string \| null` |
| `src/lib/api.ts` | **不用改**（items 透传） |
| `src/lib/session-replicas.ts` | `SessionGroupView` 增 `previewToken: string \| null`；`buildGroupViews` 回填（过 `displayText`）；`buildSyncItems` / `buildUnifyItems` 挂 `groupKey` + 令牌，**缺令牌 ⇒ 返回空数组**；新增 `failedGroupKeys` |
| `src/stores/sessions.ts` | **不用改**（透传） |
| `src/pages/SessionsPage.tsx` | 提交时带令牌；**重试改为按组重算**（`retrySync` / `retryUnify`） |
| `src/lib/screenshot-demo.ts` | 演示夹具补 `previewToken`（**照算法真算**，见「实现回写」§5） |
| `src/locales/domains/sessions.zh.ts` / `.en.ts` | 各新增 2 条：`sync.toast.nothingToRetry` / `unify.toast.nothingToRetry` |

**验收项（必须可证伪）**

1. 删掉服务端令牌复核 ⇒ 单测「令牌不符被拒」**变红**；
2. 令牌不符时目标 jsonl **逐字节未变**（不是「返回了 Err」就算数——要断言字节）；
3. 成员顺序打乱后令牌**相同**（守住「顺序无关」）；
4. **出给前端的令牌与写入口的指纹同源**（防「两边各算一份 ⇒ 令牌永远过不了」）；
5. 未携带令牌 ⇒ 退化为旧行为（守住回滚口径）。

**风险点**

- ⚠️ **`replica_groups_from_ledger` 是纯函数、无 IO**。指纹计算若引入 IO 会破坏它的可测性
  ⇒ **指纹只依赖已经在手的账本数据**（成员集合），**不读 jsonl**。已守。
- ⚠️ **两宿主同步**：令牌走 item 透传 ⇒ 本次两宿主零改动，但**将来若改 item 结构，两边仍要一起看**。

**回滚**：令牌是**可选字段** ⇒ 前端不发即退化为今天的行为（有单测钉住这条）；删掉前端传参即完全回滚。

**实现回写（2026-10-13 落地，实际做法与上表的差异 + 证据）**

| # | 计划里没有、实现时必须加的 | 为什么 |
|:--:|:---|:---|
| 1 | 令牌**按条携带**（`groupKey` + `previewToken` 放在每条 item 上），不是命令参数 | 批量可以**跨组**（页头「全部同步」），一个参数装不下；按条携带天然贴合「部分失败」模型；**且两宿主命令面零改动** |
| 2 | 额外校验**两个 cid 属于同一组** | 令牌只证明「组没变」，不证明「你这条任务属于这个组」——而这层校验**今天完全不存在** |
| 3 | 结果**回显 `groupKey`** ⇒ 前端重试改为**按组重算** | 实现时才暴露的坑：复用旧 items 会把**旧令牌**再送一遍，而令牌失效恰恰意味着组变了 ⇒ **重试永远失败**。重算才正确 |
| 4 | 缺令牌时 `buildSyncItems` / `buildUnifyItems` **返回空数组**（不给出口），而不是传 `null` 给后端 | 与既有契约一致（返回空数组 = 不渲染按钮，§3.4 规则 1）；也避免送一次**没有令牌**的写操作 |
| 5 | 演示夹具的令牌**照算法真算**，并与 Rust 侧一条「算法冻结」测试成对钉住 | 本仓红线「演示数据不许自造结构」。若随手编 hex，演示站与真机就成两套形状 |

**验证证据（可复查）**

- `cargo test -p buddy-switch-core --lib` ⇒ **1117 passed / 0 failed / 2 ignored**；
- 新增 7 条护栏（`fingerprint_tests`）：`group_fingerprint_contract`、`group_fingerprint_values_are_frozen_for_demo_fixtures`、`group_token_matches_the_writer_side_fingerprint`、`stale_preview_token_is_rejected_without_writing_a_byte`、`stale_preview_token_is_rejected_before_any_backup`、`missing_preview_token_falls_back_to_previous_behavior`、`unknown_group_key_is_rejected`；
- **变异验证**：把 `verify_preview_token` 首行插 `return Ok(())` ⇒ 上述**后三条**（陈旧 / 备份前 / 未知组）**精确变红**，其余保持绿；还原后 `sha256` 与基线**逐位一致**（`d0cfc6b8…`），变异标记 0；
- `npx tsc --noEmit` 干净；`check:api` 通过（routes=166）；`check:scope` 288 passed / 0 failed；`check:store-selectors` 通过。

⚠️ **未做**：`docs/prototype/verify-behaviors.cjs` 的原型断言（P13 §6 要求「先补原型再写代码」）。
本次 S1 的用户可见行为是「后端拒绝 + 重试重算」，**原型（静态 HTML 演示）不模拟后端拒绝**，
补一条断言需要先在 `demo-app.js` 里造出「令牌失效」这个交互 —— 属**另一件事**，未混进本批。
另：该脚本在本机**默认跑不起来**（`Cannot find module 'jsdom'`，既有环境缺口，与本批改动无关）。
隔离装一份即可跑：`npm i --no-package-lock --no-save jsdom`（在仓库外的临时目录）+
`NODE_PATH=<那个目录>/node_modules node docs/prototype/verify-behaviors.cjs` ⇒ 本批实测 **112 passed / 0 failed**（基线未被破坏）。

---

### S2 · 基线对（`pair_bases`）

**目标**：复制成功时写一条**基线**（该成员对的「上次一致点」）；增量语义从「相对对方当前内容」变成「相对基线」。

**设计要点（借 F3/F4）**

- 存储：新增 `session-baselines/{region}.json`，**只增不改**的附属信息（**不含成员归属**，见 §0.2）。
  结构对齐 `BaselineRecord`（F4）：`{ pair: [memberA, memberB], createdAt, recordCount, totalDigest, lineDigests }`。
- **写入时机**：复制成功（`run_copy_batch` 的 phaseB 落库之后）、统一（unify）成功之后。
  ⚠️ **只在这两处**——不要在「同步」成功时写（同步不产生新的「一致点」，见下）。
- **读取用途**：`deriveGroupStatus` 增加一路「相对基线」的判据；
  ⚠️ **`behind` / `diverge` 的既有判据不动**（D36）——基线只用于**回答「上次一致之后各写了多少」**，
  不用于改判组状态。
- **摘要算法**：本仓是 FNV-1a + 前 512 行（`session.rs:502` `PREFIX_HASH_LIMIT`）。
  ⚠️ **不要为对齐 F8 换成 sha2**：本仓的 hash **只在进程内比较、不持久化为「内容身份」**
  ⇒ 参考换 sha2 的理由（跨进程稳定）在本仓**不成立**。若 S2 的基线摘要要落盘，则**落盘的那一份**必须是稳定 hash。

**涉及文件**：`session.rs`（基线读写 + 在 `run_copy_batch` / `unify_replica_between_files` 成功后写基线）、
`src/lib/session-replicas.ts`（基线 → 展示用增量）、`SessionsPage.tsx` / `session-replica-view.tsx`（展示）、词条 ×2。

**验收项**

1. **旧账本无需迁移**：基线文件缺失 ⇒ 行为**逐字等于今天**（有单测）；
2. 复制成功后基线**恰好一条**、成员对**有序**（`[min, max]` 规范化）；
3. 基线损坏 ⇒ **fail-closed**（当作无基线，**不臆造**），且**不阻断**复制本身；
4. 「上次一致之后的各自新增」在**内容级**可复核（构造两副本 + 一条基线 ⇒ 断言两侧 delta）。

**风险点**

- ⚠️ **动的是 `run_copy_batch` 的三段管线**（`session.rs:1506`）——它有**四条变异验证过的护栏**。
  改前先读函数头 + 跑 `replica_groups_*` / `copy_*` 单测；**只加「成功后写基线」一处，不改 phase 结构**。
- ⚠️ **不要引入第二份成员表**（§0.2）。

**回滚**：删基线文件即回退（缺省无基线 = 今天的行为）。

**实现回写（2026-10-13 落地，实际做法与上面的差异 + 证据）**

| # | 计划 / 实际 | 为什么 |
|:--:|:---|:---|
| 1 | 存储 = `<数据目录>/buddy-switch-session-baselines.json`（region 由**目录**分家），**不是** `session-baselines/{region}.json` | 与 [`COPY_LEDGER_FILE`] 同一套机制；且与账本**分两个文件** —— 混在一起会让「清基线」变成「清去重记录」（不可逆） |
| 2 | 基线键 = **有序 cid 对**（`cid\u{1e}cid`），**不是** `uid␟cid` 对 | 建立基线时手边只有 cid（`unify` 路径拿不到 uid）；cid 是 UUID ⇒ 同 region 内不会撞。⚠️ 顶点键是给**组连通**用的，键空间不同，**不许互相套用** |
| 3 | 落盘的是**本仓自己的 FNV-1a 逐行 hash**，**没有**引入 `sha2` | 参考换 `sha2` 是因为它用了 `DefaultHasher`（跨版本不稳）。**FNV-1a 是纯函数、跨进程跨版本稳定** ⇒ 落盘安全，不引新依赖 |
| 4 | **没有**把基线折进 `group_fingerprint`（乐观令牌） | 复制会**同时**改变成员（令牌已失效）；统一后内容本就一致，陈旧计划的危害极低。把「内容相邻」的状态混进「组结构指纹」会让令牌的契约变模糊。**代价如实记**：统一之后旧令牌仍有效 |
| 5 | 指纹从**刚落盘的目标副本**读，**不动 phase A** | phase A 有「并行复制一个都不落」的变异护栏，为省一次「刚写过、必然在页缓存里」的读去改它不划算（`fingerprint_of_text` 已备好，将来真成瓶颈再搬） |
| 6 | 消费方 = 关联图抽屉里的「**上次一致 N 条**」+ 每个副本的「自上次一致 +M / **已改写更早内容**」 | 这是基线**纠正既有读数**的地方：`deltas` 相对**当前**公共前缀算 ⇒ 有人改写第 50 行时公共前缀会塌，「A 新增 54 条」凭空放大（明明只追加了 3 条）。基线给诚实的 3；**前缀验证不过就什么都不说**（`null`），不编数 |
| 7 | 演示夹具 `baselines` 留**空** | 演示站里没发生过任何写操作；编一条假基线会让「上次一致点」显示假读数（§3.5） |

**验证证据（可复查）**

- 新增 5 条护栏：`baseline_key_is_direction_agnostic`、`group_baselines_are_scoped_to_the_group`、
  `corrupt_baseline_file_is_treated_as_no_baseline`、`unify_records_a_baseline_only_when_it_really_overwrote`（在 `fingerprint_tests`）、
  `copy_records_a_baseline_for_each_copied_pair`（在 `tests`，因为它要用 `seed_batch_copy_home`）。
- ★ **变异 3/3 精确 RED**（三条定向变异各命中一条护栏，其余保持绿）：
  ① phase B 不记基线 ⇒ `copy_records_a_baseline_*` 红；
  ② 「被拒也记基线」⇒ `unify_records_a_baseline_*` 红；
  ③ 基线只判**一端**在组内 ⇒ `group_baselines_are_scoped_*` 红。
  还原后 `sha256` 与基线**逐位一致**（`ce042cb8…`），全仓变异标记 0。
- `unify_records_*` 还钉了第三条语义：**「本来就一致」不刷新一致点** ——
  否则会把更早的、更有信息量的一致点覆盖掉。

---

### S3 · 会话级备份 / 恢复面

**目标**：`rusqlite` 开 `backup` feature；用 **SQLite 在线备份 API**（不是 `cp`）生成一致快照；提供「列出 / 还原」入口。

**设计要点（借 F13，**不**照抄其生命周期）**

- 参考有完整的**事务生命周期**（`Allocating / Protected / CleanupPending` + 生命周期记录文件）。
  ⚠️ 那是为「跨多个副本的批量同步中途崩溃可恢复」服务的；**本仓 S3 不做这个**——
  只做「用户主动备份 / 还原」，**用不上三态**。这条差异要写进文件头。
- 备份目录与保留策略**复用本仓既有的 `backup.rs`**，不新造一套。

**涉及文件**：`crates/buddy-switch-core/Cargo.toml`（`rusqlite` 加 `backup`）、
新增 `crates/buddy-switch-core/src/modules/session_backup.rs`、两宿主命令面、设置页入口、词条 ×2、门禁。

**验收项**

1. **库处于写入中时备份 ⇒ 备份可过 `PRAGMA integrity_check`**（用 `cp` 实现会红——这是本条的证伪点）；
2. 「备份失败 ⇒ 绝不覆盖目标」有单测；
3. 还原前**先备份当前库**（与 `session.rs:764` 的既有纪律同源）。

**风险点**：⚠️ 新增 `rusqlite` feature 会**重编整条依赖链** ⇒ 首次构建变慢；确认 CI 缓存策略能吃住。

**回滚**：删模块 + 摘 feature（还原面是新增命令，前端不入口即不可达）。

**实现回写（2026-10-13 落地 —— ★ 本期**只做一致性修复**，还原面拆到 S3b）**

**为什么砍掉「列出 / 还原」入口**：还原 `workbuddy.db` 是**破坏性**操作（直接覆盖用户客户端的数据），
需要它自己的一套安全设计（客户端必须已关闭、覆盖前先备份当前库、显式二次确认）。
把它塞进本轮会让「一个可验证的正确性修复」和「一个高风险的新破坏性命令」捆在一起交付 ——
按 `PLAN §2.5`（破坏性操作要有后端支撑）与 P13 §7 的纪律，**拆开**更稳。

**实际做的（改在 `session.rs`，**没有**新建模块）**：

| # | 项 | 说明 |
|:--:|:---|:---|
| 1 | `Cargo.toml`：`rusqlite` 加 `backup` feature | 与参考一致（F13） |
| 2 | `backup_db_to` 从 `fs::copy` 三件套 → **在线备份 API** | 这是本期的核心 |
| 3 | 新增 `verify_sqlite_snapshot`（`PRAGMA integrity_check`） | **不信「API 返回 Ok」** |
| 4 | 新增 `snapshot_sidecars` | 清理旧版残留的 `-wal`/`-shm` |

**三个实现时才暴露的事实（都很关键）**

1. ★ **WAL 是持久化在库头里的属性** —— 从 WAL 源库备份出来的库**也是 WAL** ⇒ 旁边照样生成
   `-wal`/`-shm`，用户只拿走 `workbuddy.db` **又丢数据**。⇒ 备份完成后必须在目标库上
   `PRAGMA journal_mode=DELETE`（切回 rollback journal 会把 WAL 并回主库并删掉边车），
   快照才是真正的**单文件**。**不做这一步，在线备份 API 只解决了一半问题。**
2. **rusqlite 不暴露 `Backup::finish()`** —— 收尾在 `Backup` 的 `Drop` 里（`sqlite3_backup_finish`）
   ⇒ 必须让它**出作用域**，不能 `mem::forget`。
3. ★ **Windows 上文件仍被打开时 `remove_file` 会静默失败** ⇒ 「失败清理半截产物」必须
   把连接收在闭包里，**先析构再删**。否则失败时会留下一个看起来可用的假备份。

**★ 一处语义变化（要如实记）**：源库**不是**有效 SQLite 库时，旧实现（`fs::copy`）会「成功地」
拷一份垃圾、报告里 `backup` 字段有值、用户以为有回滚点；新实现**直接失败且不产出任何文件**。
这是「失败不得谎报成功」的加强，不是退化。

**验证证据（可复查）**

- 改造 `backup_failure_is_reported_instead_of_faked`（5 段：无需备份 / 目录建不出 / 目标被目录占住 /
  **非 SQLite 源库** / 阳性对照）+ 新增 `db_backup_is_a_self_contained_snapshot`。
- ★ **变异 2/2 精确 RED**：把 `backup_db_to` 退回 `fs::copy` 三件套 ⇒
  ① `db_backup_is_a_self_contained_snapshot` 红（备份目录里出现 `workbuddy.db-shm` / `-wal`）；
  ② `backup_failure_is_reported_instead_of_faked` 红（非 SQLite 源库被"成功"拷贝）。
  ⚠️ ② 的**报错文案是「备份校验失败」而不是「备份失败」** —— 说明是**新增的自检层**把它兜住的
  （退回旧实现时连自检都会报"file is not a database"）⇒ 那层自检不是摆设。
  还原后 `sha256` 与基线**逐位一致**（`0ee45fbd…`），全仓变异标记 0。

**S3b（未做，下一轮）**：`list_session_backups` / `restore_session_backup` + 两宿主命令面 +
设置页入口。前置条件：**必须先定「客户端运行中怎么处理」**（拒绝？提示先关闭？），
否则一次误还原会毁掉用户正在用的会话库。

---

### S4 · 多客户端会话命名空间（**XL，拆三步**）

> ⚠️ **开工前必须再读**：`vscode_session_sync.rs`（2955 行）的**记录级 diff + 追加**算法。
> 本文件只钉了它的**边界**（F9/F10/F11），**算法本体未读**。

| 步 | 范围 | 验收 |
|:--:|:---|:---|
| **S4a** | VS Code 插件：**只读 + 复制**（不做关联 / 同步） | 能列出插件会话、能复制到另一账号；`index.json` 的 `messages/` 与 `conversations` **两侧都重映射** |
| **S4b** | VS Code 插件：**关联 + 增量同步**（记录级 diff） | 运行态闸门返回结构化错误；跨变体同步被拒且目标**逐字节未变** |
| **S4c** | CodeBuddy IDE（CN/Intl）：复用 S4a/S4b | 与 VS Code **共用实现**、**独立 store 文件** |

**同时必做（A4）**：`SessionsPage` 的客户端 Tab —— **≥2 个客户端才渲染**；1 个时不渲染也不保留结构（`PLAN §3.15.1`）。

**风险点**

- ⚠️ **成本 XL**：`vscode_session.rs` 2026 行 + `vscode_session_sync.rs` 2955 行 + `codebuddy_ide_session*.rs` 1546 行。
- ⚠️ **写模型不同**（F10）：本仓的「按行 append」在这里**完全不可用**；要新写「读 → 改 → 整体重写 + 回退备份」。
- ⚠️ **运行态闸门会改用户体验**（F11）：本仓既有客户端切换是「可自动关闭重开」，而会话同步是**硬拒绝**
  ⇒ 文案必须如实说明「请先完全退出」。

**回滚**：新命名空间是**新增文件**；Tab 由客户端数驱动 ⇒ 回退到 1 个客户端时自动不渲染。

**实现回写（2026-10-13 · 只落了「读半」，写半刻意留到下一步）**

**为什么分两步**：复制要**动用户客户端的真实数据**，而 id 重映射（消息 id / 请求 id /
`extra` 里**字符串化的 JSON**）的算法我**还没读完**。⇒ 先把「安全边界 + 只读枚举」做实并钉住，
写路径等读完再动 —— **不半懂就写别人的数据**。

**已落地**：新增 `crates/buddy-switch-core/src/modules/vscode_session.rs`（只读）+ `mod.rs` 注册。

| 内容 | 说明 |
|:---|:---|
| 布局常量与路径 | `<数据根>/CodeBuddyExtension/Data/<uid>/<客户端段>/<uid>/history/<工作区目录名>`；`ExtClient::{Vscode, CodebuddyIde}` 只差客户端段 |
| `data_root_candidates` | **走 `home_dir()`**（不是 `dirs::data_local_dir()`）⇒ 单测能靠 `BUDDY_SWITCH_HOME` **完全不碰真实 home**。⚠️ 这是与参考的**有意差异** |
| `is_safe_uid` / `is_hex32` | 安全边界：脏 uid 不进文件系统；只对认得的 32 位小写 hex 动手 |
| `list_sessions_in` / `list_sessions_for` | 只读枚举：按工作区分桶，返回 `{sourceUid, sessions[], skipped, dataRoot}` |
| `conversation_has_history` | **索引与磁盘都看**（只看一条会把有正文的误报成空会话） |
| `time_to_ms` | RFC3339 / 秒 / 毫秒三形态；认不出 ⇒ 0（**不臆造**时刻） |

**三条 fail-closed 口径**（都写进模块文档）

1. 数据根找不到 ⇒ `dataRoot: null`（前端据此区分「**没装**」与「装了但没会话」——压成一个值就是撒谎）；
2. 工作区索引损坏 / 结构不符 ⇒ 计入 `skipped` **并继续**（如实报数，不用「少列几条」把问题吞掉）；
3. 脏 uid ⇒ 空列表，**不进文件系统**。

**★ 一条必须记住的未知**：布局口径来自对同产品另一实现的取证，**本机没有真实客户端数据可验证**
⇒ 出口一律 fail-closed，**绝不臆造**一条会话。写路径落地前，这一条同样适用。

**验证证据**

- 新增 10 条护栏。★ **变异 4/4 精确命中 5 条**（重跑过两轮，第二轮是夹具修正后的当前代码）：
  ① 去掉路径分隔符检查 ⇒ `unsafe_uids_are_rejected` 红；
  ② IDE 客户端段写死成 `VSCode` ⇒ `layout_…` + `listing_reads_both_client_trees_…` 两条红；
  ③ 去掉磁盘 `messages/` 分支 ⇒ `has_history_checks_both_…` 红；
  ④ 坏索引静默跳过 ⇒ `corrupt_workspace_index_…` 红。
  ✅ 阳性对照：`dirty_uid_yields_an_empty_listing_…`（uid `..`）与 `missing_data_root_…` **保持绿**
  ⇒ 说明变异是**定向**的。还原后 `sha256` 与基线**逐位一致**（`db096365…`），全仓变异标记 0。

- ⚠️ **踩到并修掉一个夹具坑（值得记）**：第一版夹具让**每个**用例都取
  `HomeOverrideGuard`（进程级独占锁）⇒ 6 个用例互相排队，配上变异导致的 panic，
  跑出「两条挂住 60s+ / 一条莫名失败」的**假信号**。
  **正解**：本模块的判据函数全是 `_in(root, …)` 形态、**显式吃路径** ⇒ 大多数用例
  **根本不需要 home 覆盖**；只有真要调 `data_root()` / `data_root_candidates()` 的两条才取锁。
  改完 10 条用例 **1.3s 跑完**（原先是串行 + 饥饿）。
  ⇒ **本仓「单测不该依赖 `BUDDY_SWITCH_HOME`」这条纪律，就是为这个坑写的。**

**下一步（S4a 写半 —— ★ 已于同日落地，见下方「写半」；下面这份清单已完成）**

1. 读完参考的 **id 重映射算法**（`RemapPlan::build` / 递归精确匹配 / `extra` 里字符串化 JSON 的处理）；
2. 复制：`RemapPlan` + 写 `.tmp-<newId>/` → `rename` 原子提交 → 合并目标工作区索引（写前备份）；
3. **加法**：目标已有同 id ⇒ 取新 id，**绝不覆盖**；失败逐条隔离 + 回退。
4. 再之后：两宿主命令面 + UI（切换弹窗 / 会话页客户端 Tab）。

### 写半（2026-10-13 同日落地）

把参考的 id 重映射算法**读完**后才动手。落地内容（同样只改 `vscode_session.rs`）：

| 内容 | 说明 |
|:---|:---|
| `RemapPlan` | 老 id → 新 id 三张表（消息 / 请求 / 合并视图）。消息 id **两个来源都要收**：索引 `messages[].id` + 磁盘 `messages/<hex32>.json` 文件名 |
| `replace_exact_ids` | ★ **精确等于**才换，**不是子串替换**。本模块最危险的一处（子串替换会静默毁内容，且改完仍是合法 JSON） |
| `remap_extra` | `extra` 实测是**字符串化的 JSON** ⇒ 必须**解析开**再改、改完字符串化回去；对象形态也兼容；解析不开就**原样返回** |
| `remap_session_index` | 四类引用：`messages[].id` / `requests[].id` / **`requests[].messages[]`**（漏改 ⇒ 扩展显示空会话） |
| `write_conversation` | 重映射后的索引 + 消息文件改名并改内容 + `.index_bak.json` + 其余文件原样复制；**非 hex 文件按字节原样**（不重排格式） |
| `merge_workspace_index` | 追加条目并**修好 `current`** —— 缺 `current` 的索引会被扩展判为**损坏**（改名 `index.json.corrupted.<ms>` 并重建，多出一条垃圾会话） |
| `copy_conversations_in` | 按工作区聚合；`.tmp-<id>` → `rename` 原子提交；索引最后**一次**写回（写前备份 + 复核字节数）；索引写失败 ⇒ **删掉本次新建目录**回退 |

**顺带修掉一个共享工具的真缺陷**：`config::atomic_write` 在 `rename` 失败时**不清理临时文件**
（目标被目录占住时实测会留下 `index.json.tmp-<uuid>`）。在**用户客户端的数据目录**里留垃圾
比写失败更坏 —— 已加清理。

**验证证据（写半）**

- 新增 6 条护栏。★ **变异 4/4 精确命中 4 条**（**串行**跑，4.42s）：
  W1 改成子串匹配 ⇒ `remap_never_touches_ids_…` 红；W2 不重映射 `requests[].messages[]` ⇒
  `copy_remaps_every_id_reference_…` 红；W3 不回退 ⇒ `failed_index_write_rolls_back_…` 红；
  W4 所有文件都当 hex ⇒ `non_hex_files_in_messages_are_copied_verbatim` 红。
  其余 12 条保持绿。还原后 `sha256` 逐位一致（`34bd8d89…`），全仓变异标记 0。

- ★★ **抓到一条「永远绿」的假护栏（本批最值得记的一条）**：
  `remap_never_touches_ids_…` 初版把子串塞进**消息正文** `message`，结果**在子串变异下照样通过**
  —— 因为 `remap_message_file` 只碰 `id` 与 `extra`，**正文根本不经过 `replace_exact_ids`**
  ⇒ 那条断言是**构造上恒真**的，等于没写。
  **修法**：子串要塞进**真的会被重映射**的字段（`extra.note`）才测得到。
  ⇒ 复述本仓那条纪律：**断言的敌人不是「红」，是「永远绿」**；每条断言都要问
  「把被测逻辑删掉，它会红吗？」—— 这次答案是**不会**。

- ⚠️ **更正一处我先前的误判**：上一轮我把「并行跑时挂住 / 用例莫名失败」归给了
  `HomeOverrideGuard` 那把进程级锁，**归错了**（见下）。收敛取锁本身是对的，但**不是**挂住的原因。

**环境观察（未解决，如实记）**

并行跑**本模块**（`cargo test -p buddy-switch-core --lib -- vscode_session`，16 条）会**挂住**：
多条用例停在 "running for over 60 seconds"，整轮 >400s 被 `timeout` 杀掉；
而**全量**并行跑（1100+ 条）与本模块**串行**跑都正常且快（16 条 6.7s）。
**当前对策**：本模块的变异验证一律加 `--test-threads=1`。**未查清，不假装已修。**

**下一步（S4a 收尾 + S4b/c）**

1. **两宿主命令面**：`list_vscode_sessions` / `copy_vscode_sessions`（Tauri `commands.rs` + server `api.rs`，`check:api` 四方一致）；
2. **UI**：切换弹窗支持插件程序位（现在弹窗硬编码「WorkBuddy 桌面」）、会话页客户端 Tab（**≥2 个客户端才渲染**，A4）；
3. **S4b/S4c**：关联 / 增量同步（记录级 diff）；IDE 侧复用（`KeepUnlessConflict` id 策略）。

### 命令面（2026-10-13 同日落地）

新增两个命令，**四处同步**（`check:api` 已通过：routes=168 / call=156 / server=154 / invoke=174）：

| 命令 | 路由 | 说明 |
|:---|:---|:---|
| `list_vscode_sessions` | `GET /api/vscode-sessions` | 只读枚举；`client` + `uid` **都必填** |
| `copy_vscode_sessions` | `POST /api/vscode-sessions/copy` | 加法复制；body `{client, sourceUid, targetUid, items[]}` |

**三条设计决定（都写进注释）**

1. ★ **不往 `list_sessions` 里塞 `client` 参数**，而是**单开命令**：桌面版会话是
   `workbuddy.db` + jsonl，插件侧是 JSON 记录仓 —— **两套形状**挤进一个出口会让每个调用点
   都长出分支。形状各自诚实。
2. ★ **`client` 空串 / 未知值一律 `Err`**（不回落默认值）：本仓不猜「用户指的是哪个客户端」，
   猜错就是去读写**另一个客户端**的会话目录。`uid` 同理**必填**（不猜「插件当前登录的是谁」）。
3. ★ **写路径的数据根缺失 ⇒ `Err`（不是「复制了 0 条」）**：读路径可以用 `dataRoot: null`
   表达「没装」，**写路径不行** —— 用户点了复制就必须知道到底做没做。
   （`copy_conversations` 因此是 `Result`，而 `list_sessions_for` 不是。）

**验证**：`cargo check -p buddy-switch-server` 与 `-p buddy-switch-rust` 都干净；
`tsc` 干净；`check:api` 通过。前端封装 `listExtSessions` / `copyExtSessions` 已就位
（类型在 `types.ts`：`ExtClientId` / `ExtSession` / `ExtSessionsResponse` / `ExtCopyItem` / `ExtCopyResponse`）。

⚠️ **顺带纠正一个我一直记错的**：`src-tauri` 在本沙箱**可以** `cargo check -p buddy-switch-rust`
（包名不是 `buddy-switch`）—— 之前记的「沙箱内不能编 src-tauri」只对 `cargo test`（要链接）成立。

### 原型（2026-10-13 同日落地 · **含一处对我自己先前判断的更正**）

**★ 更正**：我上一轮写的「UI 入口 = 会话页的客户端 Tab（A4）」对**插件侧不成立**。
原因是**实现时才看清的**：会话页的模型是**副本组**（有账本、可双向同步、有「谁落后 / 谁分叉」），
而插件侧的复制是**一次性单向复制**（副本恒取新 id）⇒ **构不成可同步的副本组**。
把它塞进 `SESSION_CLIENTS` 只会让会话页多出一个**永远空的组列表**（假客户端 Tab）。
⇒ 正确入口在**账号页**（对齐参考实现：插件/IDE 的会话复制随账号切换触发）。
`demo-data.js` 里那句「将来有第二个客户端时**零改动**地出现」已就地更正。

**已落地**（`docs/prototype/`：`demo-data.js` / `demo-app.js` / `buddy-switch-prototype.html` / `verify-behaviors.cjs`）

| 内容 | 说明 |
|:---|:---|
| `EXT_SESSIONS` 夹具 | 三种**真实返回形状**：`ok`（含 `skipped > 0`）/ `empty`（装了但账号没会话）/ `missing`（`dataRoot: null`） |
| `extSessionCopyHtml` | 插件会话抽屉：来源账号 + 数据目录 + 勾选列表 + 目标账号 + 主操作 |
| 入口 | 账号详情抽屉的「切换到此账号」区块，**只在账号绑定了插件类程序位时**出现 |
| 三条诚实规则 | ①「未检测到客户端」与「没有会话」**文案与出口都不同**；② `skipped > 0` **如实标注**；③ 目标候选 = 本区域除源账号外的**全部**账号（**不写死列表**） |
| 复制语义写在界面上 | 「副本会**新建**会话（新 id）—— 目标账号原有的会话**一个字都不会被动**」 |

**验证**：`verify-behaviors.cjs` **116 passed / 0 failed**（新增 4 条断言，含**阴性对照**：
未绑定插件程序位的账号上**不得**出现该入口；`skipped = 0` 时**不得**出现标注行）。
★ **变异 1/1 精确命中**：新增 `--mutate=extmissing`（去掉「未检测到客户端」的独立空态）
⇒「两段不同文案」那条**变红**，脚本自报「断言在缺陷下确实变红」。交付目录未被写入（变异只写临时副本）。

⚠️ **写断言时踩到**：抽屉每次重画会**整块替换 DOM** ⇒ 遍历旧 `NodeList` 点复选框只点得动第一个
（本用例初版就这么错的，表现为「取消全选后按钮仍可用」的**假失败**）。
⇒ 循环里**每轮重新查询**节点。

### 组件（2026-10-13 同日落地）

| 文件 | 内容 |
|:---|:---|
| `src/lib/ext-sessions.ts`（新） | **程序位 → 插件会话仓客户端**映射。⚠️ **`jetbrains_codebuddy_plugin` 刻意不映射** —— 会话仓只有 `VSCode` / `CodeBuddyIDE` 两段，JetBrains 没有对应树；硬塞映射 = **去读一棵不属于它的树** |
| `src/components/sessions/ext-session-drawer.tsx`（新） | 抽屉：加载 / **读取失败** / **未检测到客户端** / **没有会话** / 正常 五态；勾选 + 目标账号 + 逐条失败 |
| `src/components/account-detail-drawer.tsx` | 新增 `onExtSessions` / `extClientLabel` 两个 prop（**不注入 ⇒ 不渲染入口**） |
| `src/pages/AccountsPage.tsx` | 独立状态 `extDrawer`（与 `switchAccount` **并列不嵌套**）；目标候选 = 本区域除源账号外的全部账号 |

**三处诚实性**（与原型断言一一对应）

1. **「未检测到客户端」/「没有会话」/「读取失败」是三个不同的态**（原型只有前两个；实现多了「读取失败」——
   那是**网络 / 后端报错**，必须给重试出口，与「后端如实说没装」不是一回事）；
2. `skipped > 0` **如实标注**（`data-ext-skipped`）；
3. 部分失败**不当成功报**：逐条列出失败原因（§3.6）。

**⚠️ 一处与原型的已知差异（未消除，记在这）**：原型用「该账号**绑定了**插件程序位」作入口条件；
真实页面的 `switchTargets` **没有**「本账号是否绑定」这个位，只有「该程序位是否已接入」
⇒ 实现退化为「**该程序位已接入**」。要消除它，得让后端在账号数据里带上「绑定的程序位」。

**验证**：`tsc` 干净；`check:scope` 288/0；`check:store-selectors` 通过；`check:api` 通过（routes=168）；
原型门禁 **116/0**；`vite build` **成功**。全仓变异标记 0。

⚠️ **写组件时踩到**：`AccountMeta` **没有** `name` 字段（那是原型夹具的写法）；
展示名必须走 `displayText(nickname) → email → id` 的降级链 —— `nickname` 在后端是
`$wbEncrypted` 信封，脏值直接渲染会让 React **卸载整棵树**（issue #2）。

### S4c · IDE 侧复用（2026-10-13 同日落地）——**这不是可选优化，是正确性缺口**

**为什么必须做**：`copy_conversations_in` 原先对**两个客户端**都恒取新 id。
但同产品另一实现**真机实测**的结论是「**IDE 沿用源 id 的目标账号可直接加载**」
⇒ 照原状，IDE 的副本**可能根本加载不了**（而且是**静默**的：复制报告会说「成功 1 项」）。
两个客户端之间的语义差异只有两处，都落在 `ExtClient` 上：

| 差异 | VS Code 插件 | CodeBuddy IDE |
|:---|:---|:---|
| **副本 id** | `AlwaysNew`（扩展按 id 归类 ⇒ 必须与源区分） | `KeepUnlessConflict`（沿用源 id；**冲突才**换新 id） |
| **`.index_bak.json`** | **写**（扩展自用的索引备份要与主索引一致） | **不写**（目标侧原有的保持原样，**不存在也不新建**） |

**实现**：`ExtClient::{copy_id_policy, writes_workspace_index_backup}` + `CopyIdPolicy` 枚举；
沿用 id 时走 **`copy_dir_recursive` 字节复制**（不重映射 ⇒ 内容与源**逐字节相同**）；
冲突时仍走**全量重映射** —— 会话 id 都变了，副本内部引用跟着换才自洽（外部对它的引用本来已断）。

**验证**：新增 3 条护栏（共 19 条）。★ **变异 3/3 各自精确命中**：
C1 IDE 改成 `AlwaysNew` ⇒ 沿用 id / 逐字节相同两条红；C2 去掉冲突判定 ⇒ 冲突用例红；
C3 索引备份恒 `true` ⇒ IDE 侧那条红。还原后 `sha256` 逐位一致（`fb4158d4…`），标记 0。

- ★★ **踩到一个变异方法论问题（值得记）**：第一轮**三条一起注入**时，C2 的用例**没有变红** ——
  因为 C1 先把 IDE 策略改成了 `AlwaysNew`，`keep_source_id` 恒为 false，**C2 被掩盖了**。
  ⇒ **打在同一条路径上的变异必须逐条单独跑**；否则「没红」既可能是断言空转、
  也可能是**被别的变异遮住**，两者结论完全不同。
- ⚠️ **写夹具时踩到**：复制只在**同一个客户端的树**里找源
  （`history_root_in(root, client, uid)` 带客户端段）⇒ 测 IDE 侧必须**两棵树都播种**，
  否则「找不到源会话」会静默复制 0 条（本用例初版就这么错的）。

### S4b（**只落了「可行性判定」这半**）· 记录级 diff

**为什么先做判定、而不是直接做同步**：插件侧是**有序数组 + 一堆文件**
（顺序由 `index.json` 的 `messages[]` **数组次序**决定，**不是**文件时间），
所以桌面版那套「把源里多出来的行 append 到目标尾部」**天然保序**的性质**在这里不成立**：
目标若缺的是**中间**那几条，追加到尾部会把会话读成**错乱的顺序** —— 而且是**静默**的
（JSON 依然合法）。⇒ **能不能增量追加本身就是要判定的问题**，判错了比不同步坏得多。

`diff_conversation_in`（**只读**，不写任何文件）的 `status`，**判据顺序即优先级**：

| status | 判据 | 后续动作 |
|:---|:---|:---|
| `error` | 源缺失 / 索引损坏 / 标识非法 | 无可同步 |
| `absent` | 目标**没有**这份会话 | 走**整份复制**（不是增量） |
| `divergent` | 同 id 但**内容不同** | **拒绝**（同步会静默改写目标内容） |
| `unordered` | 目标不是源的**前缀** | **拒绝**（追加会**打乱会话顺序**） |
| `identical` | 逐条一致 | 无事可做（**不报成可同步**） |
| `appendable` | 目标 = 源的开头一段，源只是尾部多几条 | 可安全追加 |

**两条判据的细节（都是踩过才知道要这么写的）**

- **内容身份比的是字节**（FNV-1a over `messages/<id>.json` 原始字节），**不是解析后比较**：
  `extra` 是**字符串化 JSON**，解析 → 重新序列化会**抹平键序 / 空白** ⇒ 把「其实不同」判成「相同」。
  比字节是最保守的：**宁可少同步，不可误覆盖**。
- **消息与请求两条序列都要判前缀**：请求里存着消息 id 列表，只判一条会漏。

**为什么不做「尽力同步」**：`divergent` / `unordered` 两种情况下**任何**自动动作要么改写
目标已有内容、要么打乱顺序，且都是**静默**的 ⇒ 这里**只报告、不动作**，把选择权交回用户
（与桌面版 `diverge` 同一立场）。

**验证**：新增 6 条护栏（共 **25 条**）。★ **变异 3/3 各自精确命中**（三条**打在不同路径上**，
故可同轮注入）：D1 `is_prefix` 恒真 ⇒ 缺中间那条被当成可追加（红）；D2 去掉 `divergent` 判定 ⇒ 红；
D3 `identical` 也算 `appendable` ⇒ 红。还原后 `sha256` 逐位一致（`a63b764c…`），标记 0。

**S4b 剩下的（未做）**：真正的**增量追加**（把源尾部缺的消息文件复制过去 + 补齐目标索引的
`messages[]` / `requests[].messages[]`）+ **关联**（给插件侧建它自己的账本 / 基线模型，
让「这份会话在两个账号里」这个事实可查）。**前置就是本轮的 `appendable`** ——
只有它为真才允许写。

### S4b · 增量追加（2026-10-13 同日落地）

`sync_conversation_in`。三条硬纪律：

1. ★ **不信任调用方**：进来先**自己重算** diff，只有 `appendable` 才动手
   （调用方说「我算过了」不算数 —— 两件事之间源可能又变了）；
2. **只加不改**：目标已有的消息文件一个字节都不碰，索引只替换 `messages` / `requests`
   两条序列（**目标索引里它自己的字段原样保留**）；
3. **索引是提交点**：先补消息文件，**最后**写索引（写前备份）；索引写失败 ⇒
   **删掉本次新拷的消息文件**回退。

**测试抓到的两个真缺陷（都不是测试写错）**

- **「目标缺正文文件」被误判成 `divergent`**：`record_digest` 对缺失文件返回 `None`，
  `None != Some(h)` 就成了「内容不同」。但那其实是**目标损坏**（索引说有、磁盘上没有），
  混成 `divergent` 会给出**错误的处置建议**（用户以为「两边改过、要选一份」）。
  ⇒ 新增 `brokenMessages` 与 `error` 状态。
- **`if to.exists() { continue; }` 把「存在但**不是文件**」当成「已经有了」** ⇒ 索引会声称
  这条消息存在，而正文不可用 —— **静默损坏**。⇒ 改成 `is_file()` 才跳过，否则如实失败。

**验证**：新增 4 条护栏（共 **29 条**）。★ **变异 3/3 各自精确命中**：
S1 去掉 diff 守卫 ⇒ 两条「拒绝」用例红；S2 把 `is_file()` 退回 `exists()` ⇒ 回退用例红；
S3 索引整份换成源的 ⇒ 「保留目标自有字段」红。
还原后 `sha256` 逐位一致（`02c82823…`），标记 0。全量 **1159 passed / 0 failed / 2 ignored**。

- ★★ **又抓到一次「永远绿」**：S3 第一轮**没打红** —— 因为源与目标的夹具 `note` **值相同**，
  「整份换成源的索引」与「只替换两条序列」**区分不出来**。
  **修法**：给**目标**的索引加一个**源没有的**字段（`targetOnly`），断言它存活。
  ⇒ 老规矩：断言要问「**把被测逻辑删掉/换掉，它会红吗？**」；这次第一版答案是**不会**。

### S4b · 命令面（2026-10-13 同日落地）

| 命令 | 路由 | 说明 |
|:---|:---|:---|
| `diff_vscode_sessions` | `GET /api/vscode-sessions/diff` | **只读**；`status` 六态（判据顺序即优先级） |
| `sync_vscode_sessions` | `POST /api/vscode-sessions/sync` | 增量追加；**只有 `appendable` 才真写** |

★ **diff 与 sync 的「数据根缺失」刻意不同**：diff 是**读** ⇒ 回 `status: "error"`
（保持返回形状统一，前端按 `status` 统一渲染）；sync 是**写** ⇒ 回 **`Err`** ——
用户点了「同步」就必须知道到底做没做，回「同步了 0 条」会把「客户端没装」伪装成
一次成功的空操作。**这一条有单测钉着。**

**验证**：`check:api` 通过（routes=170 / call=158 / server=156 / invoke=176）；
`tsc` 干净；`cargo check -p buddy-switch-server -p buddy-switch-rust` 干净；
新增 1 条护栏（共 **30 条**）钉住生产入口的 fail-closed 分支。

**S4b 至此：diff + 增量追加 + 命令面齐了。** 剩 **关联**（给插件侧建它自己的账本 / 基线模型，
让「这份会话在两个账号里」可查）+ UI 里的「同步」入口。

**★ 顺带修掉一个真缺陷（并行跑暴露的）**：备份文件名原先只用会话 id
（`<conversationId>-index.json`），**没带工作区 hash** ⇒ 两个不同工作区里同名会话会
**互相覆盖备份**。**备份被覆盖比没有备份更坏**（用户以为有退路，其实没有）。
⇒ 改成 `<workspaceHash>-<conversationId>-index.json`。

**★★ 环境限制升级（值得全仓知道）**：全量**并行**跑两次，失败**各不相同** ——
第一次 `session::session_paths_are_region_scoped`、第二次 `vscode_session::sync_rolls_back_…`，
而两者**单独跑都通过**；本模块**串行跑 5 次全绿**。
⇒ **本沙箱内 `cargo test -p buddy-switch-core --lib` 的并行结果不可信**（会误报）。
结论一律以 `--test-threads=1` 或单独跑为准。**不要**把并行偶发失败当成真缺陷去改代码。

### S4b · 关联（2026-10-13 同日落地）

**它其实是 diff / sync 可用性的前提**：没有关联，界面就得让用户**手工指名「哪两条是一对」**。
有了它，界面才能直接给出「这份会话在另一个账号里的对应副本」。

**账本**：`<工具存储根>/.buddy-switch/vscode-session-links.json`，
键 = `client ␟ workspaceHash ␟ sourceUid ␟ targetUid ␟ sourceCid`。

- ★ **另起一个文件**，不并进桌面版的 `buddy-switch-copy-ledger.json`：① 两个客户端的会话 id 是
  **两个 id 空间**；② 桌面版账本是 P8 **去重机制**的一部分（`ledger_hit` 决定「复制过就不再复制」）
  —— 往里塞插件侧条目会**静默改变 P8 行为**，且**不可逆**（清掉就恢复不了）。
- ★ **客户端段与工作区段都要进键**：同一个 `conversationId` 在两棵树 / 两个工作区里是
  **不同的会话**，少一段就会把它们**当成同一对**（两条阴性对照钉着）。
- **查询是双向的**：它既可能是源、也可能是目标。
- ⚠️ **新记录覆盖旧记录**：同一对账号再复制会生成**又一份**新会话（恒取新 id），
  保留旧值会让账本一直指向**过时的副本**；指向最新才是可用的语义（代价：旧副本的关联会丢）。

**★ 一个设计决定（测试隔离）**：关联登记放在**生产入口** `copy_conversations`，
**不放进** `copy_conversations_in` —— 后者只碰**客户端数据目录**（测试可完全隔离）；
账本写在**工具自己的存储根**下，塞进去会让「复制客户端数据」这个动作**隐式写工具存储**，
单测就会**污染用户的真实目录**（本仓「单测不该依赖 `BUDDY_SWITCH_HOME`」正是防这个）。
⇒ 关联用例显式取 `HomeOverrideGuard`，并在注释里写明**为什么必须取**。

**验证**：新增 1 条护栏（共 **31 条**，含 4 条阴性对照）。
★ 变异 **2/2 各自精确命中**（L2 需**单独跑** —— L1 会掩盖它，**本会话第三次遇到同路径掩盖**）。
还原后 `sha256` 逐位一致（`0d0ab3a3…`），标记 0。

**S4b 至此：diff + 增量追加 + 命令面 + 关联齐了。** 只剩 **UI 里的「同步」入口**
（diff 六态怎么呈现、`divergent` / `unordered` 怎么把后果讲清楚）+ 关联的**读命令**接线。

---

## 3. 不照抄清单（红线）

| # | 参考的做法 | 为什么不抄 | 本仓该怎么做 |
|:--:|:---|:---|:---|
| **R1** | 关联表**显式存 members**（F1） | 会让「组说 3 个成员、账本只有 2 条边」成为可能，破坏 `partial: true` 的诚实机制 | 账本派生成员；关联表只存基线 + 令牌（**本仓自主决策**，§0.2） |
| **R2** | 令牌**落文件** + `prune` 保留 N 份（F5） | 本仓的提交是同进程连续动作，且后端本来就会独立复核内容 | 随响应返回、前端带回；**不落盘** |
| **R3** | 悬浮窗标题**明文进 DOM**（F14） | 与本仓 P9「标题不进 DOM」门禁互斥 | ⚠️ **用户 2026-10-13 已选「照参考显示明文」** ⇒ 这是**显式推翻**，必须同步改掉 P9 的门禁断言，**不许两处并存**（留给 P15） |
| **R4** | 内容身份用 **sha2 全量**（F8） | 本仓 hash 只在进程内比较，跨进程稳定性不是需求 | 保持 FNV-1a；**落盘的那一份**（S2 基线）才需要稳定 hash |
| **R5** | 备份的**事务三态生命周期**（F13） | 那是为批量同步中途崩溃服务的，S3 只做用户主动备份 | 只采纳「SQLite 在线备份 API」 |
| **R6** | 参考的**模块粒度**（`session.rs` 426 KB 单文件） | 本仓按域拆分 + 单测密度更高（P13 D4） | 保持本仓拆分 |
| **R7** | 参考的 `common / extraA / extraB` 三数判据 | 与本仓「逐行 hash」**不同坐标系**，混用会让同页出现两个「共同旧版」（P13 D3/D36） | 保留本仓词表 `latest/behind/diverge/unknown` |

---

## 4. 门禁与验收

| 用途 | 命令 |
|:---|:---|
| 类型 | `npx tsc --noEmit` |
| API 四方一致 | `npm run check:api` |
| 注册表 / 作用域 | `npm run check:scope` |
| 后端 | `cargo test -p buddy-switch-core --lib` |
| 构建 | `npm run build:fast` |

**每条切片的最低要求**：改回缺陷 ⇒ 护栏**真的红且逐字复现** ⇒ 还原 ⇒ `sha256sum` 一致 + 变异标记 0。

---

## 5. 待拍板

| # | 决策 | 状态 |
|:--:|:---|:---|
| **D37** | 是否引入显式关联表 | ⚠️ **假设已被 F1 纠正** ⇒ 建议维持「账本派生成员」，但**改写理由**（§0.2） |
| **D36** | 判据口径 | 建议维持本仓 hash（P13 原推荐） |
| **D38** | 插件会话 diff 粒度 + 运行态闸门 | **已取证**（F9/F11）：记录级 + 硬拒绝 ⇒ 建议照此拍板 |
| **S3 范围** | 只做「用户主动备份 / 还原」，还是照参考做事务生命周期 | 建议**只做前者** |
