# 统一收件箱 UI 重做 + 前后端全量清黑话 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把统一收件箱漏给运营的后端黑话翻译成中文,并重做收件箱 UI(补全缺失样式 + 折叠式信息密度)。

**Architecture:** 三处根因分层修复——(A) 后端把嵌在拼接串里的英文枚举先映射成中文再拼(字典翻不了的);(B) 扩 active-view 端点下发已有中文 label 的字典;(C) 前端补全枚举翻译层并接入收件箱各卡片,同时补全全部无定义的 CSS 类并做折叠重构。

**Tech Stack:** Rust(Axum)后端;React 19 + Vite + TypeScript + plain CSS(全局导入)前端。

## Global Constraints

- 后端测试基线:`cargo test --lib` ≥ 350 passed / 0 failed;PBT 四文件累计 ≥ 33/0(`scripts/check-baseline.{sh,ps1}`)。
- 禁词闸:新增行(`src/agent/` `src/routes/` `src/evolution/` `frontend/src/`)不得含 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`(`scripts/check-no-human-takeover.{sh,ps1}`)。翻译措辞用 AI 自主口径:「AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文」。
- CSS 绝不用 `.module.css` 副作用导入(Rollup tree-shake 会删光);收件箱样式一律进 `frontend/src/features/ask-human/AskHuman.css`(plain CSS 全局导入)。色值/圆角/描边全走 `tokens.css` 变量,禁硬编码。
- 不改 `ReviewQueue` 泛型逻辑、不改数据流、不改路由契约(除本计划 Task 2 扩 active-view kinds、Task 1/3 拼串中文化外)。
- 测试只 append,不删改旧维度。不为过测试改业务逻辑/阈值。
- 本机磁盘紧张:本地只跑 `cargo test --lib` 与单个 PBT,完整集成测试留 CI。前端 `cd frontend && npm run build` + vitest。

---

## Task 1: 后端 escalation 请示串中文化

截图黑话根因。`escalation/mod.rs:102-105` 把 `blocked_status`(如 `blocked_unverified_product_claim`)和 `risk_level`(如 `low`)直接 `format!` 进给领导看的中文句子。修复:拼串前经映射函数转中文。

**Files:**
- Modify: `src/agent/escalation/mod.rs:102-105`
- Create(映射函数):`src/agent/escalation/labels.rs`
- Modify: `src/agent/escalation/mod.rs`(顶部加 `mod labels;` 或在现有 mod 树注册,见 Step 3)

**Interfaces:**
- Produces:
  - `pub(crate) fn blocked_status_zh(status: &str) -> &'static str`
  - `pub(crate) fn risk_level_zh(level: &str) -> &'static str`

- [ ] **Step 1: 写失败测试**

在 `src/agent/escalation/labels.rs` 末尾建 `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_status_maps_known_values() {
        assert_eq!(blocked_status_zh("blocked_unverified_product_claim"), "产品说法未经核实");
        assert_eq!(blocked_status_zh("blocked_by_safety_guard"), "安全门拦截");
        assert_eq!(blocked_status_zh("held_by_ai_policy"), "AI 策略主动暂缓");
        assert_eq!(blocked_status_zh("ai_waiting_for_more_context"), "AI 等待更多上下文");
    }

    #[test]
    fn blocked_status_unknown_falls_back_to_input() {
        assert_eq!(blocked_status_zh("some_new_status"), "some_new_status");
    }

    #[test]
    fn risk_level_maps_known_values() {
        assert_eq!(risk_level_zh("low"), "低");
        assert_eq!(risk_level_zh("medium"), "中");
        assert_eq!(risk_level_zh("high"), "高");
    }

    #[test]
    fn risk_level_unknown_falls_back_to_input() {
        assert_eq!(risk_level_zh("critical"), "critical");
    }
}
```

- [ ] **Step 2: 建映射函数(先让 labels.rs 存在但函数体最小,验证测试能编译并失败)**

`src/agent/escalation/labels.rs` 顶部:

```rust
//! 请示串里内嵌的内部状态码 → 运营可读中文映射。
//! 这些值嵌在给领导看的自然语言句子里(escalation/mod.rs),前端无法字典翻译,
//! 故在后端拼串前就转中文。未知值回落原字面量(不吞信息)。

/// blocked_status 内部码 → 中文。取值来源:should_escalate_held(logic.rs:333-343)
/// 覆盖 HOLD_CATEGORY_VALUES 三值 + 游离裸串 blocked_unverified_product_claim(logic.rs:337)。
pub(crate) fn blocked_status_zh(status: &str) -> &'static str {
    match status {
        "blocked_unverified_product_claim" => "产品说法未经核实",
        "blocked_by_safety_guard" => "安全门拦截",
        "held_by_ai_policy" => "AI 策略主动暂缓",
        "ai_waiting_for_more_context" => "AI 等待更多上下文",
        _ => "",
    }
}

/// risk_level(low|medium|high,见 types.rs:410)→ 中文。
pub(crate) fn risk_level_zh(level: &str) -> &'static str {
    match level {
        "low" => "低",
        "medium" => "中",
        "high" => "高",
        _ => "",
    }
}
```

注:此时 `_ => ""` 会让「未知回落原值」测试失败——下一步改成回落。先保留 `""` 以确认测试确实在跑。

- [ ] **Step 3: 注册 mod**

确认 `src/agent/escalation/mod.rs` 顶部的子模块声明区(logic/ledger 等 `mod` 处),加一行:

```rust
mod labels;
```

先 Grep 确认现有声明写法:`grep -n "^mod \|^pub(crate) mod \|^pub mod " src/agent/escalation/mod.rs`,照同款可见性追加。

- [ ] **Step 4: 跑测试确认失败**

Run: `cargo test --lib escalation::labels 2>&1 | tail -15`
Expected: `blocked_status_unknown_falls_back_to_input` 与 `risk_level_unknown_falls_back_to_input` FAIL(返回 `""` ≠ 输入)。

- [ ] **Step 5: 改成未知回落原值**

因返回类型是 `&'static str` 无法回落 `&str` 输入,把签名改为返回 `String`,并同步改测试断言用 `String`。更新 `labels.rs`:

```rust
pub(crate) fn blocked_status_zh(status: &str) -> String {
    match status {
        "blocked_unverified_product_claim" => "产品说法未经核实".to_string(),
        "blocked_by_safety_guard" => "安全门拦截".to_string(),
        "held_by_ai_policy" => "AI 策略主动暂缓".to_string(),
        "ai_waiting_for_more_context" => "AI 等待更多上下文".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn risk_level_zh(level: &str) -> String {
    match level {
        "low" => "低".to_string(),
        "medium" => "中".to_string(),
        "high" => "高".to_string(),
        other => other.to_string(),
    }
}
```

同步把 Step 1 测试里的 `assert_eq!(f(...), "字面量")` 改为 `assert_eq!(f(...), "字面量".to_string())`(或 `f(...).as_str(), "字面量"`)。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test --lib escalation::labels 2>&1 | tail -15`
Expected: 6 tests PASS。

- [ ] **Step 7: 改拼串处**

`src/agent/escalation/mod.rs:102-105` 改为:

```rust
    let question = format!(
        "该客户议题触发高风险闸门（{}），AI 暂不自行答复。拟答风险等级：{}。请领导定夺该如何回复。",
        labels::blocked_status_zh(blocked_status),
        labels::risk_level_zh(&final_decision.risk_level),
    );
```

(`blocked_status` 与 `final_decision.risk_level` 的绑定/类型不变,仅包一层映射。确认 `blocked_status` 是 `&str`,`final_decision.risk_level` 是 `String`——按 Read 结果传引用。)

- [ ] **Step 8: 编译 + 禁词闸 + 基线**

Run: `cargo check 2>&1 | tail -5` → 无错误
Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -3` → 0 violations
Run: `cargo test --lib 2>&1 | tail -5` → ≥ 350/0

- [ ] **Step 9: 提交**

```bash
git add src/agent/escalation/labels.rs src/agent/escalation/mod.rs
git commit -m "fix(escalation): 请示串内嵌状态码/风险等级中文化(截图黑话根因)"
```

---

## Task 2: 后端 digest 兜底文案 + inbox 内嵌字段名中文化

**Files:**
- Modify: `src/knowledge_digest/mod.rs:350,354`(兜底拼串)
- Modify: `src/routes/knowledge/digest_inbox.rs:471,487`(中文句里的英文字段名)
- Create(block_reason 映射,供 digest 兜底用):`src/knowledge_digest/labels.rs`

**Interfaces:**
- Produces: `pub(crate) fn block_reason_zh(reason: &str) -> String`

- [ ] **Step 1: 写失败测试**

`src/knowledge_digest/labels.rs`:

```rust
//! digest 兜底文案里内嵌的 final_review_status 拦截码 → 中文。
//! 取值来源:analyze_run_logs 扫描的 4 个状态(knowledge_digest/mod.rs:277-282)。
//! 未知回落原值。

pub(crate) fn block_reason_zh(reason: &str) -> String {
    match reason {
        "blocked_by_required_field" => "必填信息缺失".to_string(),
        "blocked_by_budget" => "本轮算力预算耗尽".to_string(),
        "blocked_unverified_product_claim" => "产品说法未经核实".to_string(),
        "blocked_by_safety_guard" => "安全门拦截".to_string(),
        "unknown" => "未知原因".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_block_reasons() {
        assert_eq!(block_reason_zh("blocked_unverified_product_claim"), "产品说法未经核实");
        assert_eq!(block_reason_zh("blocked_by_required_field"), "必填信息缺失");
        assert_eq!(block_reason_zh("blocked_by_budget"), "本轮算力预算耗尽");
        assert_eq!(block_reason_zh("blocked_by_safety_guard"), "安全门拦截");
    }

    #[test]
    fn unknown_falls_back_to_input() {
        assert_eq!(block_reason_zh("brand_new_reason"), "brand_new_reason");
    }
}
```

- [ ] **Step 2: 注册 mod**

Grep 现有声明:`grep -n "^mod \|^pub.*mod " src/knowledge_digest/mod.rs`,在子模块声明区加 `mod labels;`。

- [ ] **Step 3: 跑测试确认通过(纯新增函数,应直接过)**

Run: `cargo test --lib knowledge_digest::labels 2>&1 | tail -10`
Expected: 2 tests PASS。

- [ ] **Step 4: 改 digest 兜底拼串**

`src/knowledge_digest/mod.rs:350` 与 `:354` 两处相同的 `format!` 改为经映射(两处都改):

```rust
                    format!("AI 观察：该切片在 {} 条 run 上被{}拦截", block_count, labels::block_reason_zh(&top_block_reason))
```

(把英文码 `{}` 换成 `labels::block_reason_zh(&top_block_reason)`;原句「被 {} 拦截」中间的空格去掉一个以贴合中文,可保留亦可。)

- [ ] **Step 5: 改 inbox 内嵌英文字段名**

`src/routes/knowledge/digest_inbox.rs:471`:

```rust
                context_summary: "AI 检测到该切片缺原文出处，无法通过验证。".into(),
```

`src/routes/knowledge/digest_inbox.rs:487`:

```rust
                context_summary: "AI 检测到该切片原文定位锚点为空，需要重新锚定。".into(),
```

- [ ] **Step 6: 编译 + 禁词闸 + 基线**

Run: `cargo check 2>&1 | tail -5` → 无错误
Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -3` → 0 violations
Run: `cargo test --lib 2>&1 | tail -5` → ≥ 350/0

- [ ] **Step 7: 提交**

```bash
git add src/knowledge_digest/labels.rs src/knowledge_digest/mod.rs src/routes/knowledge/digest_inbox.rs
git commit -m "fix(knowledge): digest 兜底文案与 inbox 卡片英文字段名中文化"
```

---

## Task 3: 后端扩 active-view 字典下发范围

`operation_view.rs:54-64` 只下发 `profile_dimensions ∪ {relationship_type, conversation_mode}`。补上 seed 里已有中文 label 的 `objection_type / value_tier / churn_reason / purchase_lifecycle`,前端 `labelFor` 即可翻译,无需新建映射。

**Files:**
- Modify: `src/routes/operation_view.rs:59-64`

**Interfaces:**
- Produces:(端点返回 JSON 的 `taxonomies` 对象多出 4 个 kind,前端消费。)

- [ ] **Step 1: 加恒定 kind 补充**

`src/routes/operation_view.rs`,在现有 `relationship_type` / `conversation_mode` 追加块后(`:64` 之后)加:

```rust
    for extra in ["objection_type", "value_tier", "churn_reason", "purchase_lifecycle"] {
        if !kinds.iter().any(|k| k == extra) {
            kinds.push(extra.to_string());
        }
    }
```

- [ ] **Step 2: 编译**

Run: `cargo check 2>&1 | tail -5` → 无错误

- [ ] **Step 3: 手工验证字典可达(确认 seed 有这些 kind 的 active 值)**

Run: `grep -rn "objection_type\|value_tier\|churn_reason\|purchase_lifecycle" src/db/migrations/ | grep -i "seed\|insert\|kind" | head`
Expected: 四个 kind 在 seed migration 里都有定义(研究已确认:m006 objection_type、m023 value_tier、m021 churn_reason、m020 purchase_lifecycle)。

- [ ] **Step 4: 基线 + 提交**

Run: `cargo test --lib 2>&1 | tail -5` → ≥ 350/0

```bash
git add src/routes/operation_view.rs
git commit -m "feat(operation-view): active-view 补下发 objection_type/value_tier/churn_reason/purchase_lifecycle 字典"
```

---

## Task 4: 前端翻译层补全(reviewLabels 扩字典)

**Files:**
- Modify: `frontend/src/lib/reviewLabels.ts`
- Test: `frontend/src/__tests__/lib/reviewLabels.test.ts`(新建)

**Interfaces:**
- Produces(新增导出,供 Task 5/6 消费):
  - `GAP_SIGNAL_KIND_LABELS: Record<string,string>`
  - `GAP_SIGNAL_SEVERITY_LABELS: Record<string,string>`
  - `GAP_SIGNAL_STATUS_LABELS: Record<string,string>`
  - `GAP_SIGNAL_SOURCE_LABELS: Record<string,string>`
  - `ESCALATION_CATEGORY_LABELS: Record<string,string>`
  - `ESCALATION_VERDICT_LABELS: Record<string,string>`
  - `ESCALATION_RESOLVED_VIA_LABELS: Record<string,string>`
  - `RISK_DIMENSION_LABELS: Record<string,string>`
  - 复用已有 `labelOf(map, value)`。

- [ ] **Step 1: 写失败测试**

`frontend/src/__tests__/lib/reviewLabels.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import {
  GAP_SIGNAL_KIND_LABELS,
  ESCALATION_CATEGORY_LABELS,
  ESCALATION_VERDICT_LABELS,
  RISK_DIMENSION_LABELS,
  labelOf,
} from "../../lib/reviewLabels";

describe("reviewLabels 扩展字典", () => {
  it("gap_signal kind 覆盖 10 类且中文", () => {
    ["orphan","broken_link","missing_chunk","no_outlinks","low_confidence",
     "stale","contradiction","suggestion","dangling_anchor","recall_miss"
    ].forEach((k) => {
      expect(GAP_SIGNAL_KIND_LABELS[k]).toBeTruthy();
      expect(GAP_SIGNAL_KIND_LABELS[k]).not.toBe(k);
    });
  });

  it("escalation category 中文", () => {
    expect(ESCALATION_CATEGORY_LABELS["high_risk_gated"]).toBe("高风险待裁决");
    expect(ESCALATION_CATEGORY_LABELS["out_of_scope_decision"]).toBe("超出职权待决策");
    expect(ESCALATION_CATEGORY_LABELS["stuck_or_undelivered"]).toBe("多轮僵局待介入");
  });

  it("verdict 中文", () => {
    expect(ESCALATION_VERDICT_LABELS["approved"]).toBe("同意");
    expect(ESCALATION_VERDICT_LABELS["delegated_back"]).toBe("授权 AI 自行处理");
  });

  it("风险维度名中文", () => {
    expect(RISK_DIMENSION_LABELS["factRisk"]).toBeTruthy();
    expect(RISK_DIMENSION_LABELS["ProductAccuracyScore"]).toBeTruthy();
  });

  it("labelOf 未知值回落原值", () => {
    expect(labelOf(GAP_SIGNAL_KIND_LABELS, "brand_new")).toBe("brand_new");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/lib/reviewLabels.test.ts 2>&1 | tail -15`
Expected: FAIL(导出不存在)。

- [ ] **Step 3: 扩 reviewLabels.ts**

在 `frontend/src/lib/reviewLabels.ts` 末尾(`labelOf` 之前或之后)追加。取值与中文按研究结论(gap_signal 10 类、severity 含 error/high 按代码实际,非注释):

```ts
// gap_signal(sources_meta.rs:363 裸下发)。kind 10 类 gap_signals.rs 各判定点。
export const GAP_SIGNAL_KIND_LABELS: Record<string, string> = {
  orphan: "孤立知识",
  broken_link: "引用失效",
  missing_chunk: "依赖已归档",
  no_outlinks: "缺关联引用",
  low_confidence: "置信度偏低",
  stale: "时效已过",
  contradiction: "同题冲突",
  suggestion: "建议补完核实",
  dangling_anchor: "出处对不上",
  recall_miss: "知识缺口（答不上）",
};

export const GAP_SIGNAL_SEVERITY_LABELS: Record<string, string> = {
  info: "一般提示",
  warning: "需注意",
  error: "严重",
  high: "高优",
};

export const GAP_SIGNAL_STATUS_LABELS: Record<string, string> = {
  pending: "待处理",
  auto_resolved: "已自动消解",
  llm_resolved: "AI 已消解",
  applied: "已按建议处理",
  dismissed: "已忽略",
};

export const GAP_SIGNAL_SOURCE_LABELS: Record<string, string> = {
  rule: "规则检出",
  llm: "AI 判定",
  recall_trace: "对话追踪",
};

// escalation(principal_escalations.rs / ask_human_inbox.rs)。
export const ESCALATION_CATEGORY_LABELS: Record<string, string> = {
  high_risk_gated: "高风险待裁决",
  out_of_scope_decision: "超出职权待决策",
  stuck_or_undelivered: "多轮僵局待介入",
};

export const ESCALATION_VERDICT_LABELS: Record<string, string> = {
  approved: "同意",
  rejected: "拒绝",
  conditional: "有条件同意",
  deferred: "暂缓待定",
  delegated_back: "授权 AI 自行处理",
};

export const ESCALATION_RESOLVED_VIA_LABELS: Record<string, string> = {
  wechat: "领导微信裁决",
  admin: "后台裁决",
};

// 复核风险维度名(types.rs:1135-1160,含历史别名)。
export const RISK_DIMENSION_LABELS: Record<string, string> = {
  factRisk: "事实可靠度风险",
  hallucination_score: "事实可靠度风险",
  PressureRisk: "压迫感风险",
  pressure_risk: "压迫感风险",
  HumanLikeScore: "真人感评分",
  human_like: "真人感评分",
  EmotionalValue: "情绪价值评分",
  emotional_value: "情绪价值评分",
  ProductAccuracyScore: "产品准确度评分",
  knowledge_grounding_score: "产品准确度评分",
  boundary_privacy_safety: "边界隐私安全评分",
};
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/lib/reviewLabels.test.ts 2>&1 | tail -15`
Expected: 5 tests PASS。

- [ ] **Step 5: 禁词闸(前端新增行受扫)+ 提交**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -3` → 0 violations

```bash
git add frontend/src/lib/reviewLabels.ts frontend/src/__tests__/lib/reviewLabels.test.ts
git commit -m "feat(inbox): 扩 reviewLabels 字典(gap_signal/escalation/风险维度中文)"
```

---

## Task 5: 前端收件箱 UI 骨架重做(去重复标题 + max-width + 折叠壳)

**Files:**
- Modify: `frontend/src/features/ask-human/index.tsx`
- Modify: `frontend/src/features/ask-human/AskHuman.css`
- Test: `frontend/src/__tests__/features/ask-human/InboxRow.collapse.test.tsx`(新建)

**Interfaces:**
- Produces: `InboxRow`(折叠壳组件),供本文件 `renderItem` 使用。签名:

```tsx
function InboxRow(props: {
  badge: { label: string; tone: string };
  title: string;
  preview: string;
  children: React.ReactNode;
}): JSX.Element
```

- [ ] **Step 1: 写失败测试(折叠壳默认折叠,点击展开 children)**

`frontend/src/__tests__/features/ask-human/InboxRow.collapse.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { InboxRow } from "../../../features/ask-human/index";

describe("InboxRow 折叠壳", () => {
  it("默认折叠:children 不渲染,点击展开后渲染", () => {
    render(
      <InboxRow badge={{ label: "请示裁决", tone: "brand" }} title="#EQERR" preview="候选回复人味较好">
        <div>展开详情内容</div>
      </InboxRow>,
    );
    expect(screen.getByText("请示裁决")).toBeInTheDocument();
    expect(screen.getByText("#EQERR")).toBeInTheDocument();
    expect(screen.queryByText("展开详情内容")).toBeNull();
    fireEvent.click(screen.getByText("#EQERR").closest("button")!);
    expect(screen.getByText("展开详情内容")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/InboxRow.collapse.test.tsx 2>&1 | tail -15`
Expected: FAIL(`InboxRow` 未导出)。

- [ ] **Step 3: 在 index.tsx 加 InboxRow 并导出**

`frontend/src/features/ask-human/index.tsx` 顶部 import 加 `useState`(已有 `useCallback, useState` 则复用),新增导出组件:

```tsx
export function InboxRow({
  badge,
  title,
  preview,
  children,
}: {
  badge: { label: string; tone: string };
  title: string;
  preview: string;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="inboxRow">
      <button type="button" className="inboxRowHead" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        <span className={`inboxBadge inboxBadge--${badge.tone}`}>{badge.label}</span>
        <span className="inboxRowTitle">{title}</span>
        {!open && <span className="inboxRowPreview">{preview}</span>}
        <span className="inboxRowChevron">{open ? "▾" : "▸"}</span>
      </button>
      {open && <div className="inboxRowBody">{children}</div>}
    </div>
  );
}
```

- [ ] **Step 4: 去重复标题**

删 `frontend/src/features/ask-human/index.tsx:114` 的 `<h1>统一收件箱</h1>`(频道外壳 Shell.tsx 已渲染标题)。`.askHumanHeader` 保留右侧操作组,CSS 改 `justify-content: flex-end`(Step 6)。

- [ ] **Step 5: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/InboxRow.collapse.test.tsx 2>&1 | tail -15`
Expected: 1 test PASS。

- [ ] **Step 6: 加 max-width + 折叠壳样式 + header 对齐**

`frontend/src/features/ask-human/AskHuman.css`:`.askHumanChannel`(:5-10)加 `max-width: 920px;`。`.askHumanHeader`(:12-17)`justify-content` 改 `flex-end`。文件末尾追加:

```css
/* 折叠式收件箱行 */
.inboxRow {
  border: 1px solid var(--hairline);
  border-radius: var(--r-md, 12px);
  background: var(--surface-1, #fff);
  margin-bottom: 10px;
}
.inboxRowHead {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 12px 14px;
  border: none;
  background: transparent;
  cursor: pointer;
  font: inherit;
  text-align: left;
}
.inboxRowTitle { font-size: 13.5px; font-weight: 650; color: var(--ink-1); flex-shrink: 0; }
.inboxRowPreview {
  font-size: 12.5px;
  color: var(--ink-3, var(--ink-2));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}
.inboxRowChevron { margin-left: auto; color: var(--ink-3, var(--ink-2)); font-size: 12px; }
.inboxRowBody { padding: 4px 14px 16px; border-top: 1px solid var(--hairline); }
.inboxBadge {
  font-size: 11.5px;
  font-weight: 600;
  padding: 2px 9px;
  border-radius: 999px;
  color: var(--ink-1);
  flex-shrink: 0;
}
.inboxBadge--brand { background: var(--fill-brand); }
.inboxBadge--scheduled { background: var(--fill-scheduled); }
.inboxBadge--held { background: var(--fill-held); }
.inboxBadge--blocked { background: var(--fill-blocked); }
.inboxBadge--running { background: var(--fill-running); }
.inboxBadge--neutral { background: var(--fill-inactive); }
```

- [ ] **Step 7: 构建 + 禁词闸 + 提交**

Run: `cd frontend && npm run build 2>&1 | tail -8` → 0 errors
Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -3` → 0 violations

```bash
git add frontend/src/features/ask-human/index.tsx frontend/src/features/ask-human/AskHuman.css frontend/src/__tests__/features/ask-human/InboxRow.collapse.test.tsx
git commit -m "feat(inbox): 收件箱去重复标题 + max-width + 折叠式行壳"
```

---

## Task 6: 卡片样式补全 + 接入折叠壳与翻译字典

补全 `escalationInline*` / `simpleAction*` 无定义样式;把两个 inline 卡片包进 `InboxRow`,并用 Task 4 字典翻译 category/verdict 等。

**Files:**
- Modify: `frontend/src/features/ask-human/index.tsx`(renderInline/renderRich 包 InboxRow)
- Modify: `frontend/src/features/ask-human/inline/EscalationInline.tsx`
- Modify: `frontend/src/features/ask-human/inline/SimpleApproveReject.tsx`
- Modify: `frontend/src/features/ask-human/AskHuman.css`
- Test: `frontend/src/__tests__/features/ask-human/EscalationInline.labels.test.tsx`(新建)

**Interfaces:**
- Consumes: `InboxRow`(Task 5)、`ESCALATION_CATEGORY_LABELS` / `ESCALATION_VERDICT_LABELS` / `labelOf`(Task 4)。

- [ ] **Step 1: 写失败测试(EscalationInline 的 category 显示中文,verdict 下拉中文)**

先 Read `EscalationInline.tsx` 确认 `item.category` 当前直接渲染。测试 `frontend/src/__tests__/features/ask-human/EscalationInline.labels.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { EscalationInline } from "../../../features/ask-human/inline/EscalationInline";
import type { InboxItem } from "../../../lib/inboxApi";

vi.mock("../../../lib/api", () => ({ api: { post: vi.fn() } }));

const item = {
  id: "EQERR",
  source: "principal_escalation",
  title: "请示 #EQERR",
  summary: "候选回复人味较好",
  category: "high_risk_gated",
  contactWxid: "biztest_c9",
  questionForPrincipal: "该客户议题触发高风险闸门（产品说法未经核实）",
} as unknown as InboxItem;

describe("EscalationInline 翻译", () => {
  it("category 显示中文而非英文 id", () => {
    render(<EscalationInline item={item} ctx={{ busy: false, runAction: vi.fn() }} />);
    expect(screen.getByText("高风险待裁决")).toBeInTheDocument();
    expect(screen.queryByText("high_risk_gated")).toBeNull();
  });
  it("verdict 下拉项为中文标签", () => {
    render(<EscalationInline item={item} ctx={{ busy: false, runAction: vi.fn() }} />);
    expect(screen.getByRole("option", { name: "同意" })).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/EscalationInline.labels.test.tsx 2>&1 | tail -15`
Expected: FAIL(显示的是 `high_risk_gated` / 英文 verdict label)。

- [ ] **Step 3: EscalationInline 接字典 + 样式类**

`EscalationInline.tsx`:import 加 `import { ESCALATION_CATEGORY_LABELS, labelOf } from "../../../components/... reviewLabels"`(按实际相对路径:`../../../lib/reviewLabels`)。`VERDICT_OPTIONS` 的 label 已是中文(`批准`等),但为与研究口径统一,改为:

```tsx
const VERDICT_OPTIONS: { value: string; label: string }[] = [
  { value: "approved", label: "同意" },
  { value: "rejected", label: "拒绝" },
  { value: "conditional", label: "有条件同意" },
  { value: "deferred", label: "暂缓待定" },
  { value: "delegated_back", label: "授权 AI 自行处理" },
];
```

`:66-71` 的 `<dd>{item.category}</dd>` 改为:

```tsx
              <dd>{labelOf(ESCALATION_CATEGORY_LABELS, item.category)}</dd>
```

- [ ] **Step 4: 补 escalationInline / simpleAction 样式**

`AskHuman.css` 末尾追加(元信息网格复用 resolvedEscMeta 模式;表单/操作分区):

```css
/* 请示裁决卡内部 */
.escalationInlineTitle { font-size: 14px; font-weight: 650; color: var(--ink-1); }
.escalationInlineSummary { font-size: 13px; color: var(--ink-2); margin: 6px 0 10px; line-height: 1.5; }
.escalationInlineMeta {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 4px 12px;
  margin: 0 0 12px;
  font-size: 12.5px;
}
.escalationInlineMeta dt { color: var(--ink-3, var(--ink-2)); font-weight: 600; }
.escalationInlineMeta dd { margin: 0; color: var(--ink-1); }
.escalationInline label { display: block; font-size: 12.5px; color: var(--ink-2); margin: 8px 0 4px; }
.escalationInline select,
.escalationInline textarea,
.escalationInline input {
  width: 100%;
  border: 1px solid var(--hairline);
  border-radius: var(--r-sm);
  padding: 8px 10px;
  font: inherit;
  box-sizing: border-box;
}
.escalationInline textarea { min-height: 88px; resize: vertical; }
.escalationInlineActions { margin-top: 12px; }
.escalationInlineReassign {
  display: flex;
  gap: 8px;
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px dashed var(--hairline);
}
.escalationInlineReassign input { flex: 1; }

/* 简单通过/拒绝卡内部 */
.simpleActionTitle { font-size: 14px; font-weight: 650; color: var(--ink-1); }
.simpleActionSummary { font-size: 13px; color: var(--ink-2); margin: 6px 0; line-height: 1.5; }
.simpleActionEvidence { display: grid; gap: 2px; }
.simpleActionButtons { display: flex; gap: 8px; margin-top: 12px; }
```

- [ ] **Step 5: renderInline/renderRich 包进 InboxRow(index.tsx)**

`index.tsx` 的 `renderItem`(:173-181)改为用 `InboxRow` 包裹,badge 按 source 取 SOURCE_META 的 label + tone。加一个 tone 映射:

```tsx
const SOURCE_TONE: Record<string, string> = {
  principal_escalation: "brand",
  knowledge_review: "scheduled",
  taxonomy_candidate: "neutral",
  relationship_suggestion: "neutral",
  gap_signal: "held",
  profile_risky: "blocked",
  evolution_proposal: "running",
  lessons_learned: "neutral",
};
```

`renderItem`:

```tsx
            renderItem={(item, ctx) => {
              const meta = SOURCE_META.find((m) => m.source === item.source);
              return (
                <InboxRow
                  badge={{ label: meta?.label ?? item.source, tone: SOURCE_TONE[item.source] ?? "neutral" }}
                  title={item.title}
                  preview={item.summary ?? ""}
                >
                  {item.actionKind === "rich"
                    ? renderRich(item, () => refreshAll())
                    : renderInline(item, ctx)}
                </InboxRow>
              );
            }}
```

(删除原 `askHumanRichRow` / `askHumanInlineRow` 外层 div,改由 InboxRow 提供边框;这两个类的 CSS 可留作兜底不删。)

- [ ] **Step 6: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/EscalationInline.labels.test.tsx 2>&1 | tail -15`
Expected: 2 tests PASS。

- [ ] **Step 7: 全量前端测试 + 构建 + 禁词闸**

Run: `cd frontend && npx vitest run 2>&1 | tail -15` → 全绿(含既有 ReviewQueue/ask-human 测试不回归)
Run: `cd frontend && npm run build 2>&1 | tail -8` → 0 errors
Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -3` → 0 violations

- [ ] **Step 8: 提交**

```bash
git add frontend/src/features/ask-human/index.tsx frontend/src/features/ask-human/inline/EscalationInline.tsx frontend/src/features/ask-human/inline/SimpleApproveReject.tsx frontend/src/features/ask-human/AskHuman.css frontend/src/__tests__/features/ask-human/EscalationInline.labels.test.tsx
git commit -m "feat(inbox): 卡片样式补全 + 接入折叠壳与中文字典"
```

---

## Task 7: 知识面板卡片样式补全(可选,视时间)

`ChunkReviewCard`(`chunkReview*`)、`ProfilePublishCard`(`profilePublish*`)、`LessonPromoteCard`(`lessonPromote*`)同样类名无样式。补全其 CSS 到 `AskHuman.css`。因这些卡走 rich 分支、依赖后端数据(chunkId 等)难在无栈下实测,只补样式不改逻辑。

**Files:**
- Modify: `frontend/src/features/ask-human/AskHuman.css`

- [ ] **Step 1: Read 三张卡片确认类名结构**

Read `ChunkReviewCard.tsx` / `ProfilePublishCard.tsx` / `LessonPromoteCard.tsx`,列出每个 className 的用途(容器/标题/正文/操作)。

- [ ] **Step 2: 补样式**

按各卡片结构补 `.chunkReviewCard`/`.chunkReviewTitle`/`.chunkReviewMeta`/`.chunkReviewActions`、`.profilePublishCard` 系列、`.lessonPromote*` 系列,复用统一的卡片内标题/正文/操作分区规范(字号、间距、描边全走 tokens)。因篇幅,实现时照 Task 6 的 escalationInline 同款规范套用。

- [ ] **Step 3: 构建 + 提交**

Run: `cd frontend && npm run build 2>&1 | tail -8` → 0 errors

```bash
git add frontend/src/features/ask-human/AskHuman.css
git commit -m "feat(inbox): 补全知识面板 rich 卡片样式"
```

---

## Self-Review 记录

- **Spec 覆盖**:①escalation 拼串→Task1;②digest/inbox 内嵌→Task2;③active-view 扩下发→Task3;④前端字典→Task4;⑤UI 骨架(去重标题/max-width/折叠)→Task5;⑥卡片样式+接字典→Task6;⑦知识卡样式→Task7。`blocked_unverified_product_claim` 常量提升——Task1 映射已覆盖该值的翻译;是否物理提升为 Rust 常量属可选加固,研究不足#2,**本计划以映射覆盖为准,不强行改常量定义**(避免牵动 logic.rs/gateway.rs 判定点,超范围)。
- **占位符扫描**:Task7 Step2 是唯一「照同款套用」的概括步骤(因三卡片结构需实现时 Read 确认),已标注为可选、并给出复用规范;其余步骤均含真代码。
- **类型一致**:`InboxRow` 签名(Task5)与 Task6 调用一致;`labelOf`/字典导出名(Task4)与 Task6 import 一致;`blocked_status_zh`/`risk_level_zh` 返回 `String`(Task1 Step5 修正)与调用点一致。

## 排除项(单独议,不在本计划)

- 情绪价值阈值三处不一致(后端默认6/前端展示5/CLAUDE.md<5)——涉改阈值红线。
- 请示 severity 恒 high(`ask_human_inbox.rs:75`)——排序分流独立 UX。
