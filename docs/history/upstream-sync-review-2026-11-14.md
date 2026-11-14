# 2026-11-14 上游同步独立审查补充：开发 API 代理同源回归

> 状态：只读历史（不再更新）。记录合并候选 `fa72de1c045083e0358bb0d0bce722daf3bd7551` 被独立审查指出的 P2、此次修复与本地验证；最终复审、提交 SHA 和推送结果以交付回执为准。

原有96个冲突的三方比较和取舍见[上游同步报告](upstream-sync-2026-11-14.md)。本补充不改写原报告中的历史验证结果。

## 问题与取舍

合并后的前端管理请求使用同源 `/api` 地址。浏览器经 Vite 开发服务器发送 POST 时，`Origin` 是前端监听地址，例如 `http://127.0.0.1:<前端端口>`。原 `vite.config.ts` 的 `/api` 代理配置为 `changeOrigin: true`，只把 `Host` 改成 Rust 后端地址，`Origin` 仍保持浏览器的原值。

上游新增的 `crates/buddy-switch-server/src/api_guard.rs::judge` 要求有 `Origin` 的管理请求与 `Host` 同源。因此两个不同端口触发 `CrossOrigin`，登录、保存等开发 WebUI 管理 POST 返回403。TypeScript、生产构建及原有 Rust 测试没有经过这条实际开发代理链，不能发现此问题。

修复只把 `/api` 的 `changeOrigin` 改为 `false`，保留浏览器的回环 `Host` 和原 `Origin`。后端仍接受回环主机，生产来源校验、TOTP 管理面保护和 `/v1` 模型协议代理保持原有逻辑。没有通过放宽 `api_guard`、伪造任意来源或移除检查器来使测试通过。

## 可重复的真实代理回归

新增 `scripts/check-vite-api-proxy.mjs`，运行方式：

```sh
# 需要先完成 npm ci，并缓存本仓 Cargo.lock 的依赖（例如 cargo fetch --locked）。
node scripts/check-vite-api-proxy.mjs

# 仅在测试进程中恢复旧配置；预期失败，证明回归能识别同一问题。
node scripts/check-vite-api-proxy.mjs --rewrite-host
```

脚本加载真实 `vite.config.ts` 的 `/api` 配置，使用 Vite 的实际代理中间件转发本地 HTTP 请求。后端夹具直接调用当前 `api_guard.rs` 编译的 Rust `judge`，没有复制同源判定逻辑；临时 Cargo crate 复用本仓锁文件和 Cargo target 缓存，不依赖某个本机缓存文件 hash 或预编译探针。两个监听端口均由系统分配。

正向用例覆盖 `/api/auth/totp/validate` 和 `/api/relay/config` 两个管理 POST，断言真实 Rust 判定、HTTP状态以及实际 Host/Origin 一致。夹具只执行来源校验，不执行真实 TOTP 登录、配置落盘或模型请求。脚本只清理自身生成的明确临时文件及空目录，不递归删除目录。

| 实际命令与配置 | 退出码 | 实际结果 |
| --- | --- | --- |
| 原候选配置执行独立审查复现脚本 | 1 | 真实代理后 Host/Origin 不同，Rust `CrossOrigin`，HTTP403 |
| 修改前运行 `node scripts/check-vite-api-proxy.mjs` | 1 | 同样得到真实 Rust 拒绝，证明仓库回归确实能检出问题 |
| 修改后运行同一命令 | 0 | 两个管理 POST 均为 HTTP200 / `accepted`，实际 Host/Origin 一致 |
| 修改后追加 `--rewrite-host` | 1（预期失败） | 仅内存恢复旧配置，再现 HTTP403 / `CrossOrigin` |
| `npm run build` | 0 | TypeScript、API契约、337项scope检查、13套原型、51项单测及生产Vite构建通过 |
| `git diff --check` | 0 | 本次差异无新增空白错误 |

回归是独立开发检查，没有修改 `package.json` 或已独立批准的前端、网关代码。原候选的独立 Rust 组合测试为2763通过、0失败、21忽略，Tauri check退出0；此次未改任何 Rust 源码，不把这些前轮结果冒充为新执行。

## 反向代理部署的升级配置提示

此次只同步源码，没有修改线上环境、部署脚本或 Nginx 配置。上游管理面来源保护仍然生效：通过非回环 `Host` 访问 `/api` 时，需要在服务端环境变量 `BUDDY_SWITCH_ALLOWED_HOSTS` 中显式列出实际管理面主机，例如：

```sh
BUDDY_SWITCH_ALLOWED_HOSTS=api.wangzs.vip
```

`api.wangzs.vip` 是 fork 历史配置中的域名示例；应按实际转发给后端的管理面 `Host` 填写，而不是照搬模型接入域名。可以填主机名或精确的 `host:port`，多个值逗号分隔；不填协议、路径或通配符。反向代理还应保留浏览器所见 Host，使其与 Origin 同源。这项允许列表只解决显式主机授权，不取消同源、JSON或TOTP校验。

## 冻结边界与未执行项

已批准前端范围 `src/**, package.json, package-lock.json` 的指纹仍为 `272738d553c64ea5896fe5d823e36f07ddbdeec880ef911d3e52f97d0a5d3ee6`；网关范围仍为 `5574be37ff16b588bc8c25664681214cee22e0b8ff983b1990031639027d36fc`。

未进行真实浏览器或原生桌面逐入口点击、真实账号登录、真实模型调用、跨平台安装、签名发布或线上部署。原型套件仍有9项人工验证未执行；Vite仍有大chunk及演示夹具重复分支提示。此次回归不能当作上述验收。
