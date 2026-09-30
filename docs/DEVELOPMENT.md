# 开发指南

## 环境要求

Node.js ≥ 20、Rust stable、macOS（或 Windows/Linux）。

## 开发命令

```bash
npm install
npm run tauri dev        # 开发模式
npm run build:app        # 构建 debug .app（含前端资源补丁）
npm run build:app:release  # 构建 release .app + 签名更新包
```

## 发布新版本

签名密钥（自动更新用）存放于 `~/.buddy-switch/buddy-switch-updater.key`，构建脚本通过
`TAURI_SIGNING_PRIVATE_KEY` 注入。发布新版本时：

1. `npm run build:app:release` 生成 `.app.tar.gz` + `.sig`（Windows NSIS 构建会额外生成当前版本的 `*_x64-setup.exe` + `.exe.sig`）。CI 会先清掉 `target/**/release/bundle`，避免 cargo cache 把旧安装包带进 Release。
2. macOS：`UPDATE_OS=macos UPDATE_ARCH=aarch64 sh scripts/gen-update-json.sh` 生成 `latest-macos-aarch64.json`；Intel 用 `UPDATE_ARCH=x86_64`
3. Windows：`UPDATE_OS=windows UPDATE_ARCH=x86_64 sh scripts/gen-update-json.sh` 生成 `latest-windows-x86_64.json`
4. `python3 scripts/merge-update-manifests.py <产物目录>` 合并为 `latest.json`，并把 Windows 平台项写入 `latest-macos-x86_64.json`（兼容已安装的 Windows 客户端）
5. 将安装包、签名更新包、`latest*.json` 一并上传到 GitHub Release

### npm 版（webui）发布

主包与平台分包统一在 **`@kingkadienm` scope** 下：主包 `@kingkadienm/buddy-switch`，
平台包 `@kingkadienm/buddy-switch-<platform>-<arch>`（**目录名不带 scope**，
仍是 `npm/platform/buddy-switch-<tag>`，`build.yml` 的 `PKG_DIR` 与生成脚本都按目录名拼路径）。
安装后的**命令名仍是 `buddy-switch`**（`bin` 字段决定，与包名无关）。

1. 编译 server 二进制并上传 GitHub Release（`.github/workflows/build.yml` 自动执行）
2. 先 `sh scripts/gen-platform-packages.sh <版本>` 生成 5 个平台包，把对应二进制放进各包 `bin/` 后逐个 `npm publish --access public`
3. `cd npm && npm publish --access public`（主包，`postinstall` 从已安装的平台包复制二进制）

> ⚠️ **发布前提**：npm 账号必须拥有 **`kingkadienm` 这个 scope**（用户名即为 `kingkadienm`，
> 或在该账号下创建同名 organization）。scope 不属于自己时 `npm publish` 会 403，
> 且 `@kingkadienm/*` 是别人无法代持的命名空间。
>
> 另外：`buddy-switch`（不带 scope）这个包名**在 npm 上已被他人占用**，所以不能再退回无 scope 命名；
> 本项目此前的发布用的是 `workbuddy-switch`。
> scoped 包必须带 `--access public`，否则会以私有包发布（私有包需要付费账号）。
>
> `npm/package.json` 的 `optionalDependencies` 列了 5 个平台（darwin-arm64 / darwin-x64 /
> win32-x64 / linux-x64 / linux-arm64），与 CI 矩阵**一一对应**，改动任一侧都要同步另一侧
> （`stamp-version.mjs` 会把它们的版本号统一重写，但**不会**替你补漏掉的平台）。

#### linux-arm64 是「CLI only」

CI 矩阵里 `linux-arm64` 的 `bundles` 为空 ⇒ 跳过 `Build desktop app`，`update_arch` 为空 ⇒
跳过 updater 清单。**它只产出 CLI 裸二进制**（npm 平台包 + Release 资产），**没有桌面 App**。

原因：桌面 App 走 Tauri，需要 webkit2gtk；在 x64 runner 上交叉编 arm64 版要开 dpkg 多架构并装
`libwebkit2gtk-4.1-dev:arm64`，代价高且易碎。而 CLI 是纯 Rust（reqwest 用 rustls、rusqlite 走
`bundled`），交叉编译很干净，只需：

- `apt install gcc-aarch64-linux-gnu`
- job 级 env：`CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc`
- job 级 env：`CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc`
  （**不能省**：`libsqlite3-sys` 用 `cc` 现场编译 SQLite 的 C amalgamation，
  不指定交叉 C 编译器就会拿 host gcc 产出 x86_64 目标文件，链接期报 `file in wrong format`）

⚠️ 若以后要给 linux-arm64 加桌面 App，改的不止是 matrix：`Build desktop app` 的
`if: matrix.bundles != ''` 会放行，随后 webkit2gtk 的 arm64 依赖必须真的装上。

#### npm 通道开关：`PUBLISH_NPM`（**当前关闭**）

CI 的两个发布 job（`publish-platform` / `publish-main`）与「是否把 CLI 裸二进制留在 Release 里」
都由**同一个仓库变量**控制：Settings → Secrets and variables → Actions → **Variables** → `PUBLISH_NPM`。

| `PUBLISH_NPM` | npm job | Release 里的 `buddy-switch-*` 裸二进制 |
| --- | --- | --- |
| 未配置 / 非 `true`（**当前**） | skipped（不红，不再需要 `NPM_TOKEN`） | **保留** —— webui 形态唯一的下载渠道 |
| `true` | 正常运行（需要 `NPM_TOKEN`） | 剔除（已随平台包发到 npm，Release 只留桌面 App） |

**为什么要有这个开关**：npmjs 账号注册/访问受限期间拿不到 token，此前每次 tag 都会红 4 个
`npm platform *` job、`publish-main` 被 skip，而 Release 与安装包本身一直是 success ——
那是噪音不是事故，却会让人误判「这次发版挂了」。恢复发布时**只改变量**，不用改 workflow。

**关闭期间的 webui 分发方式**：从 GitHub Releases 下载对应平台的裸二进制直接跑
（`buddy-switch` / `buddy-switch serve --port 57890` / `status` / `version`）。
macOS 未签名会触发 Gatekeeper，先 `xattr -d com.apple.quarantine <路径>`；Windows 会有 SmartScreen 提示，选「仍要运行」。

### 在线演示（GitHub Pages）部署前提

`.github/workflows/pages.yml` 在每次 push 到 `main` 时构建只读演示并部署。
**首次部署前必须先手动启用 Pages**：仓库 Settings → Pages → Source 选 **GitHub Actions**。

未启用时该工作流会**恰好失败在 `Configure Pages` 这一步**（前面的 `npm ci` 与
`npm run build:demo` 都是通过的，容易误判成构建坏了），并且此后每次 push 都会留一条红的 run。

> **不要试图用 `enablement: true` 绕过这一步**：`actions/configure-pages` 的文档明确要求该选项
> 使用 `GITHUB_TOKEN` **以外**的 token（PAT 的 `repo` scope，或 GitHub App 的
> `administration:write` + `pages:write`），加了不但仍然失败，还会平白引入一个密钥依赖。

## 目录结构

```
src-tauri/
  src/
    commands.rs      # Tauri command 薄包装（对应 Python 版 HTTP API）
    modules/         # 已抽离到 crates/buddy-switch-core（三宿主复用）
crates/
  buddy-switch-core/    # 核心逻辑：account/auth_file/oauth/process/switch/session/checkin/refresh/update/config
  buddy-switch-server/  # HTTP server + CLI：axum API + rust-embed 前端
src/                 # 前端：components/pages/lib（api.ts 双通道：Tauri invoke / HTTP fetch）
npm/                 # npm 包：package.json + bin + scripts/install.js
```

## 隐私注意事项

- 仓库不提交本地数据（accounts.json、认证文件、密钥、token 由 `.gitignore` 排除）
- 发布前用 `git grep` 扫描 token 模式（`ghp_`/`npm_`/`gho_` 等）
