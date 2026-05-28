//! Phase E / E1：domain-agnostic 框架的形式边界声明。
//!
//! 路线图（`docs/agent-policy.md` Phase E 段）红线：CLAUDE.md "Group / Moments
//! 不要折叠到 user-ops" —— 不同运营域的入口、状态机、知识路由、决策闸应当各自
//! 独立。Phase 1 范围只有 user-ops 一个域，所以本文件**只做边界声明**：
//!
//! 1. 给"运营域"这个抽象一个唯一 anchor（[`OpsDomain`] trait）；
//! 2. 把当前散落在 `decision.rs / runtime.rs / knowledge_router.rs / guards.rs`
//!    多处的字符串 `"user_operations"` 收敛到一个常量
//!    [`USER_OPS_DOMAIN_ID`]，新代码必须从这里取；
//! 3. user-ops 自身是 [`UserOpsDomain`] 第一实现。
//!
//! 本阶段**不**强制把 `decide_reply_with_promote / run_user_operation_gateway /
//! route_operation_knowledge` 切成 trait 分发——单实现期签名失真风险大于收益。
//! 当 group / moments 真实落地、产生第二个 domain 调用方时，再按真实需求把
//! 分发点接到 trait 上（路线图 "≥ 2 个 domain 真实需求驱动" 原则）。
//!
//! 现有 22 处 `"user_operations"` 字面量按 R11 / `agent_run_logs.domain` 写入
//! 路径保留——盲改字面量会触发 DB schema 兼容性问题，不在本次形式收口范围。

/// user-ops 运营域 id。所有新代码引用 user-ops 域时使用本常量，避免再扩散
/// `"user_operations"` 字符串字面量。
///
/// 与 `agent_run_logs.domain` / `operation_domain_configs.domain` /
/// `prompt_templates.domain` / `operation_state_policies.domain` 等
/// MongoDB 字段值保持一致——本常量本身就是这些字段的唯一合法 user-ops 值。
pub const USER_OPS_DOMAIN_ID: &str = "user_operations";

/// 运营域抽象边界声明。
///
/// Phase E1 形式收口：仅暴露 domain id + state_machine 域 key 两个不变量；
/// `enforce_decision_guards / knowledge_router` 的入参签名涉及 `AppState`
/// 引用、`Contact` / `ConversationMessage` slice、`OperatingMemory` 等
/// 大量上下文，单实现期没有第二个 domain 校验签名稳定性，所以本 trait
/// **不**收纳这些方法签名。当 group / moments 真实落地时再按真实分发点
/// 形成的最小公共签名扩展本 trait。
///
/// 红线（CLAUDE.md）：
/// - "Phase 1 scope is user (private-chat) operations. Group and Moments
///   operations are planned separate operation domains; do not fold them
///   into the user-ops code path."
/// - 任何想把 group / moments 行为塞进 user-ops 同一函数的尝试都应当被
///   拒绝；新域应当新建 `OpsDomain` 实现，与 [`UserOpsDomain`] 平级。
pub trait OpsDomain: Send + Sync + 'static {
    /// 运营域 id。与 `agent_run_logs.domain` / `operation_domain_configs.domain`
    /// 等字段值同形（snake_case，例：`"user_operations"`）。
    fn id(&self) -> &'static str;

    /// 状态机域 key。当前与 [`Self::id`] 同值，但保留作为独立方法以便未来
    /// 拆分（例如同一域多套状态机灰度时返回不同 key）。
    fn state_machine_domain_key(&self) -> &'static str {
        self.id()
    }
}

/// user-ops 域第一实现。本身是 unit struct，无内部状态——所有 user-ops
/// 行为仍由 `decision / gateway / knowledge_router / guards` 既有函数承载。
///
/// 用法（仅推荐在新代码中使用，旧代码 22 处 `"user_operations"` 字面量
/// 不强制替换）：
///
/// ```ignore
/// use crate::agent::domain::{OpsDomain, UserOpsDomain};
/// let domain = UserOpsDomain;
/// assert_eq!(domain.id(), "user_operations");
/// ```
#[derive(Default, Debug, Clone, Copy)]
pub struct UserOpsDomain;

impl OpsDomain for UserOpsDomain {
    fn id(&self) -> &'static str {
        USER_OPS_DOMAIN_ID
    }
}

#[cfg(test)]
mod tests {
    //! 形式收口契约：常量值 / trait 默认实现 / 第一实现的 id 三者一致。

    use super::*;

    #[test]
    fn user_ops_domain_id_is_canonical_string() {
        assert_eq!(USER_OPS_DOMAIN_ID, "user_operations");
    }

    #[test]
    fn user_ops_domain_implements_ops_domain() {
        let d = UserOpsDomain;
        assert_eq!(d.id(), "user_operations");
        assert_eq!(d.state_machine_domain_key(), "user_operations");
    }

    #[test]
    fn state_machine_domain_key_defaults_to_id() {
        // 未显式覆写时，state_machine_domain_key() 应回落 id()。这是契约
        // 默认值，未来子域如果需要拆分必须显式 override，避免静默漂移。
        struct DummyDomain;
        impl OpsDomain for DummyDomain {
            fn id(&self) -> &'static str {
                "dummy_ops"
            }
        }
        let d = DummyDomain;
        assert_eq!(d.state_machine_domain_key(), "dummy_ops");
    }
}
