# API 契约校验指南

> **规则：新增或修改任何前后端 API 调用后，必须通过 `npm run check:api`。**

## 一、为什么需要这条规则

本项目的前端（TS）和后端（Rust）之间没有编译期类型检查，API 命令名是纯字符串匹配：

| 层级 | 位置 | 内容 |
|------|------|------|
| 前端调用 | `src/lib/api.ts` | `call("<cmd>")` 的字符串 |
| 前端路由表 | `src/lib/api.ts` | `ROUTES` 对象的键名 + `{ method, path }` |
| 桌面端命令 | `src-tauri/src/lib.rs` | `generate_handler!` 里登记的命令名 |
| 服务端路由 | `crates/buddy-switch-server/src/api.rs` | `.route(path, method(handler))` |

任一处漂移，症状都是「构建全绿，但某端静默失效」（桌面端 `command not found` / webui 抛「暂不支持」）。

`npm run check:api`（即 `node scripts/check-api-contract.cjs`）在构建前做一致性校验，发现差异立即以非零退出码失败。

## 二、校验的 7 条规则

1. 每个 `call("<cmd>")` 必须在桌面端 `invoke_handler` 登记
2. 每个 `call("<cmd>">`（豁免表除外）必须有 `ROUTES` 条目
3. `ROUTES` 不得有死路由（没有任何 `call` 使用它）
4. 每个 `ROUTES` 的 cmd 必须在桌面端 `invoke_handler` 登记
5. 每个 `ROUTES` 的 `path + method` 必须与 `api.rs` 路由表一致
6. 每个 `DEMO_READ_COMMANDS` 的只读命令必须有 `screenshotDemoResponse` 的 case 分支
7. 「单 Value 参数」命令：前端传的顶层键名必须与 Rust 函数签名一致

## 三、新增一个 API 端点的正确步骤

假设要新增一个 `get_user_profile` 接口，返回用户资料。

### 第 1 步：后端实现（Rust）

**a. 写 Tauri command**（`src-tauri/src/commands.rs`）：

```rust
#[tauri::command]
pub async fn get_user_profile() -> Result<Value, String> {
    // 实现逻辑
    Ok(json!({ "name": "..." }))
}
```

**b. 注册到 invoke_handler**（`src-tauri/src/lib.rs`）：

在 `generate_handler![...]` 列表中添加 `commands::get_user_profile,`。

**c. 写 REST handler + 注册路由**（`crates/buddy-switch-server/src/api.rs`）：

```rust
async fn api_get_user_profile() -> Response {
    json_ok(scheduler::run_scheduled_task(schedule::ScheduleTask::Activity).await)
}
```

在 `api_routes()` 里添加：

```rust
.route("/api/user/profile", get(api_get_user_profile))
```

### 第 2 步：前端封装（TypeScript）

**a. 写调用函数 + 注册 ROUTES**（`src/lib/api.ts`）：

```ts
export function getUserProfile(): Promise<{ name: string }> {
  return call("get_user_profile");
}

// 在 ROUTES 表里加：
// get_user_profile: { method: "GET", path: "/api/user/profile" },
```

**b. 如果它是只读命令且演示模式需要假数据**，加到 `DEMO_READ_COMMANDS`：

```ts
const DEMO_READ_COMMANDS = new Set([
  // ... 现有项
  "get_user_profile",
]);
```

并在 `src/lib/screenshot-demo.ts` 的 `screenshotDemoResponse` 里加 case：

```ts
case "get_user_profile": return { name: "Demo User" };
```

### 第 3 步：校验

```bash
npm run check:api
```

必须输出 `API 契约校验通过` 才能提交。

## 四、豁免表说明

`src/lib/api.ts` 开头的 `ROUTE_EXEMPT_COMMANDS` 列出了桌面端专属命令，它们**有意**没有 ROUTES 条目：

- `open_permission_settings` — 桌面端系统权限设置
- `check_auth_permission` — 桌面端权限检查
- `reveal_app_in_finder` — 桌面端 Finder 操作
- `relaunch_app` — 桌面端重启自身
- `get_launch_at_login_enabled` / `set_launch_at_login_enabled` — 桌面端登录项

这些函数内部自带 `isWebui()` 早退守卫，webui 不可达由函数自己保证。

**如果你的新命令也是桌面端专属，加进豁免表；否则必须同时有 ROUTES 和 invoke_handler 登记。**

## 五、常见踩坑

| 坑 | 表现 | 修复 |
|----|------|------|
| 前端改了 cmd 名，后端没改 | `check:api` 报 command not found | 两端统一 |
| 只加了 ROUTES，没加 `invoke_handler` | 桌面端 `command not found` | `lib.rs` 登记 |
| 只加了 Tauri command，没加 ROUTES | webui 抛「暂不支持」 | 补 ROUTES |
| path 写错 | `check:api` 报 path 不存在 | 与 `api.rs` 的 `.route()` 逐字对齐 |
| method 写反（GET 写成 POST） | `check:api` 报 method 不匹配 | 注意大小写一致 |
| demo 只读命令没加 case | `build:demo` 运行时炸 | `screenshot-demo.ts` 补 case |
| 单 Value 参数键名不匹配 | 桌面端 `missing required key` | 前端对象字面量键名 = Rust 参数名（驼峰化后） |

## 六、命令速查

```bash
npm run check:api          # API 契约校验（构建前置）
npm run check:store-selectors  # Store 选择器一致性校验
npm run check:npm-platforms    # npm 平台包版本一致性校验
npm run build              # 全量构建（含以上三项校验 + vite build）
cargo check                # Rust 编译检查
cargo test                 # Rust 测试
```
