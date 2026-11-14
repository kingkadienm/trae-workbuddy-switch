//! 模型降级链的「hop 循环」骨架 —— 给 Trae / Qoder 两条**自有中继**用。
//!
//! ## 为什么 WB 不走这里
//!
//! WorkBuddy 的中继（[`crate::routes::relay`]）自己实现了 hop 循环：它要精确控制
//! **探针票**（按 hop 归还）、**配额**（整轮只判一次）、**账本模型**（记实际使用的那个）。
//! 那些逻辑与 WB 的治理层耦合得很深，抽不出来 —— 抽出来只会得到一个谁都不敢改的
//! 「通用中继」。
//!
//! Trae / Qoder 的中继是**各自实现**的（[`crate::trae::routes`] / [`crate::qoder::routes`]），
//! 不经过 WB 的池与账本。本模块只提供**与平台无关的那部分**：算 hop 序列、改写请求体里的
//! 模型名、跳过被禁用的目标、判断这次失败能不能降级、往成功响应上写降级头。
//!
//! ## 判定比 WB 粗一档（**明确的取舍**）
//!
//! WB 按**上游事件类型**判（6004 / 11102 / 闸门 / 5xx …）；这里只能看**响应状态码**：
//! 5xx ⇒ 服务端故障，429 ⇒ 限流。其余一律不可降级。
//!
//! 代价：429 无法区分**模型级**与**账号级**限流（这两条平台不把上游错误分类暴露给调用方）。
//! 账号级 429 降到别的模型多半还是 429 —— 白跑一跳。要按事件判，就得先改两条平台自己的
//! 错误体系（`TraeErrKind` / `QoderErrKind`），那是另一轮工程。
//!
//! ## 三条不破的边界
//!
//! 1. **已开始的流不回退**：降级只发生在「这一跳彻底失败、还没发过响应体」的时候。
//! 2. **禁用的目标直接跳过**，不发请求。
//! 3. **备选只在本作用域内选**：跨作用域 = 换账号池 + 换 wire 协议 + 换计费，
//!    那是统一网关的职责。

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::Response;
use buddy_switch_core::modules::product::Scope;
use tokio::sync::RwLock;

use crate::disabled_models::DisabledModels;
use crate::model_fallback::{self, Trigger};
use crate::routes::relay::{apply_fallback_headers, FallbackInfo};

/// 一次请求的降级 hop 序列。
pub struct FallbackLoop {
    scope: Scope,
    /// 客户端原本请求的模型（降级头里的 `from`）。
    origin: String,
    /// `[主模型, 备选1, 备选2…]`（每项带自己的作用域）。
    plan: Vec<crate::model_fallback::FallbackTarget>,
    /// 链的**触发条件白名单**（用户配的 `triggers`）。
    ///
    /// ★ 必须带上：只看状态码会**无视用户配的白名单** ——
    ///   用户没勾 5xx 也照样降级，而基线三档（闸门 / 6004 / 11102）在这两条线上
    ///   一个都命中不了 ⇒ 用户按基线配好的链是死的，卡片上还显示着它。
    triggers: BTreeSet<Trigger>,
}

impl FallbackLoop {
    /// 按作用域与主模型算出 hop 序列与触发条件白名单。
    ///
    /// 没有链（或链里没有有效备选）时，`plan` 只有主模型 —— 调用方据此走原路径。
    pub async fn plan(scope: Scope, origin_model: &str) -> Self {
        let fb = model_fallback::shared();
        let guard = fb.read().await;
        let mut plan = model_fallback::plan_for(&guard, scope, origin_model);
        // ★ 同平台跨区域**总是允许**；跨平台受**嵌套深度**限制。
        //
        //   同平台跨区域（`trae-cn` → `trae-global`）：只换 region / variant，协议一致。
        //   跨平台（`trae-cn` → `wb-cn`）：调 `cross::relay_on_scope`，它会检查
        //   `cross_depth()`。**这里先过滤一次**，是为了「不让用户白等一次必然被拒的请求」：
        //   已经跨出去过（`cross_depth() >= MAX`）时，跨平台目标不该进入 plan，
        //   否则会先发出去、再拿到 502。
        plan.retain(|target| {
            let target_scope = target.scope_or(scope);
            target_scope.product() == scope.product()
                || crate::routes::cross::cross_depth() < crate::routes::cross::MAX_CROSS_DEPTH
        });
        let triggers = model_fallback::chain_of(&guard, scope, origin_model)
            .map(|chain| chain.effective_triggers())
            .unwrap_or_default();
        Self {
            scope,
            origin: origin_model.to_string(),
            plan,
            triggers,
        }
    }

    /// 一共几跳（含主模型）。
    pub fn len(&self) -> usize {
        self.plan.len()
    }

    /// 客户端原本请求的模型（降级头里的 `from`）。
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// hop 序列（`[主模型, 备选1, 备选2…]`）。
    pub fn models(&self) -> Vec<String> {
        self.plan
            .iter()
            .map(|target| target.model.clone())
            .collect()
    }

    /// 第 `hop` 跳的模型名（`hop = 0` 是主模型）。
    ///
    /// ★ 越界返回 `None`：**不回落到主模型**。
    ///   回落的后果是「备选全被禁用时，把主模型原样重发一次」——
    ///   客户端收到一个「降级到 A → A」的头，而实际只是重试了一次主模型。
    pub fn model_at(&self, hop: usize) -> Option<&str> {
        self.plan.get(hop).map(|target| target.model.as_str())
    }

    /// 第 `hop` 跳实际要去的作用域（`None` 时取入口作用域）。
    pub fn scope_at(&self, hop: usize) -> Option<Scope> {
        self.plan.get(hop).map(|target| target.scope_or(self.scope))
    }

    /// 第 `hop` 跳的**完整目标**（含作用域）。
    ///
    /// Trae / Qoder 侧的跨作用域执行要它：同作用域换号与跨作用域换 relay 是两件事。
    pub fn target_at(&self, hop: usize) -> Option<&crate::model_fallback::FallbackTarget> {
        self.plan.get(hop)
    }

    /// 这一跳要用的请求体。
    ///
    /// 返回 `None` = **这一跳不能降级**（body 里没有 `model` 字段，改写不了）。
    ///
    /// ★ 这不是保守，是必需：body 缺 `model` 时中继会用它自己的默认值（配置默认模型
    ///   或清单第一条）发请求，而我们**不知道**那是哪个 —— 改写不成就会「每跳发同一个模型，
    ///   却告诉客户端降到了备选」，那是**对客户端撒谎**。
    pub fn body_for(&self, hop: usize, raw: &str) -> Option<String> {
        if hop == 0 {
            return Some(raw.to_string());
        }
        if !crate::model_route::body_has_model(raw) {
            return None;
        }
        Some(crate::model_route::rewrite_body_model(
            raw,
            self.model_at(hop)?,
        ))
    }

    /// 这一跳**该不该跳过**（用户禁用 ⇒ 跳过，一次请求都不发）。
    pub async fn should_skip(&self, hop: usize, disabled: &Arc<RwLock<DisabledModels>>) -> bool {
        if hop == 0 {
            return false;
        }
        let Some(model) = self.model_at(hop) else {
            return false;
        };
        let model = model.to_string();
        let skip = {
            // 读锁只在块内持有：不能跨 `.await`（守卫不是 `Send`）。
            let guard = disabled.read().await;
            crate::disabled_models::is_disabled(&guard, self.scope, &model)
        };
        skip
    }

    /// 这次失败**能不能**继续降级，返回对应的触发条件。
    ///
    /// 两个条件都要满足：
    /// 1. 状态码能映射到一个触发条件（5xx ⇒ 服务端故障，429 ⇒ 限流）；
    /// 2. 该触发条件在**用户配的白名单**里。
    pub fn can_fallback(&self, response: &Response) -> Option<Trigger> {
        let trigger = trigger_of_status(response.status())?;
        self.triggers.contains(&trigger).then_some(trigger)
    }

    /// 降级成功时往响应上写三个降级头（`from` / `to` / `reason`）。
    pub fn apply_headers(&self, hop: usize, response: &mut Response, trigger: Trigger) {
        if hop == 0 {
            return;
        }
        let Some(to) = self.model_at(hop) else {
            return;
        };
        apply_fallback_headers(
            response,
            &FallbackInfo {
                from: self.origin.clone(),
                to: to.to_string(),
                reason: trigger,
                skipped: Vec::new(),
            },
        );
    }
}

/// 响应状态码 ⇒ 触发条件。
///
/// 只认两类（见模块文档「判定比 WB 粗一档」）：
/// - **5xx** ⇒ 服务端故障；
/// - **429** ⇒ 限流（无法区分模型级 / 账号级，取模型级那一档，因为它是**可配**的：
///   用户没勾就不会降级）。
///
/// 其余（400 参数错 / 401 凭据 / 403 审核 / 404）**一律不降级**：
/// 换模型救不了它们，而审核类错误降级尤其危险。
fn trigger_of_status(status: StatusCode) -> Option<Trigger> {
    if status.is_server_error() {
        return Some(Trigger::Server);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Some(Trigger::ModelRate);
    }
    None
}
