# WorkBuddy 视频网关架构

> 状态：现行（2026-11-17，版本 `v2026.11.171425`）。范围：视频 P1 的三条路由、WorkBuddy 上游适配及进程内任务；操作步骤见 [视频接入指南](../guides/VIDEO-GATEWAY-INTEGRATION.md)。

视频模块复用 WorkBuddy 网关鉴权与出站客户端，提供「动态目录 → 单次提交 → 按需查询」闭环。它不是多供应商通用 API，也不声明 OpenAI 视频完整兼容。本文依据当前源码与历史验收边界，本轮仅做源码/文档核对，未执行真实媒体请求。

## 模块与调用流程

| 所属文件 | 职责 |
|:--|:--|
| [`lib.rs`](../../crates/buddy-switch-gateway/src/lib.rs) | 挂载 `GET /v1/video/models`、`POST /v1/videos`、`GET /v1/videos/:id` |
| [`video/routes.rs`](../../crates/buddy-switch-gateway/src/video/routes.rs) | 复用 `routes::authenticate`，解析输入、调用服务、输出 JSON 或网关错误信封 |
| [`video/models.rs`](../../crates/buddy-switch-gateway/src/video/models.rs) | 解析 `/v3/config` 的 `videoGeneration`；检查能力声明完整性并形成 region 目录 |
| [`video/service.rs`](../../crates/buddy-switch-gateway/src/video/service.rs) | Key/账号筛选、目录能力校验、提交绑定、状态归一化和任务读取隔离 |
| [`video/provider.rs`](../../crates/buddy-switch-gateway/src/video/provider.rs) | WorkBuddy 上游 URL、认证头、业务码/嵌套信封解析及共享 `UpstreamClient` 出站 |
| [`video/store.rs`](../../crates/buddy-switch-gateway/src/video/store.rs) | `RwLock<HashMap<...>>` 保存任务及终态保护 |
| [`state.rs`](../../crates/buddy-switch-gateway/src/state.rs) | 持有共享任务表；从现有身份模块解析 WorkBuddy UA 版本 |

1. **目录**：验证 Bearer Key → 拒绝配额受限 Key → 按 region/domain/Key 范围筛选可用账号 → GET 上游 `/v3/config` → 只返回带视频能力的模型，响应为 `object=list`、`data` 数组。目录异常不静默退回硬编码清单。
2. **提交**：重新获取所选账号的动态目录 → 校验模型、提示词长度、正整数时长、分辨率、比例、音频及首帧 → POST 上游 `/v2/videos/generations` **一次** → 校验业务码和任务 ID → 创建本地 `vid_` 任务，绑定 Key ID、region、原账号和上游任务 ID。
3. **查询**：按本地 ID 找到任务并检查 Key ID/region → 终态直接返回缓存 → 非终态重新校验原账号及 Key 范围 → POST 上游 `/v2/videos/tasks`（携带上游任务 ID）→ 校验业务码和状态 → 更新内存记录并返回。外部接口仍是 GET，网关不在后台主动轮询。

`VideoSubmitInput` 接受顶层参数，也接受 `extra_parameters` 内的 `resolution`、`aspect_ratio`、`enable_audio`；重复值冲突、非法类型和显式 `null` 被拒绝。上游负载把这三项收敛到 `extra_parameters`，缺省值取动态目录。图生仅实现单首帧 HTTPS `image_url`；提供首帧且未指定比例时不强加文生默认比例。目录内的尾帧和多参考素材声明不等于网关已经支持。

## 认证与作用域

客户端使用对应 region、无配额限制的 `sk-wb` Key；region 由已验证 Key 决定，不接受客户端声明覆盖。视频 P1 未实现视频配额计量，因此拒绝 `QuotaMode::Limited`，无配额限制也不代表上游免费。

账号筛选检查：非空账号标识、未停用、非空 access token、无凭据错误、`domain` 经现有 `ensure_region` 判定与 Key region 一致，以及账号/分组在 Key 访问范围内。domain 的历史缺省行为沿用现有区域解析器，不应描述成所有账号都已取得独立 domain 证明。当前视频选号是筛选后首个匹配账号，不承诺复用聊天的完整调度、换号或降级策略。

上游请求复用 `account::build_auth_headers`，追加 `X-Product: SaaS`，使用按 region 解析的 `WorkBuddyAI/<version>` UA；不把 refresh token 加入视频请求。网关 Key 用于本地鉴权，上游使用账号凭据。

任务绑定 **Key ID + region + 原账号**。错误 Key 或 region 与不存在任务采用相同的未找到路径，不能跨域读取；本地查询未找到当前返回 BadRequest 错误信封，不能假定一定是 HTTP 404。查询时仍需有效的同一 Key；新建同区域 Key 不能接管旧任务。非终态查询若原账号不可用或不再在 Key 范围内，返回错误，不换号。

## 状态与响应

| 对外状态 | 上游映射 | 处理 |
|:--|:--|:--|
| `pending` | pending/queued/waiting；提交响应缺状态也先等待 | 非终态，客户端稍后再查 |
| `generating` | 其它非空非终态状态 | 非终态，客户端稍后再查 |
| `completed` | succeed/success/succeeded/completed/complete/done | 终态，缓存返回 |
| `failed` | fail/failed/error | 终态，顶层字符串 `error` 描述失败 |

查询响应缺失或空白状态会报错并保留原记录，不会用未知状态覆盖已有进度。上游网络/HTTP 错误、非零业务码、非法 JSON 或异常信封也不会把现有任务直接写成失败。存储更新拒绝覆盖已进入终态的任务，避免迟到查询倒退状态；终态缓存读取不再请求上游。

任务 DTO 包含 `id`、`region`、`model`、`status`、`error`、`upstream_task_id`、原始 `upstream` 及毫秒创建/更新时间；内部 Key ID、账号标识和账号名不作为 DTO 字段返回。传输或业务错误沿用网关 `.error.message` 信封，与任务失败时的 `.error` 字符串区分。

历史 WorkBuddy 完成响应的 URL 路径是 `upstream.data.data[0].url`，积分路径是 `upstream.data.usage.credit`；这只是本次实测响应形状，未归一化为稳定下载/计费契约。客户端应同时检查 `status=completed` 和 URL 非空，再直取 URL；不得把 signed URL 当永久资源地址或把单次 credit 当固定价格。

## P1 边界与证据

- **没有** `/content`、`/credits`、上传或取消接口；不代下载、不归一化积分，也没有本地素材 HTTPS 桥。API 的首帧图生实现尚未真实生成验证，火宝本地 Base64 素材不能直接作为 HTTPS `image_url`。
- 任务表仅在进程内存，重启丢失；没有持久化、幂等、重启恢复或后台自动轮询接口。提交不自动重试；超时后上游可能已经受理但本地尚无可查询记录，调用方不能盲重发 POST。
- 已实现 scope 绑定、业务码检查、瞬态查询保留状态及终态保护；这些行为不补足上述恢复能力。
- 2026-11-16 历史真实生成验收仅有 CN MiniMax 完成样本：生成视频文件时长约 4.46 秒、实扣 62.29 积分；文件时长不是生成耗时，也不是固定价格保证。CN/Global 其它模型仅目录验证；不声称 Seedance、Wan、Kling 生成实测。fixtures 保存目录形状，不是实时模型或生成成功证明。
- 火宝 custom 模板/轮询来自用户本地增强版，不能推定官方所有版本具备；其「测试连接」会真实提交生成、可能扣费，保存配置也不能作为连通性验收。

扩展上游、素材、持久化或计费时应修改对应拥有模块，并同步本架构及接入指南；本轮文档工作不包含实现变更或新增真实调用验收。
