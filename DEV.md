# 本地开发调试

## 环境要求

- Node.js >= 18
- Rust >= 1.75
- 后端默认端口：`57890`

## 启动步骤

### 1. 启动 Rust 后端

```bash
cd crates/buddy-switch-server
cargo run -- --port 57890
```

后端会在 `http://127.0.0.1:57890` 启动，提供：
- API 服务：`http://127.0.0.1:57890/api/*`
- 网关服务：`http://127.0.0.1:57890/v1/*`
- Web UI（构建后）：`http://127.0.0.1:57890/`

### 2. 启动 Vite Dev Server（Web UI 热更新）

```bash
npm run dev
```

Vite 会在 `http://localhost:1420` 启动，自动代理 API 请求到后端：
- `/api/*` → `http://127.0.0.1:57890/api/*`
- `/v1/*` → `http://127.0.0.1:57890/v1/*`

### 3. 打开浏览器

访问 `http://localhost:1420`，Web UI 会自动加载并连接后端 API。

## 工作原理

```
浏览器 (localhost:1420)
    │
    ├── /api/* ──→ Vite proxy ──→ Rust 后端 (127.0.0.1:57890)
    │
    ├── /v1/* ──→ Vite proxy ──→ Rust 网关 (127.0.0.1:57890)
    │
    └── 静态资源 ──→ Vite HMR (热更新)
```

## 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `HTTPS_PROXY` | 网关出站代理（ clash 等） | 空 |
| `VITE_DEMO_MODE` | 演示模式（返回假数据） | 0 |
| `TAURI_DEV_HOST` | Tauri 开发主机 | 空 |

## 常见问题

### 端口冲突

如果 `1420` 或 `57890` 被占用：

```bash
# 修改 Vite 端口
vite.config.ts → server.port

# 修改 Rust 后端端口
cargo run -- --port 9999
# 同时修改 vite.config.ts proxy 配置
```

### CORS 错误

确保 Vite proxy 配置正确，`/api` 和 `/v1` 都代理到了 Rust 后端。如果仍有跨域，检查 Rust 后端是否正确启动。

### 网关请求超时

如果服务器开启了代理（clash），设置环境变量：

```bash
export HTTPS_PROXY=http://127.0.0.1:7897
cargo run -- --port 57890
```
