# escalation.rs 拆分重构设计

- 日期：2026-06-08
- 范围：`src/agent/escalation.rs`（1274 行，决策请示通道 Principal Decision Channel）
- 目标：按职责拆成 `escalation/{mod,logic,ledger}.rs` 三文件目录，提升内聚、便于后续在请示通道加功能。
- 性质：**纯职责分组搬运，行为零变更**——只搬代码 + 补 `use`/re-export，不改任何逻辑/控制流/字符串。

## 为什么是这个文件

全后端五维审查（体量 × churn × 巨型函数 × 测试密度 × 可分性）结论：escalation.rs 是当前最适合动手的目标。

- **无巨型函数**：最大的 `escalate_held_decision` 仅 96 行。拆分是纯职责搬运，**不需要 Extract Method 改控制流** → 零行为风险。对比 gateway.rs（1121 行巨型函数）、knowledge_agent.rs（382 行主循环）、chat.rs（3 个巨型函数 + 0 测试）的痛点都是"巨型函数不可拆"，escalation 是唯一"大小适中 + 高 churn(22，活跃) + 厚测试 + 无巨型函数"的组合。
- **测试安全网厚且边界干净**：38 个测试全是同步 `#[test]`（0 个 async），且**只测 logic 层纯函数**，不引用任何 ledger/编排函数。所以测试整体随 logic.rs 走，`use super::*` 指向 logic 自身，被测纯函数都在同文件 → **零路径调整**（避开 review.rs 拆分踩过的 `super::` 解析坑）。
- **本地可验证**：`cargo test --lib` 能跑这 38 个测试（纯函数，不依赖 Docker），符合本地资源受限纪律。

## 目标结构

```
src/agent/escalation.rs  →  src/agent/escalation/
├── mod.rs        业务编排（4 个 async 高层流程）+ 模块 wiring + re-export
├── logic.rs      纯函数 / 枚举 / 常量（无 I/O）+ 全部 38 个测试
└── ledger.rs     台账 CRUD（async + db 访问，无测试）
```

### logic.rs（纯函数，无 I/O）

顶层项：`SHORT_CODE_ALPHABET`、`SHORT_CODE_BODY_LEN`、`short_code_from_seed`、`ReplyMatch`、`extract_short_code`、`match_principal_reply`、`render_principal_card`、`fallback_holding_reply`、`authorization_is_usable`、`relay_substance_if_usable`、`HighRiskEscalationMode`、`parse_high_risk_mode`、`assert_target_is_principal`、`is_principal_relay_trigger`、`relay_output_leaks_internal_payload`、`consecutive_unprogressed_turns`、`build_decision_signals_text`、`should_escalate_held`、`DEFAULT_STUCK_THRESHOLD`、`is_stuck_or_undelivered`、`sanitize_verdict`、`is_duplicate_key_error` + `#[cfg(test)] mod tests`（38 个测试整体搬入）。

use 头：
```rust
use crate::error::{AppError, AppResult};
use crate::models::{
    Contact, OperationDomainConfig, IntentTrajectoryEntry, PrincipalDecision,
    AWAITING_PRINCIPAL_DECISION_ATTR, PRINCIPAL_RELAY_SENTINEL,
    ALLOWED_PRINCIPAL_VERDICT, PRINCIPAL_VERDICT_DEFERRED,
};
use crate::agent::types::{AgentTrigger, /* HOLD_CATEGORY_* 由测试局部 use 引入，函数体用到的在此补全 */};
```
> `PrincipalDecision` 由 `sanitize_verdict` 用（logic 纯函数，越界回落 deferred）。实际 use 清单以 `cargo build --lib` 报错为准逐个补齐。测试模块内已有的 `use crate::models::*` / `use crate::agent::types::HOLD_CATEGORY_*` 局部 use 是 `crate::` 绝对路径，搬入后不变。

### ledger.rs（台账 CRUD，async + db）

顶层项：`principal_decider_wxid`、`insert_pending_escalation`、`list_pending_for_principal`、`has_pending_for_contact`、`resolve_escalation`、`emit_knowledge_gap_proposal`、`lookup_principal_config`、`enqueue_relay_task`。

use 头：
```rust
use crate::error::AppResult;
use crate::models::{
    AgentPrincipalEscalation, AgentTask, OperationKnowledgeChunk, PrincipalDecision,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED, PRINCIPAL_VERDICT_DEFERRED,
};
use crate::routes::AppState;
use super::logic::{short_code_from_seed, is_duplicate_key_error};
use mongodb::bson::{doc, DateTime};
```

### mod.rs（业务编排）

顶层项：`escalate_held_decision`、`handle_principal_decision_relay`、`interpret_principal_reply`、`handle_principal_reply`，加模块文档注释（原文件头 //! 块）+ 模块 wiring + re-export。

结构：
```rust
//! 决策请示通道（原文件头 //! 文档块整体保留）

mod logic;
mod ledger;

pub use logic::*;
pub use ledger::*;

use super::generate_agent_json;            // super = agent（mod.rs 深度同原 escalation.rs，不变）
use super::types::{AgentDecision, AgentTrigger, DecisionReviewResult};
use crate::error::{AppError, AppResult};
use crate::models::{ /* 编排函数用到的 */ };
use crate::mcp;
use crate::prompts;
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime};

// 4 个编排 async fn ...
```

## 关键设计点（Rust 可见性 / 路径）

这些是勘察阶段挖出的、不处理就会编译失败的点：

1. **`is_duplicate_key_error` 可见性升级**：原是私有 `fn`（582 行），归 logic.rs。但它被 ledger 的 `insert_pending_escalation`（234 行）调用 → 必须升为 `pub(crate) fn`，ledger.rs 通过 `use super::logic::is_duplicate_key_error` 调用。**这是本次唯一的可见性变更。**

2. **ledger → logic 内部依赖**：`insert_pending_escalation` 用 `short_code_from_seed`（logic）+ `is_duplicate_key_error`（logic）。ledger.rs 顶部 `use super::logic::{short_code_from_seed, is_duplicate_key_error}`。

3. **`super::` 解析层级**：mod.rs 在 `escalation/` 下，其 `super::` 仍指向 `agent`（与原 escalation.rs 深度相同）→ 编排函数里的 `super::generate_agent_json`、`super::types::*` **不用改**。但 logic.rs / ledger.rs 多一层，不能用 `super::types`，须用 `crate::agent::types::*`（绝对路径）。

4. **re-export 保外部契约**：外部 13 处调用点（gateway.rs 9 处、webhooks.rs 2 处、decision.rs 1 处）都写 `escalation::xxx` 或 `crate::agent::escalation::xxx`。mod.rs 用 `pub use logic::*; pub use ledger::*;` 把 logic+ledger 的项全部 re-export 到 escalation 命名空间 → **所有外部调用零改动**。`agent/mod.rs:31` 是 `pub mod escalation`，re-export 用 `pub use`（匹配现有可见性）。

   外部依赖的具体项（re-export 必须覆盖）：
   - logic：`build_decision_signals_text`、`render_principal_card`、`is_principal_relay_trigger`、`relay_output_leaks_internal_payload`
   - ledger：`principal_decider_wxid`、`has_pending_for_contact`、`insert_pending_escalation`、`emit_knowledge_gap_proposal`、`lookup_principal_config`
   - 编排（mod.rs 直接 pub）：`handle_principal_decision_relay`、`escalate_held_decision`、`handle_principal_reply`

5. **唯一反调 gateway**：`handle_principal_decision_relay`（mod.rs）第 545 行 `crate::agent::gateway::relay_principal_decision_to_customer(...)`，`crate::` 绝对路径，搬入 mod.rs 不变。单向耦合，无循环引用。

## 验证策略

- **每步**：`cargo build --lib`（编译 + 可见性/路径正确性）+ `cargo test --lib`（38 个 escalation 测试 + 全 lib 测试，对照基线 ≥350 passed/0 failed）。
- **基线门**：`scripts/check-baseline.sh`（lib ≥350 + 4 PBT 文件 ≥33）—— 本地或 CI。
- **重套件**（integration / real-llm）：走 CI（本地磁盘 / Docker 受限）。
- 行为零变更的保证：纯搬运 + 补 use，38 个纯函数测试是回归网；async 编排/ledger 本就靠 CI 集成测试覆盖（它们无单测，拆分不改变这点）。

## 非目标（YAGNI）

- 不改任何函数逻辑 / 控制流 / 字符串 / 注释内容。
- 不给 ledger/编排补新测试（超出本次范围；它们的覆盖现状不因拆分改变）。
- 不动 escalation 之外的文件，除非 re-export 没覆盖导致编译失败（理论上不应发生，re-export 用 `*` 通配）。
- 不合并 logic/ledger（三种关注点分明：纯计算 / 数据访问 / 业务流程）。

## 风险与回退

- **风险**：escalation 是活跃文件（churn 22），拆分期间并行改动会冲突。**缓解**：当前工作区干净、main 已同步，窗口合适；一次性完成。
- **风险**：漏 re-export 某个外部依赖项 → 外部文件编译失败。**缓解**：用 `pub use logic::*; pub use ledger::*;` 通配 re-export，不逐个列；`cargo build --lib` 立即暴露。
- **风险**：logic/ledger 漏 `use` 某类型 → 编译失败。**缓解**：`cargo build --lib` 报 "cannot find" 逐个补。
- **回退**：单个 commit；验证不过且无法就地修复 → `git revert` 该 commit。
