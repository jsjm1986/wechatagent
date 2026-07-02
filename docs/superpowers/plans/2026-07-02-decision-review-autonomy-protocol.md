# 决策复盘补齐 AI 自治协议 9 字段 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让用户运营驾驶舱的决策复盘下钻展示 AI 每轮产出的 9 个自治协议「内心独白」字段（按三组：回复决策 / 理解 / 运营依据）。

**Architecture:** 纯读投影 —— 9 字段已由 `to_document(&final_decision)` 落在 `agent_run_logs.decision`（camelCase），`/api/decision-reviews` 端点已按 run_id join run log（`fetch_run_status`）。改动只是：join 时从已在手的 `log.decision` 多抽 9 字段 → 投影加嵌套 `autonomyProtocol` 对象 → 前端三组展示。不动 `AgentDecisionReview` 模型、不动写入侧、不加迁移。

**Tech Stack:** Rust/Axum（后端投影 + 契约快照单测）、React 19 + Vite + TypeScript（前端展示 + vitest）、MongoDB。

## Global Constraints

- **禁用词 lint**（`scripts/check-no-human-takeover.sh` 扫 `src/`+`frontend/src/` 新增行）：新增代码/注释/文案/class 不得含 `人工` / `接管` / `takeover` / `hand-off`（单字「人工」也被字面级拦截）。
- **后端只读聚合红线**：本次后端改动纯读投影，不碰 agent 决策 / gateway 写入侧 / 发送 / `AgentDecision` 结构本身，不加迁移。
- **优雅降级是硬要求**（非仅向后兼容）：`autonomyProtocol` 为 null（无 run_id / 无 run log / 9 字段全空）时前端不渲染该区、不报错。覆盖「历史旧数据」+「管理 Agent 主动发送路径」（`send_contact_message_gateway` 写复盘但不写 run log、decision 9 字段全空）两类现实来源。
- **契约快照不回归**：改 `decision_review_json` 投影后，同步 re-bless fixture + 更新前端 `CANONICAL_KEYS`，如实标注为「新增 autonomyProtocol 的必要连带」。
- **测试基线不回归**：`cargo test --lib` ≥ 350 passed / 0 failed；前端 `tsc` 0 error / vitest 全绿 / build 过。
- **CSS Modules**：前端新样式 `className={styles.x}` + tokens.css var()，无硬编码 hex。
- **9 字段 camelCase 键名（两 Task 必须一致）**：`userUnderstanding` / `relationshipRead` / `operationGoal` / `knowledgeNeedReason` / `memoryUpdateReason` / `riskSelfCheck` / `selfCritique` / `whyShouldReply` / `whySkipReply`。

---

### Task 1: 后端 —— run-log join 抽 9 字段 + 投影加 autonomyProtocol + 契约同步

**Files:**
- Modify: `src/routes/reviews.rs`（加纯函数 `autonomy_protocol_from_decision` + `RunStatusView` 结构体 + 改 `fetch_run_status` 返回类型 + 两个调用点 `:62`/`:86`）
- Modify: `src/routes/shared.rs`（`decision_review_json` 加第 4 参数 `autonomy_protocol` + 输出 `autonomyProtocol` 键 + 更新契约单测 `:1981`）
- Modify: `frontend/src/contracts/decision_review.fixture.json`（`UPDATE_SNAPSHOTS=1` re-bless）
- Modify: `frontend/src/contracts/decisionReview.contract.ts`（`CANONICAL_KEYS` 加 `"autonomyProtocol"`，保持字母序）

**Interfaces:**
- Produces（Task 2 前端消费）：`/api/decision-reviews` 每条 item 顶层新增 `autonomyProtocol`：`null`（降级）或 9 键全 string 的对象 `{ userUnderstanding, relationshipRead, operationGoal, knowledgeNeedReason, memoryUpdateReason, riskSelfCheck, selfCritique, whyShouldReply, whySkipReply }`（空字段以 `""` 呈现）。
- Consumes：无（依赖已存在的 `AgentRunLog.decision: Document`、`fetch_run_status` 的 run_id join）。

- [ ] **Step 1: 写纯函数失败单测**

先 Read `src/routes/reviews.rs:1-15`（现有 imports）确认 `serde_json::{json, Value}` 已在（`:10`）；`mongodb::{bson::doc, options::FindOptions}` 在 `:8` —— 需把 `bson::doc` 扩为 `bson::{doc, Document}`。

在 `src/routes/reviews.rs` 文件末尾追加（若已有 `#[cfg(test)] mod tests` 则 append 用例，不改旧用例）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn autonomy_protocol_all_empty_returns_none() {
        // decision 无任何自治字段（或全空串）→ None（优雅降级）
        let decision = doc! { "replyText": "hi", "userUnderstanding": "" };
        assert!(autonomy_protocol_from_decision(&decision).is_none());
    }

    #[test]
    fn autonomy_protocol_partial_returns_full_nine_keys() {
        // 任一非空 → Some，含全 9 键，缺失/空的填 ""
        let decision = doc! { "whyShouldReply": "用户主动询问，及时回应推进决策" };
        let v = autonomy_protocol_from_decision(&decision).expect("some");
        let obj = v.as_object().expect("object");
        assert_eq!(obj.len(), 9);
        assert_eq!(obj.get("whyShouldReply").and_then(|x| x.as_str()), Some("用户主动询问，及时回应推进决策"));
        assert_eq!(obj.get("userUnderstanding").and_then(|x| x.as_str()), Some(""));
        assert_eq!(obj.get("riskSelfCheck").and_then(|x| x.as_str()), Some(""));
    }
}
```

- [ ] **Step 2: 跑测试确认失败（未定义）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/wiki-three-p0" && touch src/lib.rs && cargo test --lib autonomy_protocol 2>&1 | tail -15`
Expected: 编译失败 `cannot find function autonomy_protocol_from_decision`。

- [ ] **Step 3: 实现纯函数 + RunStatusView + 改 fetch_run_status**

在 `src/routes/reviews.rs`：把 `use mongodb::{bson::doc, options::FindOptions};` 改为 `use mongodb::{bson::{doc, Document}, options::FindOptions};`。

在 `fetch_run_status`（现 `:92`）**上方**加纯函数与结构体：

```rust
/// agent_run_logs.decision（camelCase Document）里的 9 个 R1.1 自治协议字段。
/// 9 个全空（缺失或空串）→ None（优雅降级，前端不渲染「AI 内心独白」区，覆盖历史
/// 旧数据 + 管理发送路径两类无完整 decision 的复盘）；否则 Some(全 9 键对象，空的填 "")。
fn autonomy_protocol_from_decision(decision: &Document) -> Option<Value> {
    const KEYS: [&str; 9] = [
        "userUnderstanding",
        "relationshipRead",
        "operationGoal",
        "knowledgeNeedReason",
        "memoryUpdateReason",
        "riskSelfCheck",
        "selfCritique",
        "whyShouldReply",
        "whySkipReply",
    ];
    let vals: Vec<&str> = KEYS
        .iter()
        .map(|k| decision.get_str(*k).unwrap_or(""))
        .collect();
    if vals.iter().all(|v| v.trim().is_empty()) {
        return None;
    }
    let mut obj = serde_json::Map::new();
    for (k, v) in KEYS.iter().zip(vals.iter()) {
        obj.insert((*k).to_string(), Value::from(*v));
    }
    Some(Value::Object(obj))
}

struct RunStatusView {
    final_review_status: Option<String>,
    hold_category: Option<String>,
    autonomy_protocol: Option<Value>,
}
```

把现有 `fetch_run_status`（`:92-121`）整体替换为：

```rust
/// 关联同 run_id 的 AgentRunLog，取 final_review_status（顶层 snake 字段）、
/// review doc 内的 holdCategory（camelCase），以及 decision doc 内的 9 个自治协议字段。
/// 纯读投影，缺失则回 None。
async fn fetch_run_status(state: &AppState, run_id: Option<&str>) -> RunStatusView {
    let Some(run_id) = run_id.filter(|s| !s.is_empty()) else {
        return RunStatusView {
            final_review_status: None,
            hold_category: None,
            autonomy_protocol: None,
        };
    };
    match state
        .db
        .agent_run_logs()
        .find_one(doc! { "run_id": run_id }, None)
        .await
    {
        Ok(Some(log)) => {
            let frs = if log.final_review_status.is_empty() {
                None
            } else {
                Some(log.final_review_status.clone())
            };
            let hc = log
                .review
                .get_str("holdCategory")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let ap = autonomy_protocol_from_decision(&log.decision);
            RunStatusView {
                final_review_status: frs,
                hold_category: hc,
                autonomy_protocol: ap,
            }
        }
        _ => RunStatusView {
            final_review_status: None,
            hold_category: None,
            autonomy_protocol: None,
        },
    }
}
```

- [ ] **Step 4: 更新两个调用点传第 4 参数**

`src/routes/reviews.rs` `list_decision_reviews` 内（现 `:62-63`）：

```rust
        let status = fetch_run_status(&state, review.run_id.as_deref()).await;
        items.push(decision_review_json(
            review,
            status.final_review_status,
            status.hold_category,
            status.autonomy_protocol,
        ));
```

`get_decision_review` 内（现 `:86-87`）：

```rust
    let status = fetch_run_status(&state, review.run_id.as_deref()).await;
    Ok(Json(json!({ "item": decision_review_json(
        review,
        status.final_review_status,
        status.hold_category,
        status.autonomy_protocol,
    ) })))
```

- [ ] **Step 5: decision_review_json 加第 4 参数 + 输出键**

`src/routes/shared.rs` 把 `decision_review_json`（`:1175`）签名与输出改为：

```rust
pub(super) fn decision_review_json(
    review: AgentDecisionReview,
    final_review_status: Option<String>,
    hold_category: Option<String>,
    autonomy_protocol: Option<Value>,
) -> Value {
    json!({
        // ...（保留现有全部键，不动）...
        "finalReviewStatus": final_review_status,
        "holdCategory": hold_category,
        "autonomyProtocol": autonomy_protocol,
        "createdAt": crate::models::dt_to_string(review.created_at)
    })
}
```

> 只在现有 `finalReviewStatus`/`holdCategory` 之后、`createdAt` 之前插入一行 `"autonomyProtocol": autonomy_protocol,`，其余键原样保留。确认 `Value` 已在 shared.rs 的 `serde_json` import 内（`decision_review_json` 现返回 `Value`，已在）。

- [ ] **Step 6: 更新契约快照单测传第 4 参数（给 Some 展示形状）**

`src/routes/shared.rs` 的 `decision_review_json_matches_contract_fixture`（`:1981`）内，把 `decision_review_json(review, Some(...), Some(...))` 调用（现 `:2019-2023`）改为：

```rust
        let projected = decision_review_json(
            review,
            Some("approved_sent".to_string()),
            Some("none".to_string()),
            Some(serde_json::json!({
                "userUnderstanding": "用户在比较两款方案的价格",
                "relationshipRead": "关系处于评估期，信任中等",
                "operationGoal": "推进到方案确认",
                "knowledgeNeedReason": "需引用已核实的报价切片",
                "memoryUpdateReason": "记录用户预算区间",
                "riskSelfCheck": "避免对未验证功能做承诺",
                "selfCritique": "上一轮略急，本轮放慢确认",
                "whyShouldReply": "用户主动询问差异，及时回应推进决策",
                "whySkipReply": ""
            })),
        );
```

> 顶部注释（`:1976`「29 键」）改为「30 键（含 autonomyProtocol 嵌套对象）」。

- [ ] **Step 7: re-bless fixture**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/wiki-three-p0" && UPDATE_SNAPSHOTS=1 cargo test --lib decision_review_json_matches_contract_fixture 2>&1 | tail -8`
Expected: 测试通过；`frontend/src/contracts/decision_review.fixture.json` 被写入，含 `autonomyProtocol` 嵌套对象（9 键）。

- [ ] **Step 8: 同步前端 CANONICAL_KEYS**

`frontend/src/contracts/decisionReview.contract.ts` 的 `CANONICAL_KEYS` 数组加一行 `"autonomyProtocol",`，插入位置保持字母序（在 `"approved"` 之后、`"contactWxid"` 之前 —— `autonomyProtocol` < `contactWxid`；注意它排在 `"accountId"`/`"approved"` 之后）。最终该处顺序：`"accountId"`, `"approved"`, `"autonomyProtocol"`, `"contactWxid"`, …

- [ ] **Step 9: 跑后端单测 + 前端契约 vitest + lint**

Run（逐条，全绿/0 error 才算过）:
```
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/wiki-three-p0"
touch src/lib.rs && cargo test --lib autonomy_protocol 2>&1 | tail -6
cargo test --lib decision_review_json_matches_contract_fixture 2>&1 | tail -6
cargo test --lib 2>&1 | tail -5
cd frontend && npx vitest run src/__tests__/contracts 2>&1 | tail -10
bash ../scripts/check-no-human-takeover.sh <BASE> HEAD 2>&1 | tail -3
```
> `<BASE>` 用本 Task 起点 commit（controller 会告知；本 worktree 无 origin/main，须显式传 base，默认 base 会误报 no-changed-files）。
Expected: 纯函数单测 2 passed；契约快照单测 pass（读回 re-bless 后的 fixture 匹配）；`cargo test --lib` ≥ 350/0；前端 contracts vitest 全绿（fixture 的 autonomyProtocol 键在 CANONICAL_KEYS 内）；lint 0 violations。

> 若前端 `src/__tests__/contracts` 下无消费 `decisionReview` CANONICAL_KEYS 的测试，该 vitest 仍应全绿（不回归）；CANONICAL_KEYS 更新按 spec §1.5 步③ 仍是必须的契约真相源同步。

- [ ] **Step 10: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/wiki-three-p0"
git add src/routes/reviews.rs src/routes/shared.rs frontend/src/contracts/decision_review.fixture.json frontend/src/contracts/decisionReview.contract.ts
git commit -m "feat(decision-review): join 抽 9 自治协议字段投影为 autonomyProtocol(纯读,优雅降级)"
```

---

### Task 2: 前端 —— DecisionReview 类型 + ConversationReviewView 三组展示

**Files:**
- Modify: `frontend/src/types/index.ts`（新增 `AutonomyProtocol` 类型 + `DecisionReview` 加 `autonomyProtocol?`）
- Modify: `frontend/src/features/user-ops/cockpit/drilldowns/ConversationReviewView.tsx`（新增并 export `AutonomyProtocolView` 组件 + 在 `ReviewItem` 展开区挂载）
- Modify: `frontend/src/features/user-ops/cockpit/cockpit.module.css`（独白分组样式）
- Create: `frontend/src/__tests__/features/user-ops/autonomyProtocolView.test.tsx`（组件单测）

**Interfaces:**
- Consumes（Task 1 产出）：`DecisionReview.autonomyProtocol?: AutonomyProtocol | null`，对象含 9 个可选 string 键（见 Global Constraints 键名清单）。
- Produces：`export function AutonomyProtocolView({ protocol }: { protocol: AutonomyProtocol })`，三分组渲染（回复决策 / 理解 / 运营依据），空字段/空组不渲染。

- [ ] **Step 1: 加 TS 类型**

先 Read `frontend/src/types/index.ts:288-304`（`DecisionReview` 定义）确认现状。在 `DecisionReview` 定义**上方**加：

```ts
export type AutonomyProtocol = {
  userUnderstanding?: string;
  relationshipRead?: string;
  operationGoal?: string;
  knowledgeNeedReason?: string;
  memoryUpdateReason?: string;
  riskSelfCheck?: string;
  selfCritique?: string;
  whyShouldReply?: string;
  whySkipReply?: string;
};
```

在 `DecisionReview` 类型体内 `createdAt?: string;` 之前加一行：

```ts
  autonomyProtocol?: AutonomyProtocol | null;
```

- [ ] **Step 2: 写 AutonomyProtocolView 组件失败单测（TDD）**

Create `frontend/src/__tests__/features/user-ops/autonomyProtocolView.test.tsx`：

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AutonomyProtocolView } from "../../../features/user-ops/cockpit/drilldowns/ConversationReviewView";
import type { AutonomyProtocol } from "../../../types";

const full: AutonomyProtocol = {
  userUnderstanding: "用户在比较两款方案价格",
  relationshipRead: "评估期，信任中等",
  operationGoal: "推进方案确认",
  knowledgeNeedReason: "需引用已核实报价",
  memoryUpdateReason: "记录预算区间",
  riskSelfCheck: "不对未验证功能承诺",
  selfCritique: "本轮放慢确认节奏",
  whyShouldReply: "用户主动询问差异，及时回应",
  whySkipReply: "",
};

describe("AutonomyProtocolView", () => {
  it("三组字段与值渲染，非空字段可见", () => {
    render(<AutonomyProtocolView protocol={full} />);
    expect(screen.getByText("回复决策")).toBeInTheDocument();
    expect(screen.getByText("理解")).toBeInTheDocument();
    expect(screen.getByText("运营依据")).toBeInTheDocument();
    expect(screen.getByText(/用户主动询问差异/)).toBeInTheDocument();
    expect(screen.getByText(/本轮放慢确认节奏/)).toBeInTheDocument();
    expect(screen.getByText(/推进方案确认/)).toBeInTheDocument();
  });

  it("空字段不渲染其标签（whySkipReply 空 → 不显）", () => {
    render(<AutonomyProtocolView protocol={full} />);
    expect(screen.queryByText("为何不回复")).toBeNull();
    expect(screen.getByText("为何回复")).toBeInTheDocument();
  });

  it("整组全空则该组不渲染", () => {
    const only: AutonomyProtocol = { whyShouldReply: "及时回应" };
    render(<AutonomyProtocolView protocol={only} />);
    expect(screen.getByText("回复决策")).toBeInTheDocument();
    expect(screen.queryByText("理解")).toBeNull();
    expect(screen.queryByText("运营依据")).toBeNull();
  });
});
```

- [ ] **Step 3: 跑测试确认失败（未导出）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/wiki-three-p0/frontend" && npx vitest run src/__tests__/features/user-ops/autonomyProtocolView.test.tsx 2>&1 | tail -12`
Expected: 失败（`AutonomyProtocolView` 未导出）。

- [ ] **Step 4: 实现 AutonomyProtocolView + 挂载到 ReviewItem**

先 Read `frontend/src/features/user-ops/cockpit/drilldowns/ConversationReviewView.tsx` 全文确认 `ReviewItem` 的 `hasDetail`/展开区结构与 `import type { DecisionReview, Message }`。

在 import 段把类型 import 扩为 `import type { AutonomyProtocol, DecisionReview, Message } from "../../../../types";`。

在文件内（`ReviewItem` 之前）加分组数据 + 组件：

```tsx
const PROTOCOL_GROUPS: Array<{ title: string; fields: Array<[keyof AutonomyProtocol, string]> }> = [
  { title: "回复决策", fields: [["whyShouldReply", "为何回复"], ["whySkipReply", "为何不回复"], ["selfCritique", "自我批判"]] },
  { title: "理解", fields: [["userUnderstanding", "用户理解"], ["relationshipRead", "关系解读"], ["operationGoal", "运营目标"]] },
  { title: "运营依据", fields: [["knowledgeNeedReason", "知识需求"], ["memoryUpdateReason", "记忆更新理由"], ["riskSelfCheck", "风险自查"]] },
];

export function AutonomyProtocolView({ protocol }: { protocol: AutonomyProtocol }) {
  return (
    <div className={styles.protocolSection}>
      <div className={styles.protocolHeading}>AI 内心独白</div>
      {PROTOCOL_GROUPS.map((group) => {
        const rows = group.fields.filter(([key]) => (protocol[key] ?? "").trim() !== "");
        if (rows.length === 0) return null;
        return (
          <div key={group.title} className={styles.protocolGroup}>
            <div className={styles.protocolGroupTitle}>{group.title}</div>
            {rows.map(([key, label]) => (
              <div key={key} className={styles.protocolField}>
                <span className={styles.protocolLabel}>{label}</span>
                <p className={styles.protocolText}>{protocol[key]}</p>
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}
```

在 `ReviewItem` 内：新增 `const protocol = review.autonomyProtocol;` 与 `const hasProtocol = !!protocol && PROTOCOL_GROUPS.some((g) => g.fields.some(([k]) => (protocol[k] ?? "").trim() !== ""));`。把展开门 `hasDetail` 改为 `hasDetail || hasProtocol`（保持原有 finalReviewStatus/scores/risks 逻辑不变，只 OR 进 protocol）。在展开区 `{expanded && ...}` 块内、现有 reviewDetail 之后加：

```tsx
          {hasProtocol && protocol && <AutonomyProtocolView protocol={protocol} />}
```

> `autonomyProtocol` 为 `null`/`undefined` 或全空 → `hasProtocol=false` → 不渲染该区（优雅降级）。措辞（标题「AI 内心独白」、组名、字段标签）均不含禁词。

- [ ] **Step 5: 加 CSS Module 样式**

先 Read `frontend/src/features/user-ops/cockpit/cockpit.module.css` 顶部确认可用 tokens（如 `--ai`/`--hairline`/`--ink-2`/`--muted`/`--surface-page`/`--r-sm`）。在文件末尾追加（无硬编码 hex，token 名以实际存在的为准）：

```css
.protocolSection { display: flex; flex-direction: column; gap: 10px; margin-top: 10px; padding-top: 10px; border-top: 1px solid var(--hairline); }
.protocolHeading { font-size: 12px; font-weight: 640; color: var(--ai); }
.protocolGroup { display: flex; flex-direction: column; gap: 6px; }
.protocolGroupTitle { font-size: 12px; color: var(--muted); }
.protocolField { display: flex; flex-direction: column; gap: 2px; }
.protocolLabel { font-size: 12px; color: var(--ink-2); }
.protocolText { margin: 0; font-size: 13px; color: var(--ink-1); white-space: pre-wrap; }
```

- [ ] **Step 6: 跑组件单测转绿 + tsc + 全 user-ops vitest**

Run:
```
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/wiki-three-p0/frontend"
npx vitest run src/__tests__/features/user-ops/autonomyProtocolView.test.tsx 2>&1 | tail -8
npx tsc --noEmit 2>&1 | tail -6
npx vitest run src/__tests__/features/user-ops 2>&1 | tail -6
```
Expected: 组件单测 3 passed；tsc 0 error；全 user-ops vitest 全绿（不回归）。

- [ ] **Step 7: 禁用词 lint + Commit**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/wiki-three-p0" && bash scripts/check-no-human-takeover.sh <TASK1_HEAD> HEAD 2>&1 | tail -3`（显式传 base = Task 1 的 commit）
Expected: 0 violations。

```bash
git add frontend/src/types/index.ts frontend/src/features/user-ops/cockpit/drilldowns/ConversationReviewView.tsx frontend/src/features/user-ops/cockpit/cockpit.module.css frontend/src/__tests__/features/user-ops/autonomyProtocolView.test.tsx
git commit -m "feat(user-ops): 决策复盘展示 AI 自治协议 9 字段(三组:回复决策/理解/运营依据)"
```

---

## Self-Review

**1. Spec 覆盖**（逐 § 核）：
- §2.1 后端 join 抽 9 字段 + 投影加嵌套 autonomyProtocol → Task 1 ✅
- §2.2 前端三组展示 + 优雅降级 → Task 2 ✅
- §1.5 契约同步（re-bless fixture + CANONICAL_KEYS）→ Task 1 Step 7-8 ✅
- §1.7 优雅降级为硬要求（历史数据 + 管理发送两类）→ 后端 `autonomy_protocol_from_decision` 全空返 None（Task 1 Step 3）+ 前端 hasProtocol 判空（Task 2 Step 4）✅
- §5 YAGNI（不动模型/写入侧/autonomy 聚合/迁移）→ 两 Task 均只读投影 + 展示 ✅
- §6 测试：后端纯函数单测 + 契约快照（本机 cargo test --lib）+ 前端组件 vitest → 覆盖 ✅

**2. Placeholder 扫描**：无 TBD/TODO；每步含完整代码或确切命令；re-bless 用 `UPDATE_SNAPSHOTS=1` 真机制（非占位）。`<BASE>`/`<TASK1_HEAD>` 是 controller 运行时提供的真实 commit（非代码占位），已标注来源。

**3. 类型一致性**：9 字段 camelCase 键名在 Global Constraints、Task 1 KEYS 数组、契约单测 Some 值、Task 2 `AutonomyProtocol` 类型、`PROTOCOL_GROUPS` 五处完全一致。`autonomy_protocol` 参数名（后端 snake）↔ `autonomyProtocol`（JSON/TS camel）↔ `AutonomyProtocol`（TS 类型）命名连贯。`decision_review_json` 4 参签名在 shared.rs 定义、reviews.rs 两调用点、契约单测三处一致。`AutonomyProtocolView` 导出名在 Task 2 实现与单测 import 一致。
