# 无客户端部署：登录授权与网关可用性（issue #62）

> 状态：**已落地**（2026-10-20，前端 + 后端命令）。仅剩前端组件测试基建（§4 第 12 项）。
> 原型：[`../prototype/`](../prototype/)；门禁：`docs/prototype/verify-clientless.cjs`。
> 缘起：用户在服务器上部署（`/root/...`），本机没装任何平台的客户端。账号页只给「重新检测 / 从本机导入」两条路 —— 登录不了，网关也就没有上游账号可用。

## 1. 问题定性

不是「少一个入口」，是**少了一个维度**：旧设计把「没装客户端」当成异常态（未检测到 → 劝你去装），而服务器 / 容器部署里它是**正常形态**。

客户端提供的只有三件事：登录态来源、程序位切换、本地会话与用量文件。**账号与网关只依赖凭据本身**，与客户端是否同机无关。把这两类能力混在一张「未检测到客户端」的红字卡片里，等于告诉服务器用户「你用不了」。

判据用 `scope.installed === false`（本机没这个客户端），**不引入「是不是服务器」的新概念** —— 桌面用户也可能只装了其中一个平台，两种情形 UI 与行为完全一致。

## 2. 三家可行性（实测结论，不是猜测）

| 平台 | 授权形态 | 无客户端能否登录 | 无客户端能否续期 | 依据 |
|:--|:--|:--|:--|:--|
| WorkBuddy | `auth/state` + 服务端轮询 | ✅ 能 | ✅ 能 | 只要 `refresh_token`，无设备证明（`modules/oauth.rs`） |
| Qoder | 设备授权 PKCE + 轮询 | ✅ 能 | ✅ 能 | 设备指纹本程序自造（`qoder/oauth.rs` / `qoder/device.rs`） |
| Trae | 授权页回调 + PKCE | ❌ 不能 | ❌ 不能 | 换 token 与续期都要客户端 `storage.json` 里的 icube **EC P-256 私钥**（`trae/icube.rs`）；且授权页要探 `127.0.0.1:17388` |

**Trae 的两条现实路径**（原型里已如实写明，不许画「扫码即可」的饼）：

1. 把装有客户端那台机器的 `storage.json` 拷到服务器同一路径 —— 之后授权 + 续期都正常（授权页仍需 `ssh -L 17388` 或手工粘贴回调链接）；
2. 不在这台机器登录，用「导入备份」搬已有账号 —— 但备份只有 `device_bindings`（uid → deviceId），**不含私钥**，续期会失败，只能用到 `access_token` 过期。

> 想让 Trae 真正「零客户端」：需要自举设备身份（自建 EC 密钥对 + 走通上游 enroll）。那是独立 spike，不在本次范围，见 §6。

## 3. 交互设计（原型已落地）

四处表达，一个核心组件：

| 位置 | 无账号 | 有账号 |
|:--|:--|:--|
| 账号页 | **引导卡**：三条可用路径 + 能力矩阵（可用 / 需要客户端 两列） | **细条**：一句说清「什么受影响、什么不受」 |
| 添加账号抽屉 | 四条路并存；「从本机导入」标注不可用但**不禁用**，点了给替代 | 同左 |
| 授权弹窗 | **一套 UI**：链接 + 二维码 + 倒计时 + 轮询状态；桌面形态只额外帮你开一次浏览器，失败不报错 | — |
| 网关页 | 常驻说明条：网关只依赖池里的凭据，与客户端无关 | 同左 |

授权弹窗是这次的核心：**弹窗本身就是完整路径**，不依赖 `openInBrowser`（服务器上必然失败，旧实现整个流程就断在这里）。平台差异只有一处：Trae 在弹窗顶部先给「设备私钥 + 17388」限制块与两条路径。

原型：`docs/prototype/index.html?scope=trae-global&dest=accounts`（无客户端形态）、`&open=auth`（授权弹窗）、`&dest=gateway`（网关说明）。验收：`node docs/prototype/verify-clientless.cjs`（18 条，含 3 个变异）。

## 4. 落地清单

> 图例：✅ 已落地 · ☐ 未开始（后端）。



### 后端

1. ✅ `oauth_start` 的 `expiresIn` 三个平台都已返回（WB / Trae / Qoder 原本就有，前端只是开始用它做倒计时）。
2. ✅ 新增命令 `accounts_paste_credential(scope, text)`（注意是复数 `accounts_` 前缀，与 `accounts_import_local` 同族）：core `auth_file::import_account_from_text_for` 复用文件导入的解析 / 解锁链，落库走 `save_collected_account_for`。只接 WorkBuddy（另两家各有 PAT / JWT 入口，硬编码返回指引）。：按平台解析粘贴内容（WB auth JSON / Qoder PAT / Trae refresh_token 或回调链接），写库前做一次 `verify`，失败回原因为结构化错误码。两端同步：`api.ts` ROUTES + `call()`、`generate_handler!`、server `.route`、`commands.rs`；改完跑 `scripts/check-api-contract.cjs`。
3. ✅ Trae 新增 `trae_oauth_submit_callback(loginId, callbackUrl)`：core `oauth::submit_callback` 与监听路径共用 `parse_callback` / `verify_login_trace` / `perform_login`；成功后通知监听任务收工释放 17388。**没有会话就不兑换**（重启后贴旧链接会明确报「不存在或已过期」）。：解析整串回调地址（`authCodeInfo` + `userInfo` + `host` + `userRegion` + `loginTraceID`），复用现有 exchange 路径；**不回显回调 URL**，插值全转义。
4. ✅ 三个弹窗的 `openInBrowser` 都改成 best-effort（`.catch(() => {})`），失败不再算登录失败。

### 前端

5. ✅ 三页按 `installed` 分流：新组件 `components/clientless-notice.tsx`（`ClientlessEmptyCard` 引导卡 / `ClientlessStrip` 细条 / `GatewayClientlessNotice` 网关条）。WB / Trae 换空态卡；Qoder 保留来源四态诊断、加细条。
6. ✅ 新增 `components/auth-link-panel.tsx`（真二维码，`qrcode` 依赖），WB / Qoder / Trae 三个弹窗全部换用；Trae 去掉自建倒计时，改为单一 `setTimeout` 判超时。，`openInBrowser` 降级为 best-effort；Trae 增限制块与回调粘贴框。二维码用 `qrcode` 类库（原型里是示意点阵，**不参与交付**）。
7. ✅ `local` 不置灰、点了给替代（三页各一句 toast）；WB 抽屉新增「粘贴授权凭据」一档 + 空态卡第三张卡；Trae 弹窗加「粘贴回调链接」输入框。，点击给出替代路径（沿用「未检测到不禁用、可点后解释」的既有约定）。
8. ✅ 三个网关页统一插 `<GatewayClientlessNotice scope>`，文案同源。
9. ✅ 全部走 `shared.authPanel.*` / `shared.srv.*` / `trae.comp.oauth.tunnel.*`，中英双语。

### 文档与门禁

10. ✅ `docs/guides/DOCKER.md`：能力表补「浏览器授权登录 ✅」与「会话/用量 ❌」，新增「容器里怎么登录（没装任何客户端）」一节。：把「从本机客户端导入 ❌」改为「❌ + 替代路径（浏览器授权 / 粘贴凭据 / 导入备份）」，并补一节「容器里怎么登录」。
11. ✅ `docs/README.md` 两处索引（任务表 + 待拍板方案表）。
12. ☐ 前端测试基建（本仓无组件测试框架）—— 暂以 `tsc` + `vite build` + 六项门禁兜底。

## 5. 不做什么

- **不给 Trae 假装「扫码即可」**：设备私钥拿不到就是拿不到，UI 必须提前说清，而不是让用户走到最后一步看 20403。
- **不改 `no_credential` 的既有语义**：网关侧仍按 cause 区分「认证文件缺失 / 账号库空 / 指定账号不存在 / 池内全不可用」。
- **不引入「运行形态」开关**：判定只看 `installed`，不新增用户可见的设置项。

## 6. 遗留 spike（不在本次）

Trae 设备身份自举：自建 EC P-256 密钥对 → 找到并走通上游设备 enroll → 把私钥存进账号库（加密落盘，遵守 `icube.rs` 的脱敏红线）。成功则 Trae 也能纯浏览器登录；失败则维持 §2 的两条路径。

## 7. 变更记录

- **2026-10-20** 后端命令落地：`accounts_paste_credential` / `trae_oauth_submit_callback`，
  四处接线齐全（`api.ts` ROUTES + `call()`、`generate_handler!`、server `.route`、命令本体），
  `check:api` 由 221/204/231 → **223/206/233**。新增 core 单测 6 条（粘贴凭据 3 条 + 手工回调护栏 3 条），
  `cargo check` 三个 crate 全过。
- **2026-10-20** 前端落地（WB / Trae / Qoder 三平台的授权弹窗、空态、细条、网关条、抽屉不再置灰）。
  新增依赖 `qrcode@1.5.4` + `@types/qrcode`（纯 JS，无原生模块，仅用于把授权链接渲成二维码）。
  验收：`tsc --noEmit` 对本次改动文件零报错；`vite build` 通过；`check:api` / `check:docker` /
  `check:store-selectors` / `check:npm-platforms` / `check:agent-sources` / `check:pages-base` 全绿；
  原型门禁 `verify-clientless.cjs` 18/18 + 3 变异。
  未做：`npm run build` 整体无法通过 —— 工作区里另一位并发作者的 `cache-section.tsx` 引用了
  `api.cacheScan` / `api.cacheClear` 而 `api.ts` 尚未落地，与本改动无关。
