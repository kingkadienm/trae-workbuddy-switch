# 2026-11-14 上游同步与冲突取舍

> 状态：只读历史（不再更新）。记录此次合并时的比较依据与验证边界；后续行为以现行源码和 specs 为准。

## 来源与范围

- 工程：`trae-workbuddy-switch`；仅同步源码，不执行部署或发布安装包。
- 共同祖先：`187a04c5568dadae6656951a5acec29a2351a970`。
- fork：`mine/main`，`ee36bf7cb0c81b14a6f9b344e1b2e891c90199ce`，65 个独有提交。
- 上游：`origin/main`，`cf8755ae02daa505d0beb364e0c5b6dcecb448bd`，581 个独有提交。
- 备份：`backup/mine-main-before-upstream-20261114` 指向原 fork 提交。
- 上游相对共同祖先修改803文件；双方重叠112文件；实际Git冲突96文件。
- 使用双亲合并保留两侧历史；以新版架构承接旧 fork 功能，逐项解决冲突并检查自动合并的语义碰撞。

## 需要了解的行为变化

1. 导航采用新版六作用域、懒加载和聚合视图，成长任务与豆包保留独立入口；旧管理路径重定向到现行页面。
2. 成长排程改为 `growth_times` / `growth_jitter_minutes`，兼容读取旧 `growth_hours`；旧 fork 默认抖动迁移到新版按任务配置。
3. WorkBuddy 保持有效账号/模型冷却不可绕过。账号冷却返回429；纯模型冷却按新版503及Retry-After表达，不误报凭据失效。新产品线保留各自上游半开行为。
4. 新版网关禁止自动重放失败的非幂等POST；融合旧统计桶与新的按Key/缓存token/完整流式收尾统计。
5. 网关出站代理采用上游显式开关 `upstream_via_proxy`（默认直连）；需要使用系统 `HTTPS_PROXY` 等代理时开启此项。旧版“只要环境变量存在就自动代理”的隐式行为不再沿用。
6. WebUI认证在主界面挂载前完成；未知/网络失败闭锁并支持重试。TOTP配置保留历史 `~/totp_config.json` 路径，内部改用互斥锁与切换事务，重设/禁用撤销旧会话。
7. 更新源保持 `kingkadienm/trae-workbuddy-switch`，签名公钥与原fork字节一致；npm保持 `@kingkadienm`，版本与上游 `2026.11.131828` 对齐；Pages不自动部署。



## 核心、服务端、桌面宿主与发布资料

冻结来源：fork `ee36bf7cb0c81b14a6f9b344e1b2e891c90199ce`，upstream `cf8755ae02daa505d0beb364e0c5b6dcecb448bd`，共同祖先 `187a04c5568dadae6656951a5acec29a2351a970`。备份分支 `backup/mine-main-before-upstream-20261114`。

| 文件/组 | fork 与 upstream 差异 | 整合依据 |
| --- | --- | --- |
| core/Cargo.toml、Cargo.lock | fork TOTP/base32/rand；上游 SQLite backup、RSA/MD5、vendored OpenSSL等新模块依赖 | 逐块取并集，保留 upstream 锁定版本，只添加 fork 所需依赖，未全量更新依赖 |
| core/account.rs | fork deactivated；上游 groupName | 元数据两个字段都保留；停用/启用函数保留，分组字段维护采用 upstream |
| core/catalog.rs | fork step-5-preview/space-bunny、显式 try_refresh 与多策略；上游模型 capability/default-window/reasoning、Global28模型与精细降级原因 | 保留上游完整模型字段和28项Global清单；两项CN模型补齐新字段；保留try_refresh，失败说明用upstream failure_reason，无空清单写入 |
| core/credits.rs | fork workbuddy_api_base 运行时域名；上游格式及额度改进 | 保留运行时域名覆盖，上游其余改进自动合并 |
| core/mod.rs | fork growth/doubao/totp_auth/usage/wb_register；upstream新平台与模块 | 模块并集，不能整文件选边 |
| core/schedule.rs | fork成长队列、固定正向0–30min抖动；上游HH:MM、按任务可配置确定性抖动、Qoder/续期任务与跨日候选修复 | 用上游类型和算法，补growth_times/growth_jitter_minutes/growth_enabled，默认01:00启用，不进启动补跑；旧growth_hours自动读入，含fork旧字段且没有显式jitter时迁移30分钟幅度 |
| core/scheduler.rs | fork成长调度；upstream新任务/统一导入/补跑重查 | 将growth接入上游match与import；保留上游其它任务及启动边界 |
| core/token_stats.rs | fork网关统计桶；upstream增量扫描缓存 | 保留网关桶，并在单区域返回前执行缓存dirty持久化；两侧统计回归均保留 |
| core/trae/account.rs | fork网页免客户端登录标记、权益展示字段、区域绑定；upstream回收站、DeviceSecret、6步双产品线续期、正确account_base、风控响应 | 保留web_devices与14参数权益view；采用upstream生命周期与刷新架构，网页账号只用无proof变体并保留旧协议兜底；新客户端登录清web标记；日志携带变体 |
| core/trae/oauth.rs | fork合成网页身份、可选PKCE、userJwt回调；upstream产品线事实/统一DeviceInfo/host白名单 | 保留网页模式及回调能力，ClientFacts同时带web_device与OAuthLine；身份空公钥不发送DevicePublicKey；采用host白名单与新设备协议 |
| core/trae/credits.rs | fork地区端点、会员权益元信息、Global免费档；upstream无可解析quota不冒充0、访问成功自动解冻SessionDead、风控准入 | 区域端点与权益缓存保留，CN不可解析返回未知，Global免费档仍可返回0并保留包；上游成功解冻/风控准入保留 |
| core/trae/checkin.rs | fork账号顺序/随机停顿测试；upstream风控队列与状态判断 |保留自定义顺序和随机停顿，与新冷却判断结合；测试保留 |
| core/trae/test_support.rs | 两边均修macOS隔离测试平台限制 | 上游说明更完整，采用上游注释 |
| core/travel.rs | fork自动领养、stale token重试、官方Origin/Referer；upstream新判据 | 保留fork行为，去掉合并后未使用旧导入，upstream其余改动吸收 |
| core/upstream.rs | fork模型目录多端点；upstream跨平台SESSION_DEAD_MARKERS | 判据与模型刷新是不同职责，两者保留 |
| core/update.rs、tauri.conf.json | fork自己的发布仓库；upstream更名qoder仓库与新版更新算法 | 更新来源保持kingkadienm/trae-workbuddy-switch，pubkey不变，吸收upstream更新算法与错误诊断 |
| core/export_import.rs（自动合并语义适配） | fork Panel auths导入调用旧merge签名；upstreamConflictPolicy/TokenPolicy | Panel按原覆盖且必须token语义显式传Overwrite/Require，realm分流与秒转毫秒解析保留 |
| core/totp_auth.rs（自动合并语义适配） | fork UnsafeCell错误声称宿主串行；server为多线程 | 保持API与落盘位置兼容，内部改Mutex与AUTH_TRANSITION事务锁、使用home_dir统一隔离路径、禁用/初始化撤销会话；新增完整router开关鉴权回归 |
| server/api.rs | fork TOTP/public-config/relay、成长、豆包、panel导入、删除清池；upstreamAPI来源守卫/作用域/备份/Qoder/companion | 逐组保留路由与实现，来源守卫与TOTP同时生效（TOTP仅保护/api管理面，模型/v1端点仍用自身Bearer Key），模型live刷新搭配新max_context；两侧测试并存 |
| server/main.rs | fork初始化TOTP且写死loopback；upstream host配置 | 保留init_totp，监听host按上游配置 |
| Tauri commands/lib | fork命令注册与删除清池；upstream统一scope、新平台备份、悬浮窗 | 命令与初始化取并集，半函数冲突恢复完整边界；后续契约扫描验证 |
| .github/workflows/pages.yml | fork删除以避免误部署；upstream修改 | 保留pages.yml构建契约，移除push触发、仅workflow_dispatch手动触发；vite本地fallback slug与fork仓库名一致，check-pages-base不降级 |
| LICENSE | fork Required Notice替换作者；upstream原作者Notice且仓库更名 | 两个Required Notice并存，保留原作者与fork贡献者，许可正文不改 |
| README.md/en | fork旧产品版面与fork链接；upstream新增Qoder/架构文档大幅重排 | 按upstream新版事实保留版面，顶部明确fork来源和保留功能，安装/更新发布链接指向fork；upstream署名及说明保留 |
| CONTRIBUTING.md/en | 两侧Issue链接不同 | fork反馈入口保留，上游新增贡献规范保留 |
| docs/history/overview.md | 两侧历史远端文字 | 保留fork已有历史，不改历史事实 |
| docs/policies/LICENSING.md/en | 两侧作者联系不同，上游补捐赠与许可无关 | 保留原作者联系和新增解释，LICENSE同时保留fork贡献者 |
| docs/DEVELOPMENT.md | fork旧开发说明；upstream已迁guides | 保留兼容入口，指向现行guides及fork API_CONTRACT.md |
| docs/images/donate-* | fork删除jpg/png，upstream更新原作者素材 | 保留上游原始素材与fork已有jpeg，避免误删来源资料 |
| npm/README.md | fork旧安装二进制名；上游cli新命名和文档迁移 | 新安装说明与原作者许可来源保留 |
| npm/package.json、5平台包 | fork发布scope及旧版本；upstream新版本 | 保留@kingkadienm命名空间，版本对齐2026.11.131828；平台包一致 |
| scripts/gen-release-notes.mjs/gen-update-json.sh | 两侧默认发布仓库不同 | fork发布默认值保留，上游新生成逻辑吸收 |

机器可复查证据：`*.conflicts` 保存原始每块两侧内容，`resolution-audit*.json`、`resolve*.py` 保存逐块选择与适配；最终以命令测试与独立审查为准。

验证进度：
- `cargo test -p buddy-switch-core` 二轮 exit 0：1727 passed / 16 ignored / 0 failed；生产修复包含真实 Growth TASKS 漏登记，旧结构/域名与设备fixture断言已按新模型修订。
- `node scripts/check-api-contract.cjs` exit 0：308 routes / 329 invokes。
- `node scripts/stamp-version.mjs --check` exit 0：25处版本一致 2026.11.131828。
- `git diff --check origin/main` exit 0；按旧 HEAD 做全量检查会报告上游既有 Markdown硬换行与CRLF，未修改这些历史文档。
- Tauri cargo check -p buddy-switch-rust exit 0；server完整测试进行中；尚未提交或推送。


首轮失败与真实修复：

1. Growth 只有枚举未接新后台表 → 在 `background::TASKS` 真正登记，两个宿主共用该表启动。原 `tasks_cover_all_scheduled_tasks` 不放宽，现通过。
2. 新growth开关未纳入旧“全部关闭”fixture，另一个单猫fixture误带growth → 补全显式开关探针，保留逐任务严格映射；新增旧fork排程迁移/显式0抖动保留测试。
3. 国际refresh旧测试只准icube主机 → 新协议新路径走account_base，旧路径走icube，改断言同时验证正确国际主机且禁止CN域；保留更精确的每变体按path选择endpoint测试。
4. 设备绑定结构断言把rustfmt多行尾逗号算成空参数 → 解析器只忽略末尾空白，仍严格检查实参数量和第三参绑定、第四参web标志。
5. OAuth变体测试对已收敛merged程序位误读旧Global/SOLO设备 → 对齐同一canonical变体，比对其真实fixture身份。
6. 网页降级测试造了设备fixture后只删Windows APPDATA目录 → 使用 `TempEnv::empty()` 明确无设备，不依赖真机或平台目录，生产fallback保留。
7. 参数化account_delete自动合并后绕过fork旧删除清池 → 新WorkBuddy分支委托旧实现，Tauri与server双修；server删除清池回归现覆盖新旧两入口，防止UI改走alias后行为丢失。
8. TOTP原先把模型协议端点也套session → 放在管理面树再merge网关；启用TOTP仍允许模型API按自身Bearer流程返回JSON鉴权结果。
9. API命名门禁与Pages门禁指出fork配置适配缺口 → relay公开配置camel/snake双读、Pages仅手动、vite仓库slug准确；不删除/放宽任何检查。

最终复核：`cargo test -p buddy-switch-core -p buddy-switch-server --locked` 全绿1790 passed/16 ignored/0 failed；`cargo check -p buddy-switch-rust --locked` exit0（18.24s），无警告。logs: final-core-server-test.log / final-tauri-check.log。未执行真实账号网络调用、Windows/Linux桌面实机、安装包签名/发布/部署。合并提交与远端推送由父代理在冻结指纹与独立审查后完成。

## 网关

比较对象：共同祖先 `187a04c5568dadae6656951a5acec29a2351a970`、fork `ee36bf7cb0c81b14a6f9b344e1b2e891c90199ce`、上游 `cf8755ae02daa505d0beb364e0c5b6dcecb448bd`。父代理授权范围仅 `crates/buddy-switch-gateway/**`。

### 逐文件决策

| 冲突文件（均相对 crate 根） | fork 的差异 | 上游的差异 | 最终选择与理由 |
| --- | --- | --- | --- |
| `src/account_strategy.rs` | `EarliestExpiry`、池缓存新鲜时零网络选择、停用账号过滤 | 新 `NoCredentialCause`、`Random`/`Sequential`、派生 Default | 合并为六种策略；保留 fork 低频选择器和到期时间 helper，补新版错误原因；随机兜底也过滤停用账号。热路径改走上游共用池内核，避免持锁联网查询全部账号。 |
| `src/lib.rs` | AutoClaw 模块/代理与健康检查 | `/health`/`healthz` 单源挂载、新 Responses、业务路由 | 新路由骨架保留，加入 AutoClaw 模块与 fallback 挂载；两套健康路径仍等价。 |
| `src/pool/mod.rs` | 删除账号立即清池、按 region 裁剪、停用标记、到期时间持久化 | 用户排除表、存在性标记、模型闸门、验证票、摘要诊断等 | 保留新版治理结构，删除使用 fork 立即移除语义，且仅裁剪本 realm；存活账号的治理状态不动。保留 `present_in_store` 防御竞态和旧状态，恢复 `remove` API；`credits_expire_at_ms`/`deactivated` 持久化保留。 |
| `src/pool/pick.rs` | 全池软/模型冷却不得绕过、停用账号不可选 | 泛型 `PoolEntryLike`、Key白名单/排除/在途判定、随机与顺序策略、软冷却半开兜底 | 新泛型架构与共用硬约束保留；新增 `EarliestExpiry` 直接参与候选内排序。WB覆写 `allows_cooling_probe=false`，严格禁止有效冷却强试；Qoder/Trae保留上游默认半开行为，避免把旧 WB 专属要求扩散到新产品线。 |
| `src/protocol/usage_tap.rs` | 把 prompt/completion 分列回报 | `finish(Option<UsageReport>)` 生命周期、缓存命中字段、错误信封嗅探、多 sink | 采用新版完整生命周期：它已覆盖 prompt/completion 分列，并多出缓存token与完整收尾。旧桶记账迁到新版额外 sink；不保留冲突的旧 record ABI。 |
| `src/routes/chat.rs` | 失败/成功请求计入旧用量桶 | 首帧预读、跨平台降级、流中错误、按Key/账本/日志用量 | 上游完整路由骨架；在真实失败/成功出口重新接入旧桶；流式由统一 sink 接入，保留所有新版嗅探/失败切换链。 |
| `src/routes/messages.rs` | 同上（Anthropic） | 同上，另有输入估算/缓存token | 同上，保留 Anthropic 新的估算与缓存计费字段，旧桶在非流式成功与失败出口接回。 |
| `src/routes/models.rs` | 合并 AutoClaw 模型 | 可用/禁用过滤、上下文与图像能力声明 | 两者并存；上游目录过滤作为基础，追加去重的 AutoClaw 模型。 |
| `src/routes/relay.rs` | 策略首选、UID尾号日志、桶统计、池非空禁止回落绕冷却 | 风控准入、统一 fallback、首帧识别、Key隔离、故障归因、会话诊断、验证票 | 上游控制流为骨架；保留 Current 的客户端首选、Pinned首选，不可用时优先粘性再池；Max/Earliest/Random/Sequential各在合格候选内执行。日志保留UID尾号。旧桶接统一 stream_sinks，Responses也一并覆盖。纯账号冷却返回429，纯模型冷却沿用上游503+Retry-After，避免401误报。 |
| `src/state.rs` | AutoClaw配置/状态/客户端、旧 `UsageRecorder`、隐式 HTTPS_PROXY | 新状态诊断、append-only流水、显式 `upstream_via_proxy`、模型思考档/上下文配置 | 配置与状态字段取并集，保留旧桶与桥接；代理采用上游显式开关（默认直连，可设置 `upstream_via_proxy=true` 使用系统代理），替代旧无条件读取 HTTPS_PROXY。 |
| `src/sticky.rs` | 删除用 `remove_uid` 批量解绑 | 排除/禁用/删除统一 `purge_uid`，空UID保护 | 使用上游实现，增加 `remove_uid` 薄兼容别名，避免旧删除调用失效。 |
| `src/trae/payload.rs` | 未知模型不再静默换成默认、加入step/space-bunny，但派生名把连字符换下划线 | 实测未知名按原名+`__dev`、更完整客户端静态清单、过滤实测拒绝模型 | 采用上游实测映射规则与完整新请求体，保留 fork 历史 `space-bunny` 兜底名；避免把 `step-5-preview` 错改为 `step_5_preview`。 |
| `src/trae/pool.rs` | Global 免费档零积分可用、variant归属与free_plan状态 | 泛型池共用、可持续顺序序号、冷却类别/排除/白名单 | 字段/初始化取并集；Global 免费档例外与状态显示保留，新泛型 trait 的健康判据继续复用 rejection，保持展示与选号同源。 |
| `src/trae/routes.rs` | 上游host随CN/Global变体，支持测试override | 出站拆成 chat_once_for_account、风控准入、抓取旁路与新payload参数 | 保留上游完整新流程，把变体选host迁入抽出的 chat_once_for_account；同时传给 send_llm_chat，默认CN/Global不串区、测试override仍有效。 |

### 自动合并部分的补充核对

- `src/pool/entry.rs` 已自动保留到期时间/停用字段，但上游 `healthy` 和 trait `is_disabled` 未检查停用。已补共同判据，避免手动停用从新随机/顺序/偏好路径漏回。
- `src/pool/entry_like.rs` 增加可选积分到期时刻和产品线冷却探针策略，默认值保持上游新产品线兼容。
- `src/credits_refresh.rs` 自动合并保留 `soonestExpireAt` 写回，`EarliestExpiry` 读取相同字段与毫秒单位。
- 上游新增 `routes/responses.rs` 也接入旧用量桶，避免新协议统计缺口。
- 上游目前 `should_rotate` 明确禁止自动重放失败的非幂等 POST。保留现行上游行为；没有把 fork 旧版每失败自动重发整个 POST 的逻辑生搬回来。
- 序列化/测试ABI适配：Trae池新增 pick 参数；Trae AccountsFile新增 `web_devices`；新版 send_llm_chat 参数同步更新测试探针编译。未执行任何真实账号/真实模型探针。

### 验证设计

- 新增 `tests/fork_cooldown_merge_e2e.rs`：真实鉴权+路由+夹具账号，账号级冷却须在3秒内直接回429且不报no_credential。
- 选择器测试覆盖加权/积分最多/最近到期/随机/顺序与显式偏好：有效6004、停用、Key名单/排除不得绕过；最近未来到期优先。
- 保留并适配上游原有模型冷却503/闸门Retry-After、完整三协议和跨产品线集成测试。

实际执行结果以同目录 `test-*.result.json` 与日志为准；本报告不将编译开始或代理自报视为独立验收。

### 上游遗留测试契约不一致的实证与修正

- 上游 `c96ebd9`（`feat: 接入三平台可配置风控`）把 `qoder/pool.rs::QoderErrKind::should_rotate` 从 `!matches!(self, ClientError)` 明确改为无条件 `false`，同提交把相关单测/余额耗尽说明改为「不由其他账号接力」。当前 `qoder/routes.rs` 收到失败后先分类、写回模型冷却，随后因 `!kind.should_rotate()` 直接返回原HTTP状态。此两文件与 `origin/main` 无差异。
- 旧 `tests/qoder_gateway_e2e.rs` 仍期待429后换号成功200、单号耗尽后503，与上述新版现行契约不一致。首次完整执行得到这两条失败，其他14条Qoder集成均通过。
- 修正测试名/断言为单次429原样保留，额外断言 mock **恰收1次POST**、首个调用账号模型冷却已落、第二个账号无失败冷却；单账号429也恰好1次POST。没有更改Qoder生产重试政策，也没有削弱模型fallback/结构化→文本降级/显式auto_failover受控路径测试。

- Trae 同提交在流式/聚合两条 `AttemptResult::Failed` 分支明确新增 `terminal = matches!(status, 401|402|403|429) || code == upstream_unreachable`，命中即停止轮换。旧 Trae 401换号集成期待200同样过时。新断言保留401、mock捕获的JWT列表必须仅为 `[jwt-a]`、A写入AuthRejected浅冷却而B无冷却。其它上游受控重试/模型降级路径未改。
- WB单账号模型冷却503集成原夹具明确「只upsert内存，不写store」，与fork删除即清池的前置条件冲突：同步时本应裁剪。已把该唯一账号持久化到隔离账号库，原503、`Retry-After`、`model_gate_paused`、模型/可用账号数原因断言全部保留。没有把401实际结果改成期待值。

### 最终本岗验证

`cargo test -p buddy-switch-gateway --offline --no-fail-fast` 退出码0：973通过、0失败、5忽略（2个真实账号探针、3个文档示例）。`git diff --cached --check -- crates/buddy-switch-gateway` 退出码0，范围内工作树与index一致。源码范围指纹：`5574be37ff16b588bc8c25664681214cee22e0b8ff983b1990031639027d36fc`；计算命令 `git ls-files -s -z -- crates/buddy-switch-gateway | shasum -a 256`。

## 前端

来源：fork `ee36bf7`，upstream `cf8755a`，共同祖先 `187a04c`。原始 stage1/2/3 与逐块冲突保留在同目录 `stages/`。下列是合并决策，不代表已获独立验收。

### 关键行为对比

| 范围 | fork 与上游区别 | 采用方案与依据 |
| --- | --- | --- |
| App 导航 | fork 3产品旧路径，上游6作用域注册表/聚合/命令面板 | 保留新版注册表与全部6作用域；成长任务与豆包作为独立路由和侧栏扩展，不污染作用域枚举。旧模型/池/日志路径重定向新网关分区；保留配置编辑、活跃、统一统计、批量账号路径。 |
| TOTP | fork为布局内绝对遮罩，上游无TOTP | 新增外层WebuiAuthGate：认证后才挂载Router/快捷键/所有浮层；未知状态/网络失败闭锁+重试；残留session需服务端校验；401卸载受保护内容；桌面/demo沿原宿主路径。6项React实际渲染单测。设置启停卡保留在通用设置组。 |
| HTTP/域名 | fork同源Nginx与运行时公开配置，上游按localhost监听派生 | 管理API保留同源空基址（Vite代理+生产Nginx兼容），X-Session-Token/401回调保留；WebUI网关展示读取公开配置，按新gatewayOpenAiBaseUrlFrom归一/v1；桌面继续监听地址。 |
| 成长任务 | fork独有19动作/队列/单账号弹窗/调度，上游无 | API、演示只读命令、词条保留；卡片与表格共用更多菜单都有入口；账号页CN队列和独立任务中心可达；调度迁为growth_times/growth_jitter_minutes/growth_enabled，与core新schema对齐。 |
| 账号停用 | fork独有停用/启用，上游重构卡片与表格 | 统一AccountMoreMenu加入停用/启用回调，卡片/表格同时可用，deactivated徽章保留；账号列表与服务端元数据同形。 |
| 最近到期 | fork新加每卡与列表汇总，上游有更完整逐包/临期指标 | 每卡用新版summarizeExpiring/统一逐包详情；列表保留近期到期合计与账号数，读取后端expiringSoonRemaining。 |
| 网关策略 | fork earliest_expiry，上游新增Random/Sequential并将下拉并入账号池卡 | 保留全部AccountStrategy联合，WB新池卡加入earliest_expiry哨兵与持久化，Random/Sequential保留；Current仍为跟随当前客户端首选，WB下拉用“当前登录账号”文案，未套用上游“自动择优”标签；旧策略组件仅保留兼容。 |
| Trae | fork免客户端网页登录/userJwt后备与Work会员，上游统一抽屉/程序位/手动回调授权 | 保留上游AuthLinkPanel/手动回调/客户端检测，增加webMode说明；保留Work/通用积分、会员到期/下次扣款/身份到新版账号详情；Global卡片隐藏不适用签到；free_plan共用池状态有正确文案。 |
| 更新 | fork自有kingkadienm更新源与手动检查，上游自动更新开关/焦点补查 | 自有源保留；自动轮询沿上游缓存逻辑；版本号旁手动按钮调用独立checkManualUpdate，force仅在显式点击路径，避免干扰自动补查。 |
| 导入 | fork panel目录导入且WebUI禁本地读，上游统一添加抽屉/无客户端空态 | panel目录弹窗与导入语义保留；WebUI按钮禁用且handler防御；本地自动导入在WebUI跳过，授权/粘贴/备份仍可用；WB空态仍有OAuth入口。 |
| 翻译 | 上游大量新增key、fork成长任务/会员等独有key | 以新版同名词条为准，补fork独有双语词条，保持zh/en键集完全一致；上游检查器不降级。 |
| 锁文件与图片 | 上游新增Radix/cmdk/vitest/jsdom，fork图片改jpeg | 上游lock覆盖全部依赖，npm ci实际安装；保留fork jpeg并接收上游旧名资源用于新版引用，未改署名。 |
| 自动合并语义碰撞 | fork旧自定义页raw email回落触发新版脱敏门禁 | 策略卡、旧池页、批量账号页都改为共享accountDisplayName；不放宽门禁。 |

### 初始冲突文件清单与处理

| 文件 | 首轮处理（随后按上表定向移植） |
| --- | --- |
| `package-lock.json` | 上游锁文件包含新增组件与测试依赖；fork无独有依赖，后续npm安装/构建复核 |
| `src/App.tsx` | 采用上游重构骨架，随后定向移植fork专有行为；旧布局/重复统计/旧模型表由新版同类实现替代 |
| `src/assets/donate-alipay.jpg` | fork将旧jpg/png改为jpeg；保留既有jpeg并接收上游旧名资源供新版引用，未更改署名 |
| `src/assets/donate-wechat.png` | fork将旧jpg/png改为jpeg；保留既有jpeg并接收上游旧名资源供新版引用，未更改署名 |
| `src/components/account-card.tsx` | 采用上游重构骨架，随后定向移植fork专有行为；旧布局/重复统计/旧模型表由新版同类实现替代 |
| `src/components/app-settings.tsx` | 采用上游重构骨架，随后定向移植fork专有行为；旧布局/重复统计/旧模型表由新版同类实现替代 |
| `src/components/gateway/account-strategy-card.tsx` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/components/gateway/model-list.tsx` | 采用上游重构骨架，随后定向移植fork专有行为；旧布局/重复统计/旧模型表由新版同类实现替代 |
| `src/components/oauth-login-dialog.tsx` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/components/trae-account-card.tsx` | 采用上游重构骨架，随后定向移植fork专有行为；旧布局/重复统计/旧模型表由新版同类实现替代 |
| `src/components/trae-oauth-login-dialog.tsx` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/lib/api.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/lib/screenshot-demo.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/lib/trae-gateway.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/lib/trae-types.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/lib/types.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/lib/update.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/locales/domains/shared.en.ts` | 以新版词条为准，合并fork独有24词条，避免覆盖新版共享key |
| `src/locales/domains/shared.zh.ts` | 以新版词条为准，合并fork独有24词条，避免覆盖新版共享key |
| `src/locales/domains/shell.en.ts` | 以新版词条为准，合并fork独有17词条，避免覆盖新版共享key |
| `src/locales/domains/shell.zh.ts` | 以新版词条为准，合并fork独有17词条，避免覆盖新版共享key |
| `src/locales/domains/traeComponents.en.ts` | 以新版词条为准，合并fork独有42词条，避免覆盖新版共享key |
| `src/locales/domains/traeComponents.zh.ts` | 以新版词条为准，合并fork独有42词条，避免覆盖新版共享key |
| `src/locales/domains/traePages.en.ts` | 以新版词条为准，合并fork独有47词条，避免覆盖新版共享key |
| `src/locales/domains/traePages.zh.ts` | 以新版词条为准，合并fork独有47词条，避免覆盖新版共享key |
| `src/locales/domains/wbAccounts.en.ts` | 以新版词条为准，合并fork独有144词条，避免覆盖新版共享key |
| `src/locales/domains/wbAccounts.zh.ts` | 以新版词条为准，合并fork独有144词条，避免覆盖新版共享key |
| `src/locales/domains/wbSettings.en.ts` | 以新版词条为准，合并fork独有32词条，避免覆盖新版共享key |
| `src/locales/domains/wbSettings.zh.ts` | 以新版词条为准，合并fork独有32词条，避免覆盖新版共享key |
| `src/locales/domains/wbStats.en.ts` | 以新版词条为准，合并fork独有193词条，避免覆盖新版共享key |
| `src/locales/domains/wbStats.zh.ts` | 以新版词条为准，合并fork独有193词条，避免覆盖新版共享key |
| `src/locales/en.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/locales/zh.ts` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/pages/AccountsPage.tsx` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/pages/ApiServicePage.tsx` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/pages/SettingsPage.tsx` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |
| `src/pages/TokenStatsPage.tsx` | 采用上游重构骨架，随后定向移植fork专有行为；旧布局/重复统计/旧模型表由新版同类实现替代 |
| `src/pages/TraeAccountsPage.tsx` | 采用上游重构骨架，随后定向移植fork专有行为；旧布局/重复统计/旧模型表由新版同类实现替代 |
| `src/pages/TraeApiServicePage.tsx` | 逐块保留上游架构/类型；对新增import、命令、独立弹窗使用并集，重叠布局选上游，fork功能另定向迁入 |

### 验证证据

- `npm ci --ignore-scripts --no-audit --no-fund`：成功，298依赖。
- `npm run test:unit`：49测试通过（含6项认证门渲染测试）；完整日志`unit-tests.log`。
- `npm run build`：完整最终结果以`build-final-frozen.log`及父代理记录为准。早期编译、契约/词条/脱敏静态门禁失败均保留日志，已按生产代码修复。
- 未进行真实浏览器/桌面宿主点击、真实账号登录、上游模型调用；不能把JSDOM渲染视为这些验收。

补充：上游新增文件credit-expiry.ts/update-preference.ts结尾多余空行已规范化；范围内git diff --check通过。

当前范围指纹：`272738d553c64ea5896fe5d823e36f07ddbdeec880ef911d3e52f97d0a5d3ee6`；命令：`git ls-files -s -z -- src package.json package-lock.json | shasum -a 256`。

### 网关用量双计碰撞（实际生产路径）

- fork旧`get_token_statistics`保留`workbuddy-gateway`虚拟源，兼容旧UnifiedStats/API调用与旧usage数据。
- 上游新版`TokenStatsPage`原先`mergeSources(stats.sources)`把每个source当客户端相加（`usage-charts.ts`中的summary/daily聚合），而趋势同时从`get_gateway_token_trend`取独立网关账本再算`combinedTotal`，两条路径相撞会双计。
- 修复边界：新版TokenStatsPage先`workbuddyClientSources(stats.sources)`排除旧虚拟源，再merge；不修改旧接口/原始数组，旧页面仍可合并旧来源。
- `src/lib/__tests__/workbuddy-ledger.test.ts`用真实合并/combinedTotal函数验证：客户端10+5、旧虚拟源20、新ledger20 ⇒ 客户端15、总计35（不是55）；只有旧虚拟源时客户端0、总计20；旧来源数组仍3条。
- 最终前端单测51项全部通过（6项TOTP渲染+2项账本回归）。

最终冻结验证：`npm run build`（`build-final-frozen.log`）exit0，包含TypeScript、API契约、scope337/0、全部原型套件、51/51单测、Vite产物。`git diff --check --cached -- src package.json package-lock.json` exit0。Vite首屏chunk体积警告仍存在，不影响构建；未把警告写成失败。

## 汇总验证与限制

以下均为同一合并候选源码上的实际命令，退出码均为0；不是生产服务或真实账户验收。

| 命令 | 实际结果 |
| --- | --- |
| `npm run build` | TypeScript、API、Docker、store/hooks、337项scope检查、Pages、13套原型检查、51项单测及Vite构建通过 |
| `cargo test -p buddy-switch-core -p buddy-switch-server --locked` | 1790通过、0失败、16忽略 |
| `cargo test -p buddy-switch-gateway --offline --no-fail-fast` | 973通过、0失败、5忽略 |
| `cargo check -p buddy-switch-rust --locked` | 桌面宿主编译检查通过，无警告 |
| fork新增前端命令键和HTTP路由覆盖比较 | fork新增键与路由全部保留；行为另由上述测试验证 |
| 更新配置比对 | updater公钥与原fork一致，更新端点及npm包指向kingkadienm，平台包版本一致 |

前端单测覆盖TOTP闭锁/重试/会话失效等6项React渲染断言，以及客户端/网关用量不重复计数回归。网关集成测试用隔离账号库和本地模拟上游，断言实际出站次数、冷却状态及HTTP结果。未修改检查器以放宽门禁。

未执行：真实浏览器/桌面客户端逐入口点击、原型9项人工验证、真实账号登录与模型调用、Windows/Linux桌面实机、安装包签名发布、部署、性能与长时运行。构建仍有Vite大chunk提示，不影响退出成功。

独立审查结果、最终提交SHA、push回执和源码指纹记录在本次交付回执；本报告不把实施者自报当作独立批准。

## 完整原始冲突清单

用冻结双方SHA执行`git merge-tree --write-tree --name-only ee36bf7 cf8755a`重演，确认**96个实际冲突路径**（初步人工统计97已更正）。结果与初始保存的冲突清单逐项一致，覆盖内容、修改/删除、重命名及二进制冲突。

逐块比较可复现：

```sh
git diff 187a04c5568dadae6656951a5acec29a2351a970 ee36bf7cb0c81b14a6f9b344e1b2e891c90199ce -- <path>
git diff 187a04c5568dadae6656951a5acec29a2351a970 cf8755ae02daa505d0beb364e0c5b6dcecb448bd -- <path>
```


- `.github/workflows/pages.yml`
- `CONTRIBUTING.en.md`
- `CONTRIBUTING.md`
- `Cargo.lock`
- `LICENSE`
- `README.en.md`
- `README.md`
- `crates/buddy-switch-core/Cargo.toml`
- `crates/buddy-switch-core/src/modules/account.rs`
- `crates/buddy-switch-core/src/modules/catalog.rs`
- `crates/buddy-switch-core/src/modules/credits.rs`
- `crates/buddy-switch-core/src/modules/mod.rs`
- `crates/buddy-switch-core/src/modules/schedule.rs`
- `crates/buddy-switch-core/src/modules/scheduler.rs`
- `crates/buddy-switch-core/src/modules/token_stats.rs`
- `crates/buddy-switch-core/src/modules/trae/account.rs`
- `crates/buddy-switch-core/src/modules/trae/checkin.rs`
- `crates/buddy-switch-core/src/modules/trae/credits.rs`
- `crates/buddy-switch-core/src/modules/trae/oauth.rs`
- `crates/buddy-switch-core/src/modules/trae/test_support.rs`
- `crates/buddy-switch-core/src/modules/travel.rs`
- `crates/buddy-switch-core/src/modules/update.rs`
- `crates/buddy-switch-core/src/modules/upstream.rs`
- `crates/buddy-switch-gateway/src/account_strategy.rs`
- `crates/buddy-switch-gateway/src/lib.rs`
- `crates/buddy-switch-gateway/src/pool/mod.rs`
- `crates/buddy-switch-gateway/src/pool/pick.rs`
- `crates/buddy-switch-gateway/src/protocol/usage_tap.rs`
- `crates/buddy-switch-gateway/src/routes/chat.rs`
- `crates/buddy-switch-gateway/src/routes/messages.rs`
- `crates/buddy-switch-gateway/src/routes/models.rs`
- `crates/buddy-switch-gateway/src/routes/relay.rs`
- `crates/buddy-switch-gateway/src/state.rs`
- `crates/buddy-switch-gateway/src/sticky.rs`
- `crates/buddy-switch-gateway/src/trae/payload.rs`
- `crates/buddy-switch-gateway/src/trae/pool.rs`
- `crates/buddy-switch-gateway/src/trae/routes.rs`
- `crates/buddy-switch-server/src/api.rs`
- `crates/buddy-switch-server/src/main.rs`
- `docs/DEVELOPMENT.md`
- `docs/history/overview.md`
- `docs/images/donate-alipay.jpg`
- `docs/images/donate-wechat.png`
- `docs/policies/LICENSING.en.md`
- `docs/policies/LICENSING.md`
- `npm/README.md`
- `npm/package.json`
- `npm/platform/buddy-switch-darwin-arm64/package.json`
- `npm/platform/buddy-switch-darwin-x64/package.json`
- `npm/platform/buddy-switch-linux-arm64/package.json`
- `npm/platform/buddy-switch-linux-x64/package.json`
- `npm/platform/buddy-switch-win32-x64/package.json`
- `package-lock.json`
- `scripts/gen-release-notes.mjs`
- `scripts/gen-update-json.sh`
- `src-tauri/src/commands.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/tauri.conf.json`
- `src/App.tsx`
- `src/assets/donate-alipay.jpg`
- `src/assets/donate-wechat.png`
- `src/components/account-card.tsx`
- `src/components/app-settings.tsx`
- `src/components/gateway/account-strategy-card.tsx`
- `src/components/gateway/model-list.tsx`
- `src/components/oauth-login-dialog.tsx`
- `src/components/trae-account-card.tsx`
- `src/components/trae-oauth-login-dialog.tsx`
- `src/lib/api.ts`
- `src/lib/screenshot-demo.ts`
- `src/lib/trae-gateway.ts`
- `src/lib/trae-types.ts`
- `src/lib/types.ts`
- `src/lib/update.ts`
- `src/locales/domains/shared.en.ts`
- `src/locales/domains/shared.zh.ts`
- `src/locales/domains/shell.en.ts`
- `src/locales/domains/shell.zh.ts`
- `src/locales/domains/traeComponents.en.ts`
- `src/locales/domains/traeComponents.zh.ts`
- `src/locales/domains/traePages.en.ts`
- `src/locales/domains/traePages.zh.ts`
- `src/locales/domains/wbAccounts.en.ts`
- `src/locales/domains/wbAccounts.zh.ts`
- `src/locales/domains/wbSettings.en.ts`
- `src/locales/domains/wbSettings.zh.ts`
- `src/locales/domains/wbStats.en.ts`
- `src/locales/domains/wbStats.zh.ts`
- `src/locales/en.ts`
- `src/locales/zh.ts`
- `src/pages/AccountsPage.tsx`
- `src/pages/ApiServicePage.tsx`
- `src/pages/SettingsPage.tsx`
- `src/pages/TokenStatsPage.tsx`
- `src/pages/TraeAccountsPage.tsx`
- `src/pages/TraeApiServicePage.tsx`
