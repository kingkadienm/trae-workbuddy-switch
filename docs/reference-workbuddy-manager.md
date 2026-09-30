# 外部参考：ithtelab/workbuddy-manager 及其牵出的同栈项目

> 调研日期：2026-09-29 ｜ 对象：`https://github.com/ithtelab/workbuddy-manager`（627★，Python/FastAPI + Next.js）
> 目的：判断这个项目对 **buddy-switch**（Tauri 桌面端账号切换器）有什么可参考的。
> 结论口径：**协议层知识可搬，架构不可搬**；并且它牵出了一个比它本身更值得看的同栈项目。

---

## 0. 结论先行（三条）

1. **最值钱的是「协议事实」而不是代码。** 它和我们打的是**同一个腾讯后端**（CodeBuddy / WorkBuddy），
   仓库里把扫码登录、令牌刷新、签到、积分、模型目录、国际版地区注册的**端点、请求头、错误码、
   脏数据钳位规则**都写成了一手实测结论，且大量注释标注了「与上游 Go 源码逐条对齐」。
   这些可以直接拿去核对/补全我们的客户端协议实现。

2. **架构参考价值接近零。** 它是「服务端集中式账号池 + OpenAI 反代网关」，我们是「桌面端本机切换器」，
   数据面、部署面、威胁模型全不同。抄架构会走偏。

3. **★★ 真正该看的是它致谢里的 `lbjlaq/Antigravity-Manager`（31805★，Rust/Tauri v2）。**
   同技术栈、同产品形态（账号管理与切换）、同平台（Windows 桌面），
   连模块划分（`oauth.rs` / `oauth_server.rs` / `device.rs` / `scheduler.rs` / `quota.rs` / `utils/crypto.rs`）
   都能一一对上我们的关注点。**优先级高于 workbuddy-manager 本身。**

⚠️ **可复现性警告**：它依赖的上游 `Sliverkiss/workbuddy2api` 在 GitHub 上**已无法解析**
（`Could not resolve to a Repository`）。仓库自带说明也写明「单独 clone 本仓库无法运行」。
所以它是**可读不可跑**的参考，只能当文档用。

---

## 1. 可直接采用的协议事实（与我们同源）

来源：`server/services/tencent.py`、`server/services/realm.py`、`docs/WorkBuddy-WebUI设计与实现方案.md`

### 1.1 扫码登录三步（CN / Global 同路径，仅 base 与 Origin/UA 变）

| 步 | 请求 | 要点 |
|---|---|---|
| 1 | `POST {chat_base}/v2/plugin/auth/state?platform=CLI`，body `{}` | 回 `{state, authUrl}` |
| 2 | `GET {chat_base}/v2/plugin/auth/token?state=` | 未扫码 `code != 0`；成功回 `{accessToken, refreshToken, expiresIn, domain}`（**驼峰**） |
| 3 | `GET {chat_base}/v2/plugin/login/account?state=`，**必须带** `Authorization: Bearer {accessToken}` | 回 `{uid, enterpriseId, nickname}` |

**值得直接抄的两个判断：**

- **state TTL 放宽到 900 秒**（`tencent.py:53`）。理由写得很实在：腾讯授权页除扫码外还支持
  **手机号 + 短信验证码**，短信有运营商延迟、用户会中途翻手机，5 分钟很容易超。
  上游对此**根本没有超时**。原实现写 300 秒 → 用户报「二维码已失效」但自己刚授权成功，完全对不上。
- **★ 拿到 token 后不要立刻 drop state**（`tencent.py:177-188`，issue #26）。
  轮询返回 `ready` 之后还有「落盘 + 记日志」两步；若其中任一步抛错（典型是目录权限），
  前端那次请求 500、catch 静默吞掉，下一轮轮询时 state 已不在缓存 → 返回 `invalid` → 界面显示
  **「二维码已失效」**。而腾讯侧其实**已经授权成功**，用户被引导去重扫，重扫还是同样结果 ——
  真正的毛病是权限，报错信息却指向二维码。
  **判据**：state 的有效期是「发码起 N 分钟」，不是「拿到 token 就走完一生」。**交给调用方在真正落盘后再丢。**

- **state 要记 realm，并在回调时校验**（`tencent.py:38-41, 146-148`）。
  否则用户先开国内版的码、又切到国际版再轮询，会把国际版的 token 写进国内版的会话流程。
  不一致时返回 `realm_mismatch` 而**不 drop**（用户切回去还能继续用这张码）。

### 1.2 ★ 令牌刷新（这条我们最该核对）

```
POST {chat_base}/v2/plugin/auth/token/refresh
  X-Refresh-Token: <refreshToken>          ← 走头，不是 body
  X-Auth-Refresh-Source: plugin            ← 官方客户端的刷新渠道标识，缺了可能被风控当异常来源
  X-Enterprise-Id / X-User-Id / X-Device-Token（有则带）
  其余用刷新前的旧 accessToken 构造
响应：{accessToken, refreshToken?, expiresIn?, domain?}
```

三个可搬的细节（`tencent.py:370-464`）：

- **`expiresIn` 缺省时保留旧到期时间**，不要自己算。拿不到就乱写会把有效期改成错值、让面板显示误导信息。
- **`expiresIn` 要做合理性校验**（`0 < x < 10*365*86400`，且 `bool` 要排除——`isinstance(True, int)` 为真）。
- 失败文案要**说清「该重新扫码」**而不是让用户反复点刷新（12153 / session dead 时 refreshToken 也失效了）。

### 1.3 签到 / 积分 / 计费路径：**两个区域的回落方向相反**

| | CN | Global |
|---|---|---|
| chat base | `copilot.tencent.com` | `www.workbuddy.ai` |
| billing base | `www.codebuddy.cn` | `www.workbuddy.ai` |
| Origin/Referer | `www.codebuddy.cn` | `www.workbuddy.ai` |
| UA 品牌段 | `WorkBuddy` | `WorkBuddy AI` |
| **billing 路径** | 只有 `/v2/billing/meter/*` | **无 `/v2` 前缀优先**，404 回落 `/v2/...` |

- 签到：`POST {billing_base}/v2/billing/meter/daily-checkin`，body `{}`。
  **`code=10001` = 今日已签到，属正常幂等，不得当成错误中断流程。**
- 积分：`POST {billing_base}/v2/billing/meter/get-user-resource`，
  body 带 `ProductCode: 'p_tcaca'`、`Status: [0,3]`、`PageNumber/PageSize`、
  以及一个 `PackageEndTimeRangeBegin/End` 时间窗（实测是**粗过滤**，给 101 年余量）。
- **★ 国际版没有签到体系**（也没有旅行 / 活跃上报）。上游调度器对 global 账号直接过滤、不发请求。
  正确做法是**直接跳过**，而不是打过去吃一个 4xx —— 那可能被当异常行为。
  令牌保活两边都支持，不在此列。

### 1.4 ★ 积分口径的两处坑（我们若展示积分必踩）

1. **钳位要在「逐个套餐」上做，再求和**（`tencent.py:840-859`）。
   顺序不能换：先求和再钳的话，一个 `-100` 的坏套餐会从合计里扣掉 100，而上游只当它 0
   ⇒ 同一账号我们显示的余额会比上游少，用户对不上账。
   且要**双向钳**（腾讯偶发 `CycleCapacityRemain > CycleCapacitySize`，只钳负值会**高估**）。
2. **★ 套餐到期时间是 UTC+8 墙钟，与容器/本机时区无关**（`tencent.py:862-884`）。
   必须显式 `+08:00` 解析；按本机时区解析会在西半球或 UTC 机器上整体偏移数小时，倒计时跟着错。
   而**同一个函数里的时间窗**却刻意用了本机时区 —— 注释写明这不是 bug 而是照抄上游：
   窗口是 101 年余量的粗过滤（偏移无所谓），到期时刻要展示给用户对账（偏移看得见）。
   **「不要顺手统一」** —— 两处要求不同。

### 1.5 ★ 错误码表（可直接拿来用）

`tencent.py:995-1005`，并附一个真实的 Python 陷阱：

| code | 含义 |
|---|---|
| `10001` | 今日已签到（幂等成功） |
| `11101` | 上游不接受**非流式**请求（协议问题，非账号问题） |
| `11140` | request illegal → 上游**硬禁用**，到期也不自愈，必须重新登录 |
| `14017` | trial not activated（国际版新号需先做地区注册） |
| `14018` | 积分耗尽 |
| `14051` | trial 已领过（按幂等处理） |
| `12153` | 会话已失效，需重新登录 |
| `11-128` | 首条消息必须是 system |

> ⚠️ 注释里专门记了一个坑：`11-128` 这种**带横线的错误码是字符串**，字典键不加引号会被 Python
> 当算术表达式（`11-128 = -117`），于是提示永远匹配不上，真收到 `"11-128"` 时 `int()` 还会抛异常被静默吞掉。

### 1.6 模型目录：**两级取数**（只探一级会漏模型）

- 企业端点：CN `/console/enterprises/personal/models`；Global `/v2/enterprises/personal/models` → `/console/...` 回落。
- **`/v3/config`**（`tencent.py:55-60`）：官方客户端模型目录的第二级。
  **实测国际版独有的 `deepseek-v4.1-flash` / `gpt-6-astra` / `hy4-preview-f` / `kimi-k2.8-preview` 只从这里来**
  —— 用户报的「国际版没有 DeepSeek」即此。
- ⚠️ **`/v3/config` 有 UA 门禁**：只有三段式 CLI UA 能过，**web UA 被 400 拒**。
- 两路**并发**取，任一路失败降级用另一路。
- **非对话模型过滤**（`tencent.py:778-796`）：id 前缀 `nes-` / `completion-` / `codewise-`、
  `maxOutputTokens <= 256`、`tags` 含 `text-to-image` ⇒ 剔除。
  理由：**列出选不了的东西等于制造一次必然失败的尝试**（选了报 `code=11102`）。
  ⚠️ 这套规则**只适用于国内版**，套到国际版是「用国内版口径裁剪国际版」的错误。

### 1.7 国际版地区注册 + trial（一次性）

```
GET  {billing_base}/auth/realms/copilot/overseas/user/register?userId=<uid>   查是否已注册
POST {billing_base}/console/login/account                                    提交地区
POST {billing_base}/billing/ide/trial                                        领一次性 trial
```

**提交地区的三个字段取值各不相同**（`tencent.py:1081-1095`），此前错把三者都填成 IOS2：

```json
{"attributes": {
  "countryCode":    [数字地区码],   // 如 810000
  "countryFullName":[英文全名],     // 如 China Hong Kong
  "countryName":    [IOS2 短码]     // 如 HK
}}
```

地区短名单：`HK / MO / SG / TH / PH / MY / ID`。真实条目从 `/billing/area/get-country-code` 查
（**响应 `data` 是内嵌 JSON 字符串**，要多解一层）。

### 1.8 ★ 设备风控头：每账号一台固定虚拟设备（**派生公式是确定的**）

`realm.py:210-241`：

```
X-Machine-ID = sha256("wb2a:machine:" + uid).hexdigest()[:36]
X-Session-ID = sha256("wb2a:session:" + uid).hexdigest()[:36]
```

- 语义：跨重启恒定、账号间互异、同 uid 同用途恒同值。
- **实现细节刻意与上游逐字一致（固定盐 `wb2a:`、截 18 字节）** —— 两边派生的值必须相同，
  否则同一个账号在「经上游」与「直连」两条路上会是两台设备，反而制造出可被关联的异常。
- 理由：**「防多号被按设备指纹缺失/漂移关联风控」**。
- `uid` 为空时不发（匿名请求无设备可言）。
- 另有 `X-Device-Token`，三级回退：`auth.device_token` > `config.upstream.device_token` > `device_token_file`（5 分钟缓存、1KB 上限）。
  **它是凭据**：只进请求头，不得写日志、不得回显前端。

**其他风控头**（`realm.py:294-333`）：`X-CodeBuddy-Request: 1`（官方客户端风控闸门头，所有 API 请求必带）、
`Accept-Language` 按域切 `zh-CN`/`en-US`、非流式 `Accept` 收紧为 `application/json`。
出站 UA 两套：chat 域 `WorkBuddy/<ver> WorkBuddy[ AI]/<ver> CLI/<cli>`；**billing 域是单段 `WorkBuddy/<ver>`**
（官方客户端在签到这类白名单接口显式覆写 UA）。

> 注释里有一段值得学的判断方法：上游自己的登录工具至今没跟这些头，他们仍然选择跟，
> 理由是「**以官方客户端行为为参照，而不是以上游某个工具的现状为参照**」——
> 因为登录是最敏感的一步，形态不符代价最高。

### 1.9 ★ chat 路径为什么固定 `/v2/chat/completions`

`realm.py:264-276`：国际版原是 `/console` 优先、404 回落 `/v2`；上游 2026-09-18 改为**固定 `/v2`**。
原因：**`/console` 挂在腾讯云 WAF 的请求体内容规则下 —— 正文里出现反引号 / `printf` / `whoami`
这类命令执行特征会被确定性拦成 403**。用户问一句 shell 命令就中招。`/v2` 是同 base 下不挂该规则的等价端点。

> 附带一条诚实的注释：`chat_paths()` 返回列表只是**保留「多候选」的形态**，
> 当前两边都只有一个元素 ⇒ 调用方的 404 回落分支**实际不会触发**。
> 这种「别以为有回落保护」的自我提醒，值得我们在写候选路径时照做。

---

## 2. 值得搬的工程模式

### 2.1 ★★ 区域/版本隔离做成**单一推导层**（和我们「全局探测 ≠ 当前上下文」同构）

`realm.py` 把「这次请求该走哪套端点」变成一个可推导的量，而不是散落的常量。两个函数**刻意分开**：

| 函数 | 含「逃生门」？ | 用途 |
|---|---|---|
| `resolve_realm(explicit, domain)` | **否** | **落盘时用**。否则一旦开了逃生门，会把国际版账号**永久写成 cn** |
| `realm_of(auth)` | 是（`global.enabled=false` → 恒 cn） | 运行时路由 |

判定顺序：显式值优先 → 按 domain 后缀（`workbuddy.ai` / `*.workbuddy.ai`，大小写不敏感）→ 默认 cn。
**存量账号没有 realm 字段时按 domain 回退 ⇒ 升级后既有部署行为不变。**

这和我们「`_for(variant, …)` vs 无参全局探测」是同一个教训的两种写法，可以直接对照看。
**可搬的判据**：凡「会被写进持久化」的推导，**不得**经过任何逃生门/兜底开关。

### 2.2 ★★ 「取不到 ≠ 没有」= 状态诚实（state honesty）

这是他们**投入最大的一条纪律**，有专门的验收脚本 `dev/verify_state_honesty_ui.py`，
并在 1.0.74 一口气修了安全页 / 任务记录页 / 密钥页 / 设置页 / 账号页五处同类问题。
**每一处的形态都一样：取数失败被静默丢掉，界面照常渲染初始值 ⇒「不知道」被显示成「确实没有」。**

后果最严重的一例：**密钥页取数失败时显示「暂无 API 密钥」**——密钥是凭据，这句话读起来是
「我的密钥被删了」，用户会顺手点旁边的「新建密钥」重建 ⇒ 建出重复密钥，
之后「按密钥限额 / 按密钥统计用量」全都对不上。

前端抽象成两个**零运行时依赖的纯函数模块**（可在 Node 里直接跑真实实现）：

- `web/lib/async-state.ts`
  - `splitSettled`：用 `allSettled` 而非 `all`，**逐字段独立成败** —— 一份数据挂了不该拖垮其余几份。
  - `asyncFlags`：三态互斥。**`isInitialLoading` 的判据是「没有数据」而不是「请求在飞」**
    —— 有心跳轮询的页面若用后者，骨架会每隔一个心跳闪一次，用户以为页面在抽风。
  - `depMode`：★ **区分「数据上下文变了」与「查询范围变了」**，两者要求**相反**：
    - 切区域 = 换了上下文 ⇒ 屏幕上的数字属于旧区域，**必须清掉**（标题写着国际版、数字还是国内版的，比空着更误导）。
    - 翻页 / 改时段 = 只换了查询范围 ⇒ **旧内容留在屏幕上直到新数据到达**（否则每翻一页闪一次骨架）。
    - 同时变化时按「上下文变了」处理。
    > 这条和我们 `resources.ts` 的 SWR 键设计是同一个问题的两种解法。我们的键里编码 `variant` 解决了「切区域」，
    > 但**没有显式区分「查询范围」这一档** —— 值得对照检查一下我们翻页/改筛选时是否在闪骨架。
- `web/lib/account-status.ts`：账号可用性的**单一判定来源**。
  - 起因很具体：首页说「在线」、账号列表说「未加载」——同一账号两个说法。根因是两页各算各的：
    首页用了「Token 有效期」的分档 label 当「账号可用」，只要 token 没过期就显示在线。
  - **分档顺序编码了优先级，不可随意调整**：
    `disabled → disabledByPanel → manualDisabled → expired → unknown → cooling → neverSucceeded → notLoaded → online`
  - ★ **`unknown`（本次读不到上游状态）必须早于 `notLoaded`（不在池里）**：
    上游连接失败时 `/status` 仍返回 200，只是 `connected:false` 且没有账号列表；
    若不看这个标记，池就是空的 ⇒ **每个账号都被判成「不在池里」** ⇒ 界面把「连不上上游」误报成
    「账号文件坏了」，而账号页有 30 秒心跳，任何一次抖动都会命中、30 秒后又自己恢复，
    **用户会以为账号随机坏掉**。
  - **判据是「截止时间是否在未来」，不是「字段是否存在」**：前端拿到的可能是几十秒前的快照，
    字段还在、窗口已过。用于 `isDegraded`（连败降权）与模型级限流两处。
  - 「上游状态取不到」→ 如实标 `poolUnknown`，**不判断它在不在池里**。

> 另有两条踩过的坑值得记：
> ① 上游 Go 的 `omitempty` 对 `time.Time` **不生效** ⇒ 从未成功过的账号序列化成 `"0001-01-01T00:00:00Z"`，
> 在 JS 里是**真值**，判空永远不命中 ⇒ 改用 `success_count`（int64，omitempty 生效）。
> ② 跨分组合并时 **`file` 只在分组内唯一**（两个组可以有同名账号文件）⇒ React key 必须带分组，
> 否则列表少一条或状态串到别的号上，**两种都不报错**。

### 2.3 ★ 原子写 + 热加载竞态（我们写账号文件同样适用）

`tencent.py:202-246`，理由链完整，可直接对照我们的写盘路径：

- 上游每 5 秒轮询 auths 目录指纹（文件名 + mtime + 大小）热加载。
  **`write_text` 是「先截断再写」，中间存在长度为 0 的窗口** ⇒ 轮询正好落在那里会读到空文件
  ⇒ 该账号被判「已删除」从池里剔除。表现是**账号偶发短时掉线，且日志里看不出原因**。
- 用 `mkstemp` + `os.replace` 原子替换。
- **临时文件名必须唯一**（mkstemp 随机后缀），不能用固定名：同一账号并发写入时两次写会共用同一个临时文件，
  先完成者 replace 成功后文件已不存在，后完成者的 replace 失败并触发清理，**把对方刚写好的内容一并删掉**。
- 临时名以 `.` 开头且**不以 `.json` 结尾** ⇒ 既不被上游 glob 收到，也不被目录指纹计入。
- **权限要显式 `chmod 0644`**：mkstemp 固定 0600，而文件是给**另一个 uid** 读的 ⇒ 上游读不到该账号。
  注释写明「不照抄上游的 0o600，因为上游是同一进程既写又读，我们是跨 uid 写读，**前提不同**」。
- 续期写回时**必须保留未知键**（如 `device_token`），重建式写入会把设备风控凭据冲掉 ⇒ **静默降级风控形态**。
- 解析失败时**宁可不写**，也不要把坏内容覆盖到用户仅存的凭证上。

### 2.4 续期策略：按**剩余寿命**而不是按整点（`renew.py`）

- 巡检 1 小时一次，**剩余 < 3 天**才续期。
- 刻意**不重复上游的保活排程**：上游只在「有 chat 流量时」「签到前」「每天 22 点一次」三个内部时机刷新
  ⇒ 长期闲置的号会一路走到过期而无人续期（用户报的 issue #40 就是这个空档）。
- **不看上游的开关**：`keepalive_enabled=false` 是用户对**上游排程**的选择，
  而 token 过期会让账号彻底报废，属于不能放任的那类。
- 刷新失败**不做惩罚**（不冷却、不禁用）—— 续期是尽力而为的辅助动作，失败可能只是网络抖动。
- 失败日志**限频**（6 小时），并**同时落一条任务日志**（容器日志重建即丢，用户排查看的是任务页）。

### 2.5 发布包签名：CI 只构建，签名只在本机（`docs/release-signing.md`）

威胁模型讲得很清楚：**「能合并 PR 的协作者、或被钓鱼的维护者账号都能发版」**
⇒「合并恶意 PR → 发版 → 用户点更新」是一条完整链路，一次得手就是**所有部署同时沦陷**（SolarWinds 形态）。
签名把「能改代码」与「能发布可信产物」变成两件事。

```
下载包 + .sig → 内置公钥验签 → 失败：中止（不落盘、不替换、不重启）
                              → 成功：解压 → 替换 → 重启
```

可搬的几条：

- **顺序不可颠倒**：必须先有 CI 产出的 tar.gz 再对它签名；签名后重新打包 ⇒ 字节流一变签名失效 ⇒ 用户侧全部拒绝安装。
  CI 已加保护：**一旦 Release 上出现 `.sig`，就不再覆盖 tar.gz**。
- **私钥不进 CI**，理由三条（能合并 PR 的人可改 workflow 任意步骤把密钥外传；攻击面从笔记本扩大到每个环节；
  签名的全部价值来自「私钥只有你知道」）。⇒ **CI 只构建，签名只在本机**。
- 公钥要更新**两处**（更新器常量 + `.pub` 文件），**内容必须一致，有测试盯着**。
- 签名前用指纹核对密钥（`ssh-keygen -lf`）。
- 逃生门 `WB_SKIP_SIGNATURE=1` 会留 `[warn]` 日志 —— **它是逃生门，不是日常选项**。
- 第六节「诚实的边界」值得学：明确写出**挡不住什么**（挡不住本人被钓鱼、签名不覆盖代码漏洞）。
- 首次启用有**鸡生蛋问题**（已部署实例还没有验签能力，需手动更新一次），这个必须提前说清。

> 这条和我们 `tauri-nsis-release-verify` 关注的是同一件事，可以对照我们的 updater 签名链检查：
> 我们的私钥是否真的没进 CI？签名是否对着**最终产物**做的？

### 2.6 IP 与可信代理（`server/iputil.py`）

**曾有一个被实测绕过的高危漏洞**：`X-Forwarded-For` / `X-Real-IP` 都是**客户端可伪造**的普通头。
服务直接暴露时（默认监听 `0.0.0.0`），攻击者加一行 `X-Real-IP: 9.9.9.9` 就能冒充任意来源 IP，
**绕过全局 IP 白/黑名单、密钥 IP 白名单、以及登录失败按 IP 锁定**。

修法（可搬的判据）：
1. **仅当 TCP 对端落在可信代理网段内**时，才采信转发头（对端不可信 ⇒ 一律不看）。
2. `X-Forwarded-For` **从右往左**数第 N 个（N = 可信跳数）—— **右侧是反代追加的真实地址，左侧才是可伪造部分**。
3. TCP 对端地址永远可回退。
4. 存量脏数据兜底：CIDR 两侧空白要 `strip()`，否则 `ip_network(' 10.0.0.0/8')` 解析失败
   ⇒ 白名单里只要有一条这样的记录，那把密钥就对**所有**来源拒绝。

其他安全项（`docs/SECURITY-AUDIT.md`）：密码 PBKDF2-SHA256 **26 万次迭代**加盐；
登录失败按 **IP + 用户名双维度**锁定（只按 IP 可换 IP 破解同一账号）；
失败计数字典**加容量上限**（否则攻击者用大量源 IP 可持续占内存）；
每密钥滑动窗口限流；安全响应头齐全；**网关密钥只存 SHA-256 哈希，明文只在创建时展示一次**。

---

## 3. ★★ 同栈参考：`lbjlaq/Antigravity-Manager`（31805★）

它自己的 README 把它列为「管理端功能形态参考」，但它的技术栈其实和**我们**几乎一模一样：

```
Tauri v2 + React (Rust) ｜ reqwest 0.12 ｜ rusqlite (bundled) ｜ tokio ｜ axum 0.7（内嵌服务）
tauri-plugin: updater / single-instance(deep-link) / autostart / process / window-state / dialog / fs
```

对比我们的选型：Tauri + React + 内嵌 webui 服务（57890）+ 账号库 + 更新签名 —— **高度重合**。
它的模块划分可以直接当检查清单用（我们括号里是疑似对应物）：

| 它的模块 | 我们这边对应/关注 |
|---|---|
| `modules/oauth.rs` + `modules/oauth_server.rs` | **Trae OAuth 回调结果页 / 17388 回调端口** |
| `modules/device.rs` | 设备指纹（见 §1.8） |
| `modules/account.rs` / `account_service.rs` | 账号库与账号服务 |
| `modules/scheduler.rs` | 定时签到 / 保活 |
| `modules/quota.rs` | 积分 |
| `modules/migration.rs` | 数据/目录迁移（老用户数据零失效） |
| `utils/crypto.rs` | ★ **信封解密**（我们 `modules/at_rest.rs` 的同类） |
| `utils/win_shortcut.rs` | Windows 快捷方式 |
| `modules/tray.rs` | 托盘 |
| `db.rs` / `proxy_db.rs` / `user_token_db.rs` / `security_db.rs` | 本地持久化分库 |
| `proxy/`（`server.rs` / `adapters` / `mappers` / `middleware` / `rate_limit.rs` / `sticky_config.rs` / `session_manager.rs` / `monitor.rs`） | ★ **我们的网关 / 粘性会话 / 限流** |
| `modules/update_checker.rs` + `tauri-plugin-updater` | ★ **更新签名链** |
| `commands/`（`autostart` / `patch` / `proxy` / `proxy_pool` / `security` / `user_token`） | 命令层划分 |

**建议**：把它 clone 下来做一次针对性阅读，优先 `utils/crypto.rs`、`modules/oauth_server.rs`、
`proxy/rate_limit.rs` + `sticky_config.rs`、`modules/update_checker.rs` 四处。

另一个同栈线索：`linux-do/cdk`（727★，TypeScript）是他们 UI 设计令牌与浮动底栏组件的来源 ——
若我们要调前端设计令牌，这是出处。

---

## 4. 不要搬的东西 / 明确的坑

1. **架构不要搬。** 服务端集中式（systemd + 1Panel + Docker + SQLite + 多用户 RBAC + 对外反代）
   与桌面端本机工具是两个物种。它的多用户、审计、限流、Cloudflare 那套对我们全是负担。
2. **★ 它的文档会漂移，以代码为准。** 设计文档里 UA 写的是 `CLI/2.63.2 CodeBuddy/2.63.2`，
   实际实现是 `WorkBuddy/<ver> ... CLI/<cli>`；文档还写着「Python 后端 + Vue3 CDN 前端，零构建」，
   而实际交付的是 Next.js 15 静态导出。**读它的代码和注释，别读它的方案文档当现状。**
3. **不要用它的 realm 逃生门思路做落盘**（见 §2.1）—— 它自己明确警告过这点。
4. **上游不可得** ⇒ 无法复现、无法验证其行为。所有结论只能当「他人实测记录」，不能当「已验证事实」。
5. **它不处理 `$wbEncrypted` 信封**（全仓 grep 无命中）。
   因为它是操作上游 Go 网关的 `auths/*.json`，而不是桌面客户端的 storage。
   ⇒ **我们的信封解密 / `account-snapshot.json` 回落这条线，在这个项目里没有对应参考**，别指望它。

---

## 5. 逐条核实结果与处置（2026-09-29 已执行）

### 5.1 核实后判定「本来就对，不动」

| 项 | 现场 | 判据 |
|---|---|---|
| **设备指纹派生式** | `crates/buddy-switch-gateway/src/session_headers.rs` 的 `derived_device_id` | `sha256("wb2a:<purpose>:<uid>")` 前 18 字节 hex = 36 字符，与参考**逐字一致**，且有测试 |
| **积分逐包钳位** | `credits.rs::resource_summary` | 对**单个套餐**先 `.max(0.0)` 再合并 ⇒ 「先钳后求和」顺序正确，`-100` 的坏套餐不会扣掉合计 |
| **前端不闪骨架** | `src/lib/use-cached-resource.ts` | `loading` 判据是「无值**且**无错」，有旧值时后台刷新不清屏 ⇒ 参考的 `depMode` 我们已等价实现 |

### 5.2 已修复（4 处，均通过变异验证）

| # | 文件 | 问题 | 修法 |
|---|---|---|---|
| 1 | `credits.rs` | 套餐到期时间用 `Local` 解析，而腾讯给的是 **UTC+8 墙钟** ⇒ 非 UTC+8 机器上到期时刻/倒计时偏移数小时 | 固定 +08:00（`cn_wallclock_to_epoch_ms`），顺带消除 DST 缺口解析出 `None` 的静默丢失 |
| 2 | `refresh.rs` | `expiresIn` 无边界，负值/荒谬大数会写坏 `expiresAt` | `sane_relative_seconds`：正数且 < 10 年；越界与缺失同样**保留旧值** |
| 3 | `trae/store.rs` | 声称与 `config::atomic_write`「同源」实则不然：临时名固定（并发写互相删成果）+ rename 前先删目标（崩溃即真丢文件） | 对齐 config：唯一 uuid 临时名 + 不预删 + 失败清理 |
| 4 | `AccountsPage.tsx` | `showEmpty` 未排除 `error` ⇒ 取数失败渲染「未检测到客户端/未登录」，真错误一句不显示 | 加 `&& !error`，错误 Alert 补「重新检测」按钮 |

**验证方式**：每处都做了变异测试 —— 改回缺陷行为 ⇒ 用例**真的变红**且差异可解释
（时区那处差值恰为 28,800,000 ms = 8 小时；边界那处报 `Some(0) != None`）⇒ 还原 ⇒
`sha256sum` 与基线逐字一致 + 变异标记 0。

> ⚠️ **诚实边界**：本机时区是 +08:00，所以时区用例**抓不到「改回 `Local`」**（两者在此机器上相等）。
> 它能抓到的是「偏移被整个丢掉」（例如误用 `Utc`）。要在任何机器上都可证伪，得跑在非 +08:00 的机器/容器里。

**门禁**：`cargo test -p buddy-switch-core --lib` **839 passed / 0 failed / 2 ignored**（无 warning）；
`tsc --noEmit` exit 0；`check:api` / `check:store-selectors` 通过。

### 5.3 核实后判定「不盲改，仅留档」

**① `X-Auth-Refresh-Source` 的取值与缺失**

- 现状：server 侧（`upstream.rs::refresh_token`）发 `workbuddy`；**core 侧（`refresh.rs`）压根不发** ⇒ 两处形态不一致。
- 参考称官方客户端发 `plugin`。
- **为什么不改**：`git log -S` 只能追到初始提交 ⇒ 本仓取值**无出处**；且刷新在当前取值下**确实成功**
  ⇒ 服务端不以此为门禁。拿第三方未验证结论替换另一个未验证取值，**风险对等而收益不明**。
- 已做：在 `upstream.rs` 与 `refresh.rs` 两处写明分歧、双方证据、以及「要动就两端一起动且先抓包」，
  并按本仓「契约常量须有出处」的约定标注该值为**待定案**。

**② core 直连路径不带设备指纹头（`X-Machine-ID` / `X-Session-ID`）**

- 现状：只有 gateway 的 chat 路径带；core 的签到/积分/刷新路径不带。
- **为什么不改**：我们的积分路径**刻意伪装 web 客户端**（`X-Client-Platform: web`，注释说明「与用户中心 Axios 拦截器一致」）
  ⇒ 再叠一层「桌面端设备指纹」会**内部自相矛盾**。参考的建议建立在「统一模仿官方桌面端」的前提上，与我们的既有取向不同。
  要改得先确定「这批接口到底该像 web 还是像桌面端」。

### 5.4 待用户决策（涉及流程/设计，不宜擅自改）

| 项 | 事实 | 说明 |
|---|---|---|
| **更新签名私钥在 CI 里** | `.github/workflows/build.yml` 用 `secrets.TAURI_SIGNING_PRIVATE_KEY` 在 CI 内签名 | 参考的做法是「**CI 只构建，签名只在本机**」，理由是「能合并 PR 的人可改 workflow 把密钥外传」。⚠️ 但要认清边界：**CI 持有私钥时，签名只防「CDN/传输被篡改」，不防「仓库被攻陷」**（能改源码的人让 CI 签出来的包同样会被用户接受）。改成离线签名是一次**流程变更**，收益是抬高攻击门槛，代价是每次发版多一步手工签名 |
| **`X-Auth-Refresh-Source` 取值** | 见 5.3 ① | 需要一次抓包定案，两端一起改 |


---

## 附：仓库速览

- `ithtelab/workbuddy-manager` — 627★，Python 3.11+/FastAPI + Next.js 15，MIT，2026-09-11 建仓，**更新极活跃**（最后 push 2026-09-28）。
  约 13 万行（server 5.1 万 / web 4.4 万 / dev 2.3 万 / docs 1.2 万）。
- 值得一读的文件（按性价比）：`server/services/tencent.py`（协议全集）、`server/services/realm.py`（区域分派）、
  `web/lib/async-state.ts` + `web/lib/account-status.ts`（状态诚实）、`docs/release-signing.md`（签名）、
  `docs/SECURITY-AUDIT.md`（安全）、`dev/verify_state_honesty_ui.py`（验收写法）。
- 测试规模：README 称发版前全量 **1804 测试**通过 + `tsc` + 静态导出构建 + 浏览器验收跑在**生产产物**上。
  其 `dev/` 目录是「`verify_X.py`（后端）+ `verify_X_ui.py`（驱动界面）」的成对脚本，
  断言的是**用户看得见的东西**（哪些话说出来了、哪些话不许说）—— 这个验收形态值得我们借鉴。
