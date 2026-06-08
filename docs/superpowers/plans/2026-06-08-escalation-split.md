# escalation.rs 拆分重构 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `src/agent/escalation.rs`（1274 行）按职责拆成 `escalation/{mod,logic,ledger}.rs` 三文件目录，行为零变更。

**Architecture:** `git mv escalation.rs escalation/mod.rs` 保留 git 历史，然后从 mod.rs 把纯函数簇（+38 测试）移出到 `logic.rs`、把台账 CRUD 移出到 `ledger.rs`，mod.rs 只留 4 个 async 编排函数 + 模块 wiring + `pub use` re-export。这是一次**原子重构**——不追求中间步可编译，整体改完一次性 `cargo build --lib` + `cargo test --lib` 验证。

**Tech Stack:** Rust 2021 / cargo / tokio。验证用 `cargo build --lib`（编译 + 可见性/路径）+ `cargo test --lib`（38 个 escalation 纯函数测试 + 全 lib 基线 ≥350 passed/0 failed）。

**重构性质说明：** 这是"行为零变更 + 已有 38 个测试做回归网"的纯结构重构，**不是 TDD**（不写新失败测试）。测试本身原样搬运，是验证拆分正确的安全网。

---

## 项 → 目标文件 映射（基于当前 escalation.rs 行号）

> 搬运以"函数/类型定义整体 + 上方文档注释"为单位。

**logic.rs**（纯函数/枚举/常量 + 全部测试）：
| 项 | 行 |
| --- | --- |
| `SHORT_CODE_ALPHABET` / `SHORT_CODE_BODY_LEN` | 23–24 |
| `short_code_from_seed` | 28–38 |
| `ReplyMatch` enum | 40–49 |
| `extract_short_code` | 51–62 |
| `match_principal_reply` | 64–77 |
| `render_principal_card` | 79–89 |
| `fallback_holding_reply` | 91–98 |
| `authorization_is_usable` | 100–110 |
| `relay_substance_if_usable` | 112–123 |
| `HighRiskEscalationMode` enum | 125–133 |
| `parse_high_risk_mode` | 135–141 |
| `assert_target_is_principal` | 160–179（含 `#[allow(dead_code)]`）|
| `is_principal_relay_trigger` | 293–304 |
| `relay_output_leaks_internal_payload` | 306–319 |
| `consecutive_unprogressed_turns` | 321–339（私有，仅 build_decision_signals_text 用）|
| `build_decision_signals_text` | 341–387 |
| `should_escalate_held` | 389–398 |
| `is_duplicate_key_error` | 582–588（**升 pub(crate)**）|
| `sanitize_verdict` | 629–641 |
| `is_stuck_or_undelivered` | 811–817 |
| `DEFAULT_STUCK_THRESHOLD` | 820 |
| `#[cfg(test)] mod tests` | 822–1274（38 测试整体搬）|

**ledger.rs**（async + db CRUD）：
| 项 | 行 |
| --- | --- |
| `principal_decider_wxid` | 144–158 |
| `insert_pending_escalation` | 181–244 |
| `list_pending_for_principal` | 247–268 |
| `has_pending_for_contact` | 271–291 |
| `resolve_escalation` | 550–579 |
| `emit_knowledge_gap_proposal` | 594–627 |
| `lookup_principal_config` | 689–707 |
| `enqueue_relay_task` | 780–807 |

**mod.rs**（业务编排 + wiring）：
| 项 | 行 |
| --- | --- |
| 文件头 `//!` 文档块 | 1–5 |
| `escalate_held_decision` | 408–503 |
| `handle_principal_decision_relay` | 506–547 |
| `interpret_principal_reply` | 645–683 |
| `handle_principal_reply` | 712–777 |

---

## Task 1: git mv 保留历史

**Files:**
- Rename: `src/agent/escalation.rs` → `src/agent/escalation/mod.rs`

- [ ] **Step 1: 创建目录并 git mv**

Run（在仓库根）：
```bash
mkdir -p src/agent/escalation
git mv src/agent/escalation.rs src/agent/escalation/mod.rs
```

- [ ] **Step 2: 验证 mv 后仍编译（结构未变，应通过）**

Run:
```bash
cargo build --lib 2>&1 | tail -5
```
Expected: 编译成功（`git mv` 只是改路径，`agent/mod.rs:31` 的 `pub mod escalation;` 现在解析到 `escalation/mod.rs`，等价）。

> 不在此步 commit——整个拆分是一次原子重构，Task 5 一起 commit。

---

## Task 2: 建 logic.rs，搬入纯函数 + 测试

**Files:**
- Create: `src/agent/escalation/logic.rs`
- Modify: `src/agent/escalation/mod.rs`（删除已搬出的项）

- [ ] **Step 1: 新建 logic.rs，写 use 头**

创建 `src/agent/escalation/logic.rs`，顶部：
```rust
//! 决策请示通道——纯函数层（短码 / 匹配 / 授权 / 信号 / 卡死判定 / 出站守卫 / verdict 校验）。
//! 无 I/O、无 async、无 db/mcp/state 依赖，全部可单测（见文件末 mod tests）。

use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, Contact, IntentTrajectoryEntry, OperationDomainConfig,
    PrincipalDecision, ALLOWED_PRINCIPAL_VERDICT, AWAITING_PRINCIPAL_DECISION_ATTR,
    PRINCIPAL_RELAY_SENTINEL, PRINCIPAL_VERDICT_DEFERRED,
};
use crate::agent::types::AgentTrigger;
```
> 这是基于勘察的初始 use 清单。`cargo build --lib` 报 unused 或 cannot-find 时增删：`AgentPrincipalEscalation`（match_principal_reply 参数）、`Contact`/`OperationDomainConfig`（build_decision_signals_text 参数）、`AgentTrigger`（is_principal_relay_trigger）、`PrincipalDecision`（sanitize_verdict）、`AppError`/`AppResult`（assert_target_is_principal）。测试模块内的 `HOLD_CATEGORY_*` 由测试自己的局部 `use crate::agent::types::...` 引入，不在文件顶部。

- [ ] **Step 2: 从 mod.rs 剪切纯函数整体搬入 logic.rs**

按"项→logic.rs 映射表"，把这些项**从 `escalation/mod.rs` 剪切**（连同上方文档注释）粘到 logic.rs（保持原有顺序）：`SHORT_CODE_ALPHABET`、`SHORT_CODE_BODY_LEN`、`short_code_from_seed`、`ReplyMatch`、`extract_short_code`、`match_principal_reply`、`render_principal_card`、`fallback_holding_reply`、`authorization_is_usable`、`relay_substance_if_usable`、`HighRiskEscalationMode`、`parse_high_risk_mode`、`assert_target_is_principal`、`is_principal_relay_trigger`、`relay_output_leaks_internal_payload`、`consecutive_unprogressed_turns`、`build_decision_signals_text`、`should_escalate_held`、`sanitize_verdict`、`is_stuck_or_undelivered`、`DEFAULT_STUCK_THRESHOLD`。

- [ ] **Step 3: 搬 is_duplicate_key_error 并升可见性**

把 `is_duplicate_key_error`（原 582–588）从 mod.rs 剪切到 logic.rs，**把签名从 `fn is_duplicate_key_error` 改为 `pub(crate) fn is_duplicate_key_error`**（ledger 的 insert 要跨文件调用它）：
```rust
/// 检测 mongodb 唯一键冲突（短码碰撞重试用）。
pub(crate) fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    matches!(
        *e.kind,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(ref we))
            if we.code == 11000
    )
}
```
> 以原实现为准（上面是结构示意，剪切时用原文件实际函数体，不要凭记忆重写）。

- [ ] **Step 4: 把 mod tests 整体搬入 logic.rs 末尾**

把 `#[cfg(test)] mod tests { ... }`（原 822–1274，含 `use super::*;` 和 38 个测试）从 mod.rs 整体剪切到 logic.rs 末尾，**一字不改**。因为测试全测 logic 层纯函数，`use super::*` 现在指向 logic 模块自身，被测项都在同文件 → 路径天然有效。

---

## Task 3: 建 ledger.rs，搬入台账 CRUD

**Files:**
- Create: `src/agent/escalation/ledger.rs`
- Modify: `src/agent/escalation/mod.rs`

- [ ] **Step 1: 新建 ledger.rs，写 use 头**

创建 `src/agent/escalation/ledger.rs`，顶部：
```rust
//! 决策请示通道——台账 CRUD 层（pending 台账增删查改 / 知识缺口提案 / relay task 入队）。
//! 全部 async + db 访问。

use crate::error::AppResult;
use crate::models::{
    AgentPrincipalEscalation, AgentTask, OperationKnowledgeChunk, PrincipalDecision,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED, PRINCIPAL_VERDICT_DEFERRED,
};
use crate::routes::AppState;
use super::logic::{is_duplicate_key_error, short_code_from_seed};
use mongodb::bson::{doc, DateTime};
```
> `cargo build --lib` 报错时增删。`enqueue_relay_task` 用 `AgentTask`；`emit_knowledge_gap_proposal` 用 `OperationKnowledgeChunk`；`resolve_escalation` 用 `PrincipalDecision`/`PRINCIPAL_*`；`insert_pending_escalation` 用 `short_code_from_seed`/`is_duplicate_key_error`（来自 super::logic）+ `ALLOWED_ESCALATION_CATEGORY`。

- [ ] **Step 2: 从 mod.rs 剪切 CRUD 函数整体搬入 ledger.rs**

按映射表把这些项**从 mod.rs 剪切**（连同文档注释）粘到 ledger.rs：`principal_decider_wxid`、`insert_pending_escalation`、`list_pending_for_principal`、`has_pending_for_contact`、`resolve_escalation`、`emit_knowledge_gap_proposal`、`lookup_principal_config`、`enqueue_relay_task`。

> `enqueue_relay_task` 原是私有 `fn`，只被 `handle_principal_reply`（mod.rs 编排）调用 → 跨文件后需升 `pub(crate)`。剪切时把 `async fn enqueue_relay_task` 改为 `pub(crate) async fn enqueue_relay_task`。

---

## Task 4: 改造 mod.rs（wiring + re-export + 保留编排）

**Files:**
- Modify: `src/agent/escalation/mod.rs`

- [ ] **Step 1: mod.rs 现状应只剩 4 个编排函数 + 文件头 //! + 原 use 块**

经 Task 2/3 剪切后，mod.rs 应只剩：文件头 `//!` 文档块（原 1–5）、原 `use` 块（原 7–20）、`escalate_held_decision`、`handle_principal_decision_relay`、`interpret_principal_reply`、`handle_principal_reply`。确认无其它残留项（用 `grep -nE "^(pub )?(pub\(crate\) )?(async )?fn " src/agent/escalation/mod.rs` 核对，应只有这 4 个）。

- [ ] **Step 2: 在文件头 //! 块之后插入模块声明 + re-export**

在 `//!` 文档块（原 1–5 行）之后、原 `use` 块之前，插入：
```rust

mod ledger;
mod logic;

pub use ledger::*;
pub use logic::*;
```
> `pub use` 匹配 `agent/mod.rs:31` 的 `pub mod escalation`（外部用 `pub` 可见性）。通配 `*` 把 logic+ledger 全部项 re-export 到 escalation 命名空间 → 外部 13 处 `escalation::xxx` 调用零改动。

- [ ] **Step 3: 清理 mod.rs 的 use 块**

mod.rs 现在只剩 4 个编排函数，原 `use` 块（原 7–20）按编排函数实际所需保留/精简：
```rust
use super::generate_agent_json;
use super::types::{AgentDecision, DecisionReviewResult};
use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, AgentTask, Contact, OperationDomainConfig,
    PrincipalDecision, AWAITING_PRINCIPAL_DECISION_ATTR, ESCALATION_CATEGORY_HIGH_RISK_GATED,
};
use crate::mcp;
use crate::prompts;
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime};
```
> 已核实 4 个编排函数实际用到的符号：`generate_agent_json`、`AgentDecision`/`DecisionReviewResult`、`AppError`/`AppResult`、`AgentPrincipalEscalation`/`AgentTask`/`Contact`/`OperationDomainConfig`/`PrincipalDecision`/`AWAITING_PRINCIPAL_DECISION_ATTR`/`ESCALATION_CATEGORY_HIGH_RISK_GATED`、mcp/prompts/AppState/bson。**不含** `AgentTrigger`（已随 is_principal_relay_trigger 归 logic）和 `PRINCIPAL_ESCALATION_STATUS_PENDING`（已随 ledger 走）——这两个留在 mod.rs 会触发 `-Dwarnings` unused 硬错误。`super::generate_agent_json` / `super::types::*` 保留不变——mod.rs 深度同原 escalation.rs，`super::` 仍指向 agent。最终以 `cargo build --lib` 报错为准增删。编排函数调 logic/ledger 项靠 Step 2 的 `pub use` re-export（同模块内直接可见），不需要额外 `use self::logic::...`。

- [ ] **Step 4: 确认 handle_principal_decision_relay 的 gateway 调用不变**

确认 `handle_principal_decision_relay` 里的 `crate::agent::gateway::relay_principal_decision_to_customer(...)`（原 545 行）原样保留——`crate::` 绝对路径，搬入 mod.rs 后不变。

---

## Task 5: 整体验证 + commit

**Files:**
- Verify: 全 `escalation/` 目录

- [ ] **Step 1: cargo build --lib（编译 + 可见性 + 路径）**

Run:
```bash
cargo build --lib 2>&1 | tail -20
```
Expected: 编译成功，0 error。
常见报错与对策：
- `cannot find value/type X` → 某文件 use 头漏了，按提示在对应文件补 `use crate::...`。
- `function is private` → 跨文件调用的项可见性不足（检查 `is_duplicate_key_error`/`enqueue_relay_task` 是否已升 `pub(crate)`）。
- `unused import` → 删对应 use（`-Dwarnings` 下 unused 是硬错误，必须清）。

- [ ] **Step 2: 确认外部调用点未受影响**

Run:
```bash
cargo build --lib 2>&1 | grep -E "escalation|gateway|webhooks|decision" || echo "无相关错误"
```
Expected: 无错误（re-export 保住了 gateway.rs/webhooks.rs/decision.rs 的 13 处 `escalation::xxx` 调用）。

- [ ] **Step 3: cargo test --lib（38 测试 + 全 lib 基线）**

Run:
```bash
cargo test --lib 2>&1 | tail -15
```
Expected: 全 pass，passed 数 ≥350（基线 R11.6）。其中 escalation 的 38 个测试应全部出现且 pass。

- [ ] **Step 4: 跑基线门脚本**

Run:
```bash
bash scripts/check-baseline.sh 2>&1 | tail -10
```
Expected: PASS（lib ≥350 + 4 PBT 文件累计 ≥33，0 failed）。
> 若本地因磁盘/编译卡死（PBT binary 重编译），改为只跑 `cargo test --lib` 确认 lib 部分，PBT 留 CI（符合本地资源受限纪律）。

- [ ] **Step 5: 确认最终结构 + 行数**

Run:
```bash
wc -l src/agent/escalation/mod.rs src/agent/escalation/logic.rs src/agent/escalation/ledger.rs
grep -rn "is_duplicate_key_error\|enqueue_relay_task" src/agent/escalation/ | grep "pub(crate)"
```
Expected: 三文件存在；mod.rs ~300 行、logic.rs ~600 行（含测试）、ledger.rs ~250 行；`is_duplicate_key_error` 和 `enqueue_relay_task` 都是 `pub(crate)`。

- [ ] **Step 6: commit**

```bash
git add src/agent/escalation.rs src/agent/escalation/
git commit -m "$(cat <<'EOF'
refactor(agent): escalation.rs 1274行按职责拆成 escalation/{mod,logic,ledger}

logic(纯函数+38测试) / ledger(台账CRUD) / mod(4个async编排+re-export);
is_duplicate_key_error+enqueue_relay_task 升 pub(crate) 跨文件;pub use 保 13 处外部调用;
行为零变更,cargo test --lib 全绿基线≥350

Co-Authored-By: Claude Opus 4 (1M context) <noreply@anthropic.com>
EOF
)"
```
> `git add src/agent/escalation.rs` 让 git 识别 mv（旧路径删除 + 新目录新增），git 会自动判定为 rename。

---

## 验证策略总览

| 层 | 手段 | 何时 |
| --- | --- | --- |
| 编译 + 可见性 + 路径 | `cargo build --lib` | Task 5 |
| 单测回归（38 escalation + 全 lib） | `cargo test --lib`（≥350） | Task 5 |
| 基线门 | `scripts/check-baseline.sh`（lib+4 PBT） | Task 5（本地或 CI）|
| 外部契约 | 编译无 gateway/webhooks/decision 报错 | Task 5 |
| 重套件（integration/real-llm） | CI | push 后 |

## 回退策略

单个 commit。验证不过且无法就地修复 → `git revert` 该 commit（或重构未 commit 前 `git checkout -- src/agent/` + `git clean` 清新文件回到 mv 前）。

## 非目标（YAGNI）

- 不改任何函数逻辑/控制流/字符串/注释内容——纯搬运。
- 不给 ledger/编排补新测试。
- 不动 escalation 之外的文件（re-export 通配保证外部零改动）。
- 不合并 logic/ledger。
