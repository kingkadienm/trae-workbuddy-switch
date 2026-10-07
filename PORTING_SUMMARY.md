# agentpoints → trae-workbuddy-switch 移植总结

## 移植范围
将 `trae-workbuddy/agentpoints/` 下 `workbuddy2api-panel` 和 `autoclaw2api` 的缺失功能移植到 `trae-workbuddy-switch`。

## 完成模块

### P0（7 项，全部完成）
1. **Growth Task Center** (t_4bd4f876 + t_82ff09bc)
   - Rust 模块：`crates/buddy-switch-core/src/modules/growth/mod.rs`（596 行）
   - API 路由：`/api/growth/tasks` GET、`/api/growth/tasks/accept` POST、`/api/growth/tasks/run` POST
   - 前端页面：`src/pages/GrowthTaskCenterPage.tsx`（268 行）
   - 路由：`/growth`，导航：`nav.growth`
   - 域词表：`src/locales/domains/growth.{zh,en}.ts`

2. **Model & Tier Catalog** (t_5cc36377)
   - 前端页面：`src/pages/ModelCatalogPage.tsx`（184 行）
   - 路由：`/model-catalog`，导航：`nav.modelCatalog`

3. **Account Pool Visualization** (t_802bb684)
   - 前端页面：`src/pages/AccountPoolPage.tsx`（128 行）
   - 路由：`/account-pool`，导航：`nav.accountPool`

4. **Request Log Archive** (t_f8176232)
   - 前端页面：`src/pages/RequestLogPage.tsx`（129 行）
   - 路由：`/request-logs`，导航：`nav.requestLogs`

5. **Streak Manager + Lottery** (t_ab338a72)
   - 后端：统一 `/api/activity/streak` 和 `/api/activity/lottery` 到 `api_activity_run`
   - 前端页面：`src/pages/ActivityPage.tsx`（48 行）
   - 路由：`/activity`，导航：`nav.activity`

6. **Config Hot-Reload Editor** (t_eea5ee8f)
   - 前端页面：`src/pages/ConfigEditorPage.tsx`（144 行）
   - 路由：`/config-editor`，导航：`nav.configEditor`

### P1（3 项，全部完成）
1. **OAuth Account Onboarding** (t_d91a4174)
   - 已有 `OAuthLoginDialog` / `TraeOAuthLoginDialog` 实现 device-code 流程，无需额外页面

2. **Batch Account Operations** (t_4035426a)
   - 前端页面：`src/pages/BatchAccountOperationsPage.tsx`
   - 功能：多选账号、批量删除/切换/导出（复用现有单条 API）
   - 路由：`/batch-accounts`

3. **Unified Token/Credit Stats Dashboard** (t_1e5df9c8)
   - 前端页面：`src/pages/UnifiedStatsPage.tsx`
   - 功能：WorkBuddy credit stats + Trae token stats 并排展示
   - 路由：`/unified-stats`

## 技术规范
- 前端新页面：懒加载注册、域分割词表、统一 `useT` 国际化钩子
- 后端新增模块：遵循现有 `modules/` 结构与编码风格
- API 路由：遵循现有 `api.rs` 路由格式
- 构建状态：Rust 后端 `cargo build` 通过；前端 `npm run build:fast` 通过；`npx tsc --noEmit` 无错误

## Kanban 状态
- Board: default
- 本次移植相关工单：10 个（P0×7 + P1×3）全部 done
- 剩余工单：早期 backend-test 任务，与本次移植无关

## 经验教训（已存入 skill）
- Backend API existence check：前端批量页面前先确认后端是否有 batch API
- Existing-component discovery：实现功能前先搜索已有组件，避免重复造轮子
- Duplicate routes：添加路由后立即 grep 检查重复
- TypeScript LSP staleness：构建通过即视为正确，不追逐 LSP 假阳性
- Complete with evidence：完成工单前添加证据注释，再传递摘要
