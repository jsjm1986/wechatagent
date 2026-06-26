# 前后端业务对齐 批次2(通用化前端断裂)实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把前端写死的销售域语义(标签/维度/枚举)换成 active profile 驱动的字典翻译,让非销售域(情感陪伴/正式咨询/数字分身)在前端正确显形,并补两处必要后端 gap。

**Architecture:** 9 条审查缺口分三组——纯前端字典翻译(A4/C7/E13)、纯前端+类型添加(D7/D8,后端字段已存在)、需后端配合(A5 补 conversation_mode 字典 / C8 关联 AgentRunLog 补 emit / D6 字典两 flag / E10 关系建议富投影)。取数走已建好的 `GET /api/operation/active-view`(返回 `{dimensions, taxonomies:{kind:[{id,label}]}}`)+ 前端 `labelFor` 诚实三态。

**Tech Stack:** Rust(Axum)+MongoDB 后端;React 19 + TypeScript + Zustand + Vite 前端;vitest + @testing-library/react 前端测试;cargo test 后端测试。

## Global Constraints

每个 task 的需求都隐含包含本节(逐条 verbatim 自 spec)。

- 子 agent 一律 `model:"opus"`;所有对话/总结回复中文。
- **无人工接管 CI lint**:`src/agent|routes|evolution` + `frontend/src` 新增行禁词 `人工 / 接管 / takeover / hand-off / 人工接管 / 转人工`(测试目录除外)。本批 C7/C8 标签措辞必须用 AI-internal 名(如"AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文"),绝不用"人工接管"。
- **测试基线不回退**:`cargo test --lib` ≥350 passed/0 failed;4 个 PBT 文件累计 ≥33/0(state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter);`RUSTFLAGS=-Dwarnings cargo check --tests` 须 0 警告 0 错误。
- **本地资源纪律**:本地只跑 `cargo test --lib` + 单个 PBT 文件;完整集成测(`tests/` 下 `#[ignore]`)留 GitHub CI。磁盘紧时先删 `target/debug/incremental`。
- **测试只增量叠加**:扩测试套件只 append,绝不删改旧维度/旧弧/旧金标。
- **AI 永不自动验证知识**(draft + needs_review 红线)——本批不碰知识 ingest;D6 taxonomy seed 遵循既有 taxonomy 域规范。
- **前端遵守现有设计系统**:用 `frontend/src/styles/tokens.css` 真实变量、`.module.css`(避免裸 import 被 tree-shake)、4 级层级、蓝=主操作专属、紫=AI 身份专属(见 `docs/frontend-design-system.md`)。写前端前先读该 doc + 参照现有 `.module.css`。
- **closed-set 枚举在 DB 写点校验**:C8 的 finalReviewStatus(10 项)/holdCategory(3 项)、D6 的两 flag。
- **后端 DomainProfile 序列化 snake_case**(`serde_json::to_value`,无 rename_all),故前端 types 对应字段用 snake_case;但 `AgentDecisionReview` 的 json 出口(decision_review_json)用 camelCase 手写键;taxonomy 请求体(CreateTaxonomyRequest/PatchTaxonomyRequest)是 `#[serde(rename_all="camelCase")]`——改后端 set_doc 用 camelCase 键(`value.isTerminal`/`value.isReactivationTarget`,对齐既有 `value.displayName`)。
- **git**:仅在用户要求时提交;只 `git add` 具名文件,绝不 `git add -A`;commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`;破坏性 gitops 须显式授权。
- **基线**:批次1(PR#44)已合并 main(merge `9d78282`)。本计划所有行号基于该 main。

## 已 grep 实证的关键事实(plan 代码块依据)

- `FINAL_REVIEW_STATUS_VALUES`(10 项,`src/agent/run_envelope.rs:67-78`):`approved` / `revision_applied_approved` / `revision_failed` / `held_by_ai_policy` / `blocked_by_safety_guard` / `ai_waiting_for_more_context` / `blocked_by_required_field` / `blocked_by_budget` / `blocked_unverified_product_claim` / `legacy_mode_unchecked`。
- `HOLD_CATEGORY_VALUES`(3 项,`src/agent/types.rs:1227-1231`):`held_by_ai_policy` / `blocked_by_safety_guard` / `ai_waiting_for_more_context`。
- D7 五字段后端已存在:`transaction_facts_enabled: bool`(models.rs:1792) / `reviewer_orientation: Option<ReviewerOrientation>`(:1859,结构体 models.rs:1939 含 `review_focus`/`balance_principle`/`pressure_few_shot` 三 Option<String>) / `mode_gate_policy_override: Option<String>`(:1866) / `trajectory_dimensions: Vec<TrajectoryDimension>`(:1830,元素 models.rs:4067 = `{kind, display_name}`) / `debounce_window_ms_override: Option<u64>`(:1834)。
- D8 后端已存在:`per_relationship_operation_mode: Option<BTreeMap<String, OperationMode>>`(models.rs:1763)。
- E10 证据字段已全持久化:`RelationshipTypeSuggestion`(models.rs:2823-2845)含 `evidence: Option<String>`(:2832) / `confidence: i32`(:2834) / `occurrences: i32`(:2838) / `contact_id: String`(:2828)。
- C8 数据源:`decision_review_json`(shared.rs:1053)输入 `AgentDecisionReview` 无这两字段;`send_gateway_result` doc(`SendGatewayResult` types.rs:1496)只有 `status`/`policy_blocks`,**无** final_review_status/hold_category。两值在 **AgentRunLog**:`final_review_status` 顶层、`hold_category` 在 `review` doc 内。`decision_review_json` 由 `reviews.rs:62/85` 调用(均有 `State`,可关联查 AgentRunLog)。
- 前端取数基础设施已就位:`profileStore.labelFor(taxonomies, kind, value)`(`frontend/src/stores/profileStore.ts:23-29`)诚实三态 `ok`/`unknown_value`/`no_dict`;`store.dimensions`(ProfileDimensionView[])/`store.taxonomies`(TaxonomyMap)已加载。`PlannerViewSection`(`frontend/src/features/user-ops/legacy.tsx:2029-2096`)已用 labelFor 翻译 customer_stage 单维(样板)。
- A5 机制:`operation_view.rs:54-61` 的 `kinds` 集 = profile_dimensions ∪ `relationship_type`,逐 kind 调 `dimension_values_with_labels` 建字典。conversation_mode 只需(1)seed `kind="conversation_mode"` taxonomy(m006 模式)+(2)`kinds` 集加 `"conversation_mode"`。
- C7 编辑点:`autonomy/index.tsx` 用 `import styles from "./Autonomy.module.css"`(:7)、文件头 :13 已声明"AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文"措辞;逐行渲染在 RevisionRow `item.finalReviewStatus`(:360)/`item.holdCategory`(:361)。
- D7/D8 编辑点样板:`system-strategy/index.tsx` 的 ProfileEditor 用 `update({...})` 局部 merge helper + `<details className={styles.advanced}>` 折叠面板 + `styles.inlineCheckbox`/`styles.formGrid`(operation_mode 编辑器 :1891-1947 为样板)。`strategyStore.editDomainProfile`(:303-330)拷贝列表决定编辑回填,新字段须加入。

## File Structure

**前端**
- `frontend/src/lib/reviewLabels.ts`(新建)— C7/C8 共用的 FINAL_REVIEW_STATUS_LABELS / HOLD_CATEGORY_LABELS 闭集常量 map + `labelOf` helper。
- `frontend/src/features/operations/index.tsx`(改)— E13 formatScores 动态遍历。
- `frontend/src/features/autonomy/index.tsx`(改)— C7 逐行枚举中文化。
- `frontend/src/features/user-ops/legacy.tsx`(改)— A4 多维度看板 + A5 conversationModeLabel 改 labelFor。
- `frontend/src/types/index.ts`(改)— D7 五字段、D8 map、C8 两字段、E10 InboxItem 是在 inboxApi.ts。
- `frontend/src/lib/inboxApi.ts`(改)— E10 InboxItem 加 evidence/confidence/occurrences。
- `frontend/src/features/system-strategy/index.tsx`(改)— D6 字典两 flag 表单、D7 profile 高级字段编辑、D8 per_relationship map 编辑。
- `frontend/src/stores/strategyStore.ts`(改)— D7/D8 editDomainProfile 拷贝列表加字段。
- `frontend/src/features/ask-human/inline/SimpleApproveReject.tsx`(改)— E10 富展示。

**后端**
- `src/db/migrations/`(新增 migration)— A5 seed `kind="conversation_mode"` taxonomy。
- `src/routes/operation_view.rs`(改)— A5 kinds 集加 conversation_mode。
- `src/routes/shared.rs` + `src/routes/reviews.rs`(改)— C8 decision_review_json 关联 AgentRunLog 补 emit。
- `src/routes/admin_taxonomies.rs`(改)— D6 create/patch 两 flag。
- `src/routes/ask_human_inbox.rs`(改)— E10 collect_relationship_suggestions 富投影。

---

## 组一:纯前端字典翻译扩展(零后端)

### Task 1: E13 — formatScores 动态遍历(隐私维度显形)

**Files:**
- Modify: `frontend/src/features/operations/index.tsx:42-50`
- Test: `frontend/src/__tests__/features/operations/operations.test.tsx`(已存在,append)

**Interfaces:**
- Consumes: 无(纯前端组件)。
- Produces: `formatScores(scores: Record<string, number>): string` —— 签名不变,行为从写死 5-key 白名单改为动态遍历 + 中文标签。

当前代码(`operations/index.tsx:42-50`):
```tsx
function formatScores(scores: Record<string, number>) {
  const keys = ["humanLike", "emotionalValue", "hallucinationScore", "knowledgeGroundingScore", "pressureRisk"];
  return (
    keys
      .filter((key) => scores[key] !== undefined)
      .map((key) => `${key}:${scores[key]}`)
      .join(" / ") || "-"
  );
}
```
问题:`boundaryPrivacySafety`(后端 `ReviewScores` types.rs 已随 scores 下发)不在白名单 → 静默丢弃。

- [ ] **Step 1: 写失败测试**

在 `operations.test.tsx` append(文件已有 import,若缺则加 `import { formatScores } from "../../../features/operations";` —— 注意 formatScores 当前未 export,见 Step 3 须先 export):

```tsx
import { formatScores } from "../../../features/operations";

describe("formatScores 动态遍历(E13)", () => {
  it("渲染 boundaryPrivacySafety 隐私维度(不再被白名单丢弃)", () => {
    const out = formatScores({ humanLike: 8, boundaryPrivacySafety: 7 });
    expect(out).toContain("拟人度:8");
    expect(out).toContain("隐私边界:7");
  });
  it("未知 key 回落原始 key 名,不吞", () => {
    const out = formatScores({ someNewDimension: 5 });
    expect(out).toContain("someNewDimension:5");
  });
  it("空 scores 回落 -", () => {
    expect(formatScores({})).toBe("-");
  });
});
```

- [ ] **Step 2: 运行测试,确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/operations/operations.test.tsx`
Expected: FAIL —— `formatScores` 未 export(或 boundaryPrivacySafety 不出现,断言落空)。

- [ ] **Step 3: 实现**

把 `operations/index.tsx:42-50` 替换为(并 `export` 该函数供测试导入):

```tsx
const SCORE_LABELS: Record<string, string> = {
  humanLike: "拟人度",
  emotionalValue: "情绪价值",
  hallucinationScore: "幻觉风险",
  knowledgeGroundingScore: "知识接地",
  pressureRisk: "压迫风险",
  boundaryPrivacySafety: "隐私边界",
};

export function formatScores(scores: Record<string, number>) {
  const entries = Object.entries(scores ?? {}).filter(([, v]) => v !== undefined && v !== null);
  return (
    entries
      .map(([key, v]) => `${SCORE_LABELS[key] ?? key}:${v}`)
      .join(" / ") || "-"
  );
}
```

- [ ] **Step 4: 运行测试,确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/operations/operations.test.tsx`
Expected: PASS(含原有该文件用例不回归)。

- [ ] **Step 5: 全量前端测 + commit**

Run: `cd frontend && npx vitest run`
Expected: 全绿(批次1 基线 200/200 + 本 task 新增)。

```bash
git add frontend/src/features/operations/index.tsx frontend/src/__tests__/features/operations/operations.test.tsx
git commit -m "feat(operations): formatScores 动态遍历显形 boundaryPrivacySafety 隐私维度(E13)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: C7 — 共享 reviewLabels 模块 + autonomy 逐行枚举中文化

**Files:**
- Create: `frontend/src/lib/reviewLabels.ts`
- Modify: `frontend/src/features/autonomy/index.tsx`(RevisionRow :360-361)
- Test: `frontend/src/__tests__/lib/reviewLabels.test.ts`(新建)

**Interfaces:**
- Consumes: 无。
- Produces:
  - `FINAL_REVIEW_STATUS_LABELS: Record<string, string>` —— 10 项闭集(对齐 `run_envelope.rs:67-78`)。
  - `HOLD_CATEGORY_LABELS: Record<string, string>` —— 3 项闭集(对齐 `types.rs:1227-1231`)。
  - `labelOf(map: Record<string,string>, value: string | null | undefined): string` —— `map[value] ?? value ?? "—"`,未知值回落原始值不吞。
  - **Task 8(C8)复用本模块**,勿重复定义。

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/lib/reviewLabels.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { FINAL_REVIEW_STATUS_LABELS, HOLD_CATEGORY_LABELS, labelOf } from "../../lib/reviewLabels";

describe("reviewLabels 闭集标签(C7/C8 共用)", () => {
  it("FINAL_REVIEW_STATUS_LABELS 覆盖 10 项闭集", () => {
    const keys = [
      "approved", "revision_applied_approved", "revision_failed",
      "held_by_ai_policy", "blocked_by_safety_guard", "ai_waiting_for_more_context",
      "blocked_by_required_field", "blocked_by_budget", "blocked_unverified_product_claim",
      "legacy_mode_unchecked",
    ];
    for (const k of keys) {
      expect(FINAL_REVIEW_STATUS_LABELS[k], `缺 ${k}`).toBeTruthy();
    }
  });
  it("HOLD_CATEGORY_LABELS 覆盖 3 项闭集", () => {
    for (const k of ["held_by_ai_policy", "blocked_by_safety_guard", "ai_waiting_for_more_context"]) {
      expect(HOLD_CATEGORY_LABELS[k], `缺 ${k}`).toBeTruthy();
    }
  });
  it("labelOf 已知值翻译、未知值回落原始、空值回落 —", () => {
    expect(labelOf(FINAL_REVIEW_STATUS_LABELS, "approved")).toBe("已通过");
    expect(labelOf(FINAL_REVIEW_STATUS_LABELS, "weird_value")).toBe("weird_value");
    expect(labelOf(FINAL_REVIEW_STATUS_LABELS, null)).toBe("—");
  });
});
```

- [ ] **Step 2: 运行测试,确认失败**

Run: `cd frontend && npx vitest run src/__tests__/lib/reviewLabels.test.ts`
Expected: FAIL —— 模块不存在。

- [ ] **Step 3: 实现 reviewLabels.ts**

新建 `frontend/src/lib/reviewLabels.ts`。**措辞严守无人工接管 lint**(用 AI-internal 名):

```ts
// C7/C8 共用:最终复核状态 + 暂缓类别的中文闭集标签。
// 闭集对齐后端:FINAL_REVIEW_STATUS_VALUES(run_envelope.rs:67-78,10 项)、
// HOLD_CATEGORY_VALUES(types.rs:1227-1231,3 项)。措辞用 AI 自主语义,
// 禁"人工接管/转人工"(CI lint 在 frontend/src 新增行阻断)。

export const FINAL_REVIEW_STATUS_LABELS: Record<string, string> = {
  approved: "已通过",
  revision_applied_approved: "改写后通过",
  revision_failed: "改写未达标",
  held_by_ai_policy: "AI 策略主动暂缓",
  blocked_by_safety_guard: "安全门拦截",
  ai_waiting_for_more_context: "AI 等待更多上下文",
  blocked_by_required_field: "必填信息缺失拦截",
  blocked_by_budget: "预算耗尽暂缓",
  blocked_unverified_product_claim: "未验证产品声明拦截",
  legacy_mode_unchecked: "兼容模式未校验",
};

export const HOLD_CATEGORY_LABELS: Record<string, string> = {
  held_by_ai_policy: "AI 策略主动暂缓",
  blocked_by_safety_guard: "安全门拦截",
  ai_waiting_for_more_context: "AI 等待更多上下文",
};

export function labelOf(
  map: Record<string, string>,
  value: string | null | undefined,
): string {
  if (value === null || value === undefined || value === "") return "—";
  return map[value] ?? value;
}
```

- [ ] **Step 4: 运行测试,确认通过**

Run: `cd frontend && npx vitest run src/__tests__/lib/reviewLabels.test.ts`
Expected: PASS。

- [ ] **Step 5: 接入 autonomy 逐行渲染**

在 `autonomy/index.tsx` 顶部 import 区加:
```tsx
import { FINAL_REVIEW_STATUS_LABELS, HOLD_CATEGORY_LABELS, labelOf } from "../../lib/reviewLabels";
```

把 RevisionRow 的 :360-361:
```tsx
        <td>{item.finalReviewStatus}</td>
        <td>{item.holdCategory || "—"}</td>
```
改为:
```tsx
        <td>{labelOf(FINAL_REVIEW_STATUS_LABELS, item.finalReviewStatus)}</td>
        <td>{labelOf(HOLD_CATEGORY_LABELS, item.holdCategory)}</td>
```

- [ ] **Step 6: autonomy 组件测(append 一条逐行中文化断言)**

在 `frontend/src/__tests__/features/autonomy/` 既有 autonomy 测试文件(若无则新建 `autonomy.test.tsx`)append:断言渲染含 revision 记录、finalReviewStatus="held_by_ai_policy" 时表格出现"AI 策略主动暂缓"中文。(实现期按既有 autonomy 测试 mock 结构对齐;若无既有测试则 mock `/api/outcomes/autonomy` 返回一条 revision item。)

- [ ] **Step 7: 全量前端测 + commit**

Run: `cd frontend && npx vitest run`
Expected: 全绿。

```bash
git add frontend/src/lib/reviewLabels.ts frontend/src/__tests__/lib/reviewLabels.test.ts frontend/src/features/autonomy/index.tsx frontend/src/__tests__/features/autonomy/
git commit -m "feat(autonomy): 逐行 finalReviewStatus/holdCategory 中文化 + 抽 reviewLabels 共享模块(C7)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: A4 — PlannerViewSection 多维度画像看板

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`(PlannerViewSection :2029-2096)
- Test: `frontend/src/__tests__/` 下 PlannerViewSection 既有/新建测试

**Interfaces:**
- Consumes: `useProfileStore` 的 `dimensions: ProfileDimensionView[]`(`{kind, displayName, participatesInDecision}`)+ `taxonomies: TaxonomyMap`;`labelFor(taxonomies, kind, value): {text, status}`(profileStore.ts:23);`contact.domainAttributes`(Record<string,unknown>)。
- Produces: 无对外接口(组件内渲染)。

背景:`PlannerViewSection`(legacy.tsx:2029-2096)已用 `labelFor` 翻译 `customer_stage` 单维(:2042 stageResult + :2059-2080 三态渲染),但 `store.dimensions` 列表 0 消费。本 task 增加一个"画像维度"区块,遍历 dimensions 渲染**除 customer_stage 外**的其余维度(customer_stage 既有专属"运营阶段"行保留,因它额外展示 stageUpdatedAt 时间)。

- [ ] **Step 1: 写失败测试**

新建/append `frontend/src/__tests__/features/user-ops/plannerView.test.tsx`(参照既有 user-ops 测试 mock useProfileStore 的方式):

```tsx
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { PlannerViewSection } from "../../../features/user-ops/legacy";
import { useProfileStore } from "../../../stores/profileStore";
import type { Contact } from "../../../types";

function setStore(dimensions: any[], taxonomies: any) {
  useProfileStore.setState({ dimensions, taxonomies } as any);
}

const baseContact = {
  wxid: "wx1",
  domainAttributes: { customer_stage: "first_contact", intent_level: "high" },
  domainAttributesUpdatedAt: "2026-06-26T00:00:00Z",
  commitments: [],
} as unknown as Contact;

describe("PlannerViewSection 多维度看板(A4)", () => {
  beforeEach(() => setStore([], {}));

  it("渲染 dimensions 中非 customer_stage 维度,经 labelFor 翻译为中文", () => {
    setStore(
      [
        { kind: "customer_stage", displayName: "客户阶段", participatesInDecision: true },
        { kind: "intent_level", displayName: "意向程度", participatesInDecision: true },
      ],
      {
        customer_stage: [{ id: "first_contact", label: "首次接触" }],
        intent_level: [{ id: "high", label: "高意向" }],
      },
    );
    render(<PlannerViewSection contact={baseContact} />);
    expect(screen.getByText("意向程度")).toBeInTheDocument();
    expect(screen.getByText("高意向")).toBeInTheDocument();
  });

  it("字典缺失的维度走 no_dict 灰显原始值(不显示错误销售标签)", () => {
    setStore(
      [{ kind: "emotion_state", displayName: "情绪状态", participatesInDecision: true }],
      {},
    );
    const c = { ...baseContact, domainAttributes: { emotion_state: "anxious" } } as unknown as Contact;
    render(<PlannerViewSection contact={c} />);
    expect(screen.getByText("anxious")).toBeInTheDocument();
  });

  it("维度无值时跳过不渲染", () => {
    setStore(
      [{ kind: "value_tier", displayName: "价值分层", participatesInDecision: true }],
      { value_tier: [{ id: "vip", label: "VIP" }] },
    );
    const c = { ...baseContact, domainAttributes: {} } as unknown as Contact;
    render(<PlannerViewSection contact={c} />);
    expect(screen.queryByText("价值分层")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 运行测试,确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/plannerView.test.tsx`
Expected: FAIL —— "意向程度" 维度未渲染(当前只有 customer_stage 行)。

- [ ] **Step 3: 实现多维区块**

在 PlannerViewSection 内,`taxonomies` 已经 `const taxonomies = useProfileStore((s) => s.taxonomies);`(:2030)。在其下加 dimensions 订阅:
```tsx
  const dimensions = useProfileStore((s) => s.dimensions);
```
在 `commitments` 区块(:2082-2093)之后、`</section>` 之前插入"画像维度"区块。遍历 dimensions、跳过 customer_stage(已有专属行)、跳过无值维度:
```tsx
      {(() => {
        const attrs = (contact.domainAttributes ?? {}) as Record<string, unknown>;
        const extraDims = dimensions
          .filter((d) => d.kind !== "customer_stage")
          .map((d) => {
            const raw = attrs[d.kind];
            const value = typeof raw === "string" ? raw : "";
            return { dim: d, value };
          })
          .filter((x) => x.value !== "");
        if (extraDims.length === 0) return null;
        return (
          <div data-testid="planner-dimensions" style={{ marginTop: 8 }}>
            {extraDims.map(({ dim, value }) => {
              const r = labelFor(taxonomies, dim.kind, value);
              const dim_label = dim.displayName || dim.kind;
              const isFallback = r.status === "unknown_value" || r.status === "no_dict";
              return (
                <div
                  key={dim.kind}
                  data-testid={`planner-dim-${dim.kind}`}
                  style={{ fontSize: 13, color: "#444", marginBottom: 6 }}
                >
                  {dim_label}{" "}
                  <strong
                    style={isFallback ? { color: "#999" } : undefined}
                    title={
                      r.status === "unknown_value"
                        ? "未知取值(不在当前字典内)"
                        : r.status === "no_dict"
                          ? "该维度暂无取值字典,显示原始值(待配置)"
                          : undefined
                    }
                  >
                    {r.text}
                  </strong>
                </div>
              );
            })}
          </div>
        );
      })()}
```
注意:`labelFor` 已在文件 :51 `import { useProfileStore, labelFor } from "../../stores/profileStore";`,无需再 import。同时把组件早返回守卫(:2048 `if (!hasStage && !hasCommitments && !hasMode) return null;`)放宽——若有 extraDims 也不应早返回。改为先算 extraDims、把 `extraDims.length > 0` 纳入"是否有内容"判断(将 extraDims 计算上提到守卫之前)。

- [ ] **Step 4: 运行测试,确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/plannerView.test.tsx`
Expected: PASS。

- [ ] **Step 5: 全量前端测 + commit**

Run: `cd frontend && npx vitest run`
Expected: 全绿(PlannerViewSection 既有 customer_stage 测试不回归)。

```bash
git add frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/features/user-ops/plannerView.test.tsx
git commit -m "feat(user-ops): PlannerViewSection 多维度画像看板(A4 通用化)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 组二:纯前端 + 类型添加(后端字段已存在,零后端)

### Task 4: D7 — profile 5 个高级字段编辑 UI

**Files:**
- Modify: `frontend/src/types/index.ts`(DomainProfile :607-641 + DomainProfileDraft :643-665)
- Modify: `frontend/src/stores/strategyStore.ts`(editDomainProfile :303-330)
- Modify: `frontend/src/features/system-strategy/index.tsx`(ProfileEditor,加折叠面板)
- Test: `frontend/src/__tests__/features/system-strategy/` 下既有/新建测试

**Interfaces:**
- Consumes: `DomainProfileDraft`、ProfileEditor 的 `update(patch: Partial<DomainProfileDraft>)` helper、`styles.advanced`/`styles.formGrid`/`styles.inlineCheckbox`。
- Produces: 5 个新前端类型字段(snake_case,对齐后端 serde):
  - `transaction_facts_enabled?: boolean`
  - `reviewer_orientation?: ReviewerOrientation | null`(新类型 `ReviewerOrientation = { review_focus?: string|null; balance_principle?: string|null; pressure_few_shot?: string|null }`)
  - `mode_gate_policy_override?: string | null`
  - `trajectory_dimensions?: TrajectoryDimension[]`(新类型 `TrajectoryDimension = { kind: string; display_name: string }`)
  - `debounce_window_ms_override?: number | null`

后端已确认这 5 字段都在 `DomainProfile`(models.rs:1792/1859/1866/1830/1834),`saveDomainProfile` PUT 整体透传 draft,**零后端改动**。

- [ ] **Step 1: 写失败测试(类型 + 编辑回填)**

新建/append `frontend/src/__tests__/features/system-strategy/profileAdvancedFields.test.tsx`。先测 strategyStore.editDomainProfile 回填 5 字段:

```tsx
import { describe, it, expect } from "vitest";
import { useStrategyStore } from "../../../stores/strategyStore";
import type { DomainProfile } from "../../../types";

describe("D7 profile 高级字段", () => {
  it("editDomainProfile 回填 transaction_facts_enabled / reviewer_orientation 等 5 字段", () => {
    const profile = {
      profile_id: "p1", display_name: "测试", description: "",
      profile_dimensions: [], prompt_fragment: "", conversation_modes: [],
      business_formulas: [], commitment_markers: { product_effect: [], tone_only: [] },
      coverage_dimensions: [],
      transaction_facts_enabled: true,
      reviewer_orientation: { review_focus: "情感共鸣", balance_principle: "陪伴优先" },
      mode_gate_policy_override: "自定模式说明",
      trajectory_dimensions: [{ kind: "emotion", display_name: "情绪轨迹" }],
      debounce_window_ms_override: 8000,
      version: 1, current_version: true, previous_version: null, is_active: false, seeded_by: null,
      id: "x", workspace_id: "w",
    } as unknown as DomainProfile;
    useStrategyStore.getState().editDomainProfile(profile);
    const d = useStrategyStore.getState().profileDraft;
    expect(d.transaction_facts_enabled).toBe(true);
    expect(d.reviewer_orientation?.review_focus).toBe("情感共鸣");
    expect(d.mode_gate_policy_override).toBe("自定模式说明");
    expect(d.trajectory_dimensions?.[0].kind).toBe("emotion");
    expect(d.debounce_window_ms_override).toBe(8000);
  });
});
```

- [ ] **Step 2: 运行测试,确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/profileAdvancedFields.test.tsx`
Expected: FAIL —— draft 不含这些字段(editDomainProfile 拷贝列表未含)+ TS 类型报错。

- [ ] **Step 3a: 加类型(types/index.ts)**

在 `OperationMode` 类型(:572-577)之后加两个新类型:
```ts
// D7:reviewer 评审取向覆盖。对齐后端 ReviewerOrientation(models.rs:1939)。
export type ReviewerOrientation = {
  review_focus?: string | null;
  balance_principle?: string | null;
  pressure_few_shot?: string | null;
};

// D7/H17:intent 轨迹维度声明。对齐后端 TrajectoryDimension(models.rs:4067)。
export type TrajectoryDimension = {
  kind: string;
  display_name: string;
};
```
在 DomainProfile(:631 `operation_mode?` 行后)加:
```ts
  transaction_facts_enabled?: boolean;
  reviewer_orientation?: ReviewerOrientation | null;
  mode_gate_policy_override?: string | null;
  trajectory_dimensions?: TrajectoryDimension[];
  debounce_window_ms_override?: number | null;
```
在 DomainProfileDraft(:664 `operation_mode?` 行后)加同样 5 行(去掉 `| null`,Draft 用 optional):
```ts
  transaction_facts_enabled?: boolean;
  reviewer_orientation?: ReviewerOrientation;
  mode_gate_policy_override?: string;
  trajectory_dimensions?: TrajectoryDimension[];
  debounce_window_ms_override?: number;
```

- [ ] **Step 3b: 加 editDomainProfile 拷贝(strategyStore.ts:327 后)**

在 `operation_mode: profile.operation_mode ?? undefined,`(:327)之后加:
```ts
        transaction_facts_enabled: profile.transaction_facts_enabled ?? undefined,
        reviewer_orientation: profile.reviewer_orientation ?? undefined,
        mode_gate_policy_override: profile.mode_gate_policy_override ?? undefined,
        trajectory_dimensions: profile.trajectory_dimensions ?? undefined,
        debounce_window_ms_override: profile.debounce_window_ms_override ?? undefined,
```

- [ ] **Step 3c: 加 ProfileEditor 折叠面板**

在 ProfileEditor 的运营范式 `</details>`(:1947)之后插入新折叠面板。`reviewer_orientation`/`mode_gate_policy_override`/`transaction_facts_enabled` 是 publish 危险字段,面板加说明:
```tsx
      <details className={styles.advanced}>
        <summary>高级:交易/评审/轨迹(发布危险字段)</summary>
        <p className={styles.panelHint}>
          交易事实注入 / 评审取向 / 模式说明属发布危险字段,改动经发布确认流(riskyFields)二次确认。
        </p>
        <div className={styles.formGrid}>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={draft.transaction_facts_enabled ?? false}
              onChange={(e) => update({ transaction_facts_enabled: e.target.checked })}
            />
            交易型域(注入产品目录 + 持有事实)transaction_facts_enabled
          </label>
        </div>
        <label className={styles.field}>
          <span>评审重点 review_focus</span>
          <input
            type="text"
            value={draft.reviewer_orientation?.review_focus ?? ""}
            onChange={(e) =>
              update({
                reviewer_orientation: {
                  ...(draft.reviewer_orientation ?? {}),
                  review_focus: e.target.value || undefined,
                },
              })
            }
          />
        </label>
        <label className={styles.field}>
          <span>平衡原则 balance_principle</span>
          <input
            type="text"
            value={draft.reviewer_orientation?.balance_principle ?? ""}
            onChange={(e) =>
              update({
                reviewer_orientation: {
                  ...(draft.reviewer_orientation ?? {}),
                  balance_principle: e.target.value || undefined,
                },
              })
            }
          />
        </label>
        <label className={styles.field}>
          <span>模式-闸说明覆盖 mode_gate_policy_override</span>
          <textarea
            value={draft.mode_gate_policy_override ?? ""}
            onChange={(e) => update({ mode_gate_policy_override: e.target.value || undefined })}
          />
        </label>
        <label className={styles.field}>
          <span>去抖窗口(毫秒)debounce_window_ms_override</span>
          <input
            type="number"
            value={draft.debounce_window_ms_override ?? ""}
            onChange={(e) =>
              update({
                debounce_window_ms_override: e.target.value ? Number(e.target.value) : undefined,
              })
            }
          />
        </label>
      </details>
```
注:`trajectory_dimensions` 是结构化数组,本批 YAGNI——只做"只读展示已有项 + 整体保留"(编辑回填即可,不做逐项增删 UI,避免过度设计)。`update` helper 与 `styles.field`/`styles.advanced` 沿用文件既有定义(若 `styles.field` 不存在则用现有等价类名,实现期对齐)。`pressure_few_shot` 不在本批 UI(同 YAGNI,类型已建,留后续)。

- [ ] **Step 4: 运行测试 + 类型检查,确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/profileAdvancedFields.test.tsx && npx tsc --noEmit`
Expected: PASS + 0 TS 错误。

- [ ] **Step 5: 全量前端测 + commit**

Run: `cd frontend && npx vitest run`
Expected: 全绿。

```bash
git add frontend/src/types/index.ts frontend/src/stores/strategyStore.ts frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/profileAdvancedFields.test.tsx
git commit -m "feat(strategy): profile 5 个高级字段编辑 UI(D7 通用化,后端字段已存在)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: D8 — per_relationship_operation_mode map 编辑

**Files:**
- Modify: `frontend/src/types/index.ts`(DomainProfile + DomainProfileDraft)
- Modify: `frontend/src/stores/strategyStore.ts`(editDomainProfile)
- Modify: `frontend/src/features/system-strategy/index.tsx`(ProfileEditor,加 map 编辑面板)
- Test: `frontend/src/__tests__/features/system-strategy/perRelationshipMode.test.tsx`(新建)

**Interfaces:**
- Consumes: `OperationMode` 类型(types/index.ts:572-577);`update` helper;relationship_type 候选键(可用静态 canonical 集 `["customer","peer","friend"]` 或从 active profile 字典取,见 Step 3c)。
- Produces: `per_relationship_operation_mode?: Record<string, OperationMode>` on DomainProfile + Draft。

后端 `per_relationship_operation_mode: Option<BTreeMap<String,OperationMode>>` 已存在(models.rs:1763),三级回落链后端已接,**零后端改动**。DEFAULT 域该字段永远 None(护栏)。

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/features/system-strategy/perRelationshipMode.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { useStrategyStore } from "../../../stores/strategyStore";
import type { DomainProfile } from "../../../types";

describe("D8 per_relationship_operation_mode", () => {
  it("editDomainProfile 回填 per_relationship_operation_mode map", () => {
    const profile = {
      profile_id: "p1", display_name: "数字分身", description: "",
      profile_dimensions: [], prompt_fragment: "", conversation_modes: [],
      business_formulas: [], commitment_markers: { product_effect: [], tone_only: [] },
      coverage_dimensions: [],
      per_relationship_operation_mode: {
        friend: {
          funnel: { enabled: false },
          silence: { enabled: true },
          commitment: { enabled: false },
          quiet_hours: {},
        },
      },
      version: 1, current_version: true, previous_version: null, is_active: false, seeded_by: null,
      id: "x", workspace_id: "w",
    } as unknown as DomainProfile;
    useStrategyStore.getState().editDomainProfile(profile);
    const d = useStrategyStore.getState().profileDraft;
    expect(d.per_relationship_operation_mode?.friend.funnel.enabled).toBe(false);
    expect(d.per_relationship_operation_mode?.friend.silence.enabled).toBe(true);
  });
});
```

- [ ] **Step 2: 运行测试,确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/perRelationshipMode.test.tsx`
Expected: FAIL —— draft 不含该字段 + TS 报错。

- [ ] **Step 3a: 加类型(types/index.ts)**

DomainProfile(Task 4 加的 5 字段之后)加:
```ts
  per_relationship_operation_mode?: Record<string, OperationMode> | null;
```
DomainProfileDraft 加:
```ts
  per_relationship_operation_mode?: Record<string, OperationMode>;
```

- [ ] **Step 3b: 加 editDomainProfile 拷贝(strategyStore.ts,Task 4 的 5 字段之后)**

```ts
        per_relationship_operation_mode: profile.per_relationship_operation_mode ?? undefined,
```

- [ ] **Step 3c: 加 ProfileEditor map 编辑面板**

在 Task 4 的高级面板 `</details>` 之后插入。按 relationship_type 键(本批用静态 canonical 集 `customer/peer/friend`,与 m024 字典 + seed profile 同源;非销售域可后续从 active profile 字典动态取)增删键 + 每键三 toggle(镜像 operation_mode 编辑器 :1894-1945):
```tsx
      <details className={styles.advanced}>
        <summary>按关系类型分配运营范式(数字分身 per_relationship)</summary>
        <p className={styles.panelHint}>
          为不同关系类型(customer/peer/friend)各配一套范式。未配的关系类型回落 profile 级 operation_mode。
        </p>
        {(["customer", "peer", "friend"] as const).map((rt) => {
          const map = draft.per_relationship_operation_mode ?? {};
          const mode = map[rt];
          const enabled = !!mode;
          const setMode = (next: typeof mode | undefined) => {
            const nextMap = { ...(draft.per_relationship_operation_mode ?? {}) };
            if (next === undefined) {
              delete nextMap[rt];
            } else {
              nextMap[rt] = next;
            }
            update({ per_relationship_operation_mode: nextMap });
          };
          return (
            <div key={rt} className={styles.formGrid} data-testid={`per-rel-${rt}`}>
              <label className={styles.inlineCheckbox}>
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(e) =>
                    setMode(
                      e.target.checked
                        ? { funnel: { enabled: true }, silence: { enabled: true }, commitment: { enabled: true }, quiet_hours: {} }
                        : undefined,
                    )
                  }
                />
                为 {rt} 单独配置范式
              </label>
              {enabled && mode && (
                <>
                  <label className={styles.inlineCheckbox}>
                    <input
                      type="checkbox"
                      checked={mode.funnel?.enabled ?? true}
                      onChange={(e) =>
                        setMode({ ...mode, funnel: { ...(mode.funnel ?? { enabled: true }), enabled: e.target.checked } })
                      }
                    />
                    漏斗推进 funnel
                  </label>
                  <label className={styles.inlineCheckbox}>
                    <input
                      type="checkbox"
                      checked={mode.silence?.enabled ?? true}
                      onChange={(e) =>
                        setMode({ ...mode, silence: { ...(mode.silence ?? { enabled: true }), enabled: e.target.checked } })
                      }
                    />
                    沉默唤醒 silence
                  </label>
                  <label className={styles.inlineCheckbox}>
                    <input
                      type="checkbox"
                      checked={mode.commitment?.enabled ?? true}
                      onChange={(e) =>
                        setMode({ ...mode, commitment: { ...(mode.commitment ?? { enabled: true }), enabled: e.target.checked } })
                      }
                    />
                    承诺到期 commitment
                  </label>
                </>
              )}
            </div>
          );
        })}
      </details>
```

- [ ] **Step 4: 运行测试 + 类型检查,确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/perRelationshipMode.test.tsx && npx tsc --noEmit`
Expected: PASS + 0 TS 错误。

- [ ] **Step 5: 全量前端测 + commit**

Run: `cd frontend && npx vitest run`
Expected: 全绿。

```bash
git add frontend/src/types/index.ts frontend/src/stores/strategyStore.ts frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/perRelationshipMode.test.tsx
git commit -m "feat(strategy): per_relationship_operation_mode map 编辑(D8 数字分身,后端字段已存在)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 组三:需后端配合

### Task 6: A5 — conversation_mode 字典(后端 seed + view)+ 前端 labelFor

**Files:**
- Create: `src/db/migrations/m028_seed_conversation_mode.rs`
- Modify: `src/db/migrations/mod.rs`(注册 m028)
- Modify: `src/routes/operation_view.rs:54-61`(kinds 集加 conversation_mode)
- Modify: `frontend/src/features/user-ops/legacy.tsx`(conversationModeLabel :2178-2191 改 labelFor + 调用点 :2056)
- Test: m028 单元测(seed entries)+ 前端 vitest

**Interfaces:**
- Consumes(后端): `TaxonomyEntry`/`TaxonomyValue`、`db.collection_system_taxonomies()`、m024 的 upsert 模式。
- Produces: `kind="conversation_mode"` 的 4 个 global taxonomy 值(casual_relationship/value_exchange/consultative/boundary_protection + 中文 label),经 active-view 下发 `taxonomies["conversation_mode"]`。前端 `conversationModeLabel` 退役,改 labelFor。

**为何后端必须改**:`conversation_modes: Vec<String>`(models.rs:1746)是裸 key,无 label 字典;active-view kinds 集(operation_view.rs:54-61)= profile_dimensions ∪ relationship_type,**不含 conversation_mode**。前端写死 4 销售 case(legacy.tsx:2178-2191),非销售域回落英文。

- [ ] **Step 1: 写后端 seed 失败测试**

新建 `src/db/migrations/m028_seed_conversation_mode.rs` 的 `#[cfg(test)] mod tests`(先只写测试,seed fn 暂空让它编译失败 → 实为先写 fn 签名)。实操:直接按 m024 模式写完整文件(seed fn + run_step + tests),因为这是确定性纯函数 seed。测试:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_mode_seed_covers_four_default_values() {
        let now = DateTime::now();
        let entries = conversation_mode_seed_entries(now);
        let ids: Vec<&str> = entries.iter().map(|e| e.value.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                CONVERSATION_MODE_CASUAL,
                CONVERSATION_MODE_VALUE_EXCHANGE,
                CONVERSATION_MODE_CONSULTATIVE,
                CONVERSATION_MODE_BOUNDARY_PROTECTION,
            ]
        );
        for e in &entries {
            assert_eq!(e.scope, "global");
            assert_eq!(e.kind, CONVERSATION_MODE_KIND);
            assert_eq!(e.value.status, "active");
            assert!(!e.value.display_name.is_empty());
        }
    }
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `cargo test --lib m028 2>&1 | tail -20`
Expected: 编译失败(模块未注册 / fn 未定义)。

- [ ] **Step 3: 实现 m028(完整文件,镜像 m024)**

```rust
//! m028：A5 通用化——seed `conversation_mode` taxonomy 字典四值。
//!
//! 幂等（`$setOnInsert` upsert，不覆盖运营编辑）：seed 独立 kind `conversation_mode`
//! 四销售域默认值（scope=`global`）+ 中文 label。供 active-view 下发取值字典,前端
//! `labelFor` 把 canonical 英文翻译成中文(替代写死 switch)。非销售域(情感陪伴等)
//! 可经 `POST /api/admin/taxonomies` 增 intimate_companion 等本域模式。
//!
//! 与 conversation_modes: Vec<String>(profile 声明的本域启用模式列表)解耦:profile
//! 声明"本域用哪几种模式",本字典声明"每个模式 canonical→中文 label"。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::db::Database;
use crate::error::AppResult;
use crate::models::{TaxonomyEntry, TaxonomyValue};

pub(super) const CONVERSATION_MODE_KIND: &str = "conversation_mode";
pub(super) const CONVERSATION_MODE_CASUAL: &str = "casual_relationship";
pub(super) const CONVERSATION_MODE_VALUE_EXCHANGE: &str = "value_exchange";
pub(super) const CONVERSATION_MODE_CONSULTATIVE: &str = "consultative";
pub(super) const CONVERSATION_MODE_BOUNDARY_PROTECTION: &str = "boundary_protection";

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    seed_conversation_mode_taxonomy(db, now).await?;
    Ok(())
}

pub(super) fn conversation_mode_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
    let values: &[(&str, &str, &str, &[&str])] = &[
        (
            CONVERSATION_MODE_CASUAL,
            "寒暄关系",
            "轻寒暄、建立熟悉度的关系维护对话。",
            &["寒暄", "闲聊"],
        ),
        (
            CONVERSATION_MODE_VALUE_EXCHANGE,
            "价值互换",
            "围绕需求与价值匹配的信息交换。",
            &["价值交换", "互惠"],
        ),
        (
            CONVERSATION_MODE_CONSULTATIVE,
            "顾问咨询",
            "顾问式答疑/方案沟通,提供专业建议。",
            &["顾问", "咨询", "顾问式"],
        ),
        (
            CONVERSATION_MODE_BOUNDARY_PROTECTION,
            "边界保护",
            "客户表达压力/拒绝时的边界保护与降压。",
            &["边界", "降压"],
        ),
    ];
    values
        .iter()
        .map(|(id, display, desc, aliases)| TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: CONVERSATION_MODE_KIND.to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
                priority_weight: None,
                is_terminal: false,
                is_reactivation_target: false,
            },
            updated_at: now,
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: Some("conversation_mode_migration".to_string()),
        })
        .collect()
}

async fn seed_conversation_mode_taxonomy(db: &Database, now: DateTime) -> AppResult<()> {
    let collection = db.collection_system_taxonomies();
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    for entry in conversation_mode_seed_entries(now) {
        let filter = doc! {
            "scope": &entry.scope,
            "kind": &entry.kind,
            "value.id": &entry.value.id,
        };
        let mut doc_to_set = mongodb::bson::to_document(&entry)?;
        doc_to_set.remove("_id");
        let result = collection
            .update_one(
                filter,
                doc! { "$setOnInsert": doc_to_set },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;
        if result.upserted_id.is_some() {
            inserted += 1;
        } else {
            skipped += 1;
        }
    }
    tracing::info!(
        migration_id = "m028_seed_conversation_mode",
        inserted,
        skipped,
        "seeded conversation_mode taxonomy (4 values)"
    );
    Ok(())
}
```
(把 Step 1 的 tests 模块附在文件末尾。)

- [ ] **Step 4: 注册 m028(mod.rs)**

在 `src/db/migrations/mod.rs` 的 m027 注册块(:183)之后,按既有结构加 m028 条目(id/name + `run: |db| Box::pin(m028_seed_conversation_mode::run_step(db))`),并在文件顶部 `mod m027_...;` 之后加 `mod m028_seed_conversation_mode;`。**id 用下一个序号**(实现期看 m027 条目的 id 字段格式对齐,如 `"2026_06_XX_..."`)。

- [ ] **Step 5: active-view kinds 集加 conversation_mode**

`src/routes/operation_view.rs:59-61`:
```rust
    if !kinds.iter().any(|k| k == "relationship_type") {
        kinds.push("relationship_type".to_string());
    }
```
其后加:
```rust
    if !kinds.iter().any(|k| k == "conversation_mode") {
        kinds.push("conversation_mode".to_string());
    }
```

- [ ] **Step 6: 后端测 + check**

Run: `cargo test --lib m028 2>&1 | tail -10`
Expected: PASS。
Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5`
Expected: 0 警告 0 错误。

- [ ] **Step 7: 前端 conversationModeLabel 改 labelFor**

`legacy.tsx:2178-2191` 的 `conversationModeLabel` switch 整体删除。调用点 :2056:
```tsx
          上轮对话模式 <strong>{conversationModeLabel(lastMode!)}</strong>
```
改为(在 PlannerViewSection 内,taxonomies 已订阅):
```tsx
          上轮对话模式 <strong>{labelFor(taxonomies, "conversation_mode", lastMode!).text}</strong>
```
若 conversationModeLabel 在文件其它处仍被调用,一并改 labelFor(grep `conversationModeLabel(` 确认全部调用点)。

- [ ] **Step 8: 前端测(非销售模式中文化)**

在 `plannerView.test.tsx`(Task 3 建)append:
```tsx
  it("conversation_mode 经字典翻译中文(A5)", () => {
    setStore(
      [],
      { conversation_mode: [{ id: "consultative", label: "顾问咨询" }] },
    );
    const c = { ...baseContact, lastConversationMode: "consultative" } as any;
    render(<PlannerViewSection contact={c} />);
    expect(screen.getByText("顾问咨询")).toBeInTheDocument();
  });
```

- [ ] **Step 9: 全量测 + commit**

Run: `cd frontend && npx vitest run` → 全绿;`cargo test --lib 2>&1 | tail -3` → ≥350/0。

```bash
git add src/db/migrations/m028_seed_conversation_mode.rs src/db/migrations/mod.rs src/routes/operation_view.rs frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/features/user-ops/plannerView.test.tsx
git commit -m "feat(taxonomy): conversation_mode 字典 seed + active-view 下发 + 前端 labelFor(A5 通用化)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: D6 — 字典 is_terminal / is_reactivation_target 配置(前后端)

**Files:**
- Modify: `src/routes/admin_taxonomies.rs`(CreateTaxonomyValue :58-70 + create handler :139-154 + PatchTaxonomyRequest :72-81 + patch set_doc :196-220)
- Modify: `frontend/src/features/system-strategy/index.tsx`(TaxonomyDraft :50-57、EditDraft、submitCreate :653-662、submitEdit :691-695、create/edit 表单 inputs、createDraft reset :671)
- Test: 后端单元/集成测 + 前端 vitest

**Interfaces:**
- Consumes: `TaxonomyValue.is_terminal`/`is_reactivation_target`(models.rs,bool)。
- Produces: create/patch 接收两 flag;set_doc 用 camelCase 键 `value.isTerminal`/`value.isReactivationTarget`。

**为何后端必须改**:create handler 硬编码 `is_terminal:false`/`is_reactivation_target:false`(admin_taxonomies.rs:152-153);patch set_doc 白名单(:197-220)不含两 flag。前端 TaxonomyDraft(:50-57)无字段。"改字典即通用"在 UI 走不通。

- [ ] **Step 1: 后端——CreateTaxonomyValue 加两可选 flag**

`admin_taxonomies.rs` CreateTaxonomyValue(:58-70)加(放 description 后):
```rust
    #[serde(default)]
    is_terminal: bool,
    #[serde(default)]
    is_reactivation_target: bool,
```
create handler 把 :152-153 的硬编码:
```rust
            is_terminal: false,
            is_reactivation_target: false,
```
改为:
```rust
            is_terminal: payload.value.is_terminal,
            is_reactivation_target: payload.value.is_reactivation_target,
```

- [ ] **Step 2: 后端——PatchTaxonomyRequest 加两可选 flag + set_doc 白名单**

PatchTaxonomyRequest(:72-81)加:
```rust
    is_terminal: Option<bool>,
    is_reactivation_target: Option<bool>,
```
patch set_doc(在 deprecated 分支 :215-220 之后、`if set_doc.is_empty()` :221 之前)加:
```rust
    if let Some(is_terminal) = payload.is_terminal {
        set_doc.insert("value.isTerminal", is_terminal);
    }
    if let Some(is_reactivation_target) = payload.is_reactivation_target {
        set_doc.insert("value.isReactivationTarget", is_reactivation_target);
    }
```
**键名 camelCase**(`value.isTerminal`/`value.isReactivationTarget`)对齐既有 `value.displayName`(:202)的 camelCase 写法——TaxonomyValue 序列化用 camelCase。

- [ ] **Step 3: 后端编译 + check**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5`
Expected: 0 警告 0 错误。

- [ ] **Step 4: 后端集成测(append,需 Docker → 标 #[ignore],留 CI)**

在 `tests/` 既有 taxonomy 集成测文件(grep `admin/taxonomies` 找;无则新建 `tests/taxonomy_flags_e2e.rs`)append `#[ignore]` 测试:POST 创建带 `value.isTerminal=true` → GET 该条目断言 isTerminal=true;PATCH `isReactivationTarget=true` → 断言落库。(本地无 Docker 不跑,CI integration job 跑。)

- [ ] **Step 5: 前端——TaxonomyDraft/EditDraft 加字段**

`system-strategy/index.tsx` TaxonomyDraft(:50-57)加:
```ts
  isReactivationTarget: boolean;
  isTerminal: boolean;
```
EditDraft(:56-57)加同样两字段。createDraft 初值(state 初始 + reset :671)加 `isReactivationTarget: false, isTerminal: false`。

- [ ] **Step 6: 前端——submit bodies 带 flag**

submitCreate 的 value 对象(:656-661)加:
```ts
          isTerminal: createDraft.isTerminal,
          isReactivationTarget: createDraft.isReactivationTarget,
```
submitEdit 的 patch body(:691-695)加:
```ts
        isTerminal: editDraft.isTerminal,
        isReactivationTarget: editDraft.isReactivationTarget,
```

- [ ] **Step 7: 前端——create/edit 表单加两复选**

在 create 表单(:769-810 区域)和 edit 表单(:863-885 区域)description 输入后各加:
```tsx
        <label className={styles.inlineCheckbox}>
          <input
            type="checkbox"
            checked={createDraft.isReactivationTarget}
            onChange={(e) => setCreateDraft({ ...createDraft, isReactivationTarget: e.target.checked })}
          />
          可作再激活目标 is_reactivation_target
        </label>
        <label className={styles.inlineCheckbox}>
          <input
            type="checkbox"
            checked={createDraft.isTerminal}
            onChange={(e) => setCreateDraft({ ...createDraft, isTerminal: e.target.checked })}
          />
          终态 is_terminal
        </label>
```
(edit 表单用 editDraft / setEditDraft 对应替换。类名 `styles.inlineCheckbox` 沿用文件既有;实现期对齐实际表单容器/类名。)

- [ ] **Step 8: 前端 vitest(表单提交含两 flag)**

新建/append `frontend/src/__tests__/features/system-strategy/taxonomyFlags.test.tsx`:mock api,渲染 create 表单,勾选两复选 + 填 scope/kind/id/label,提交,断言 `postRaw` 调用 body 含 `value.isTerminal===true`/`value.isReactivationTarget===true`。(按既有 system-strategy 测试 mock 结构对齐。)

- [ ] **Step 9: 全量测 + commit**

Run: `cd frontend && npx vitest run` → 全绿;`cargo test --lib 2>&1 | tail -3` → ≥350/0。

```bash
git add src/routes/admin_taxonomies.rs frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/taxonomyFlags.test.tsx tests/
git commit -m "feat(taxonomy): 字典 is_terminal/is_reactivation_target 配置入口(D6 通用化)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: C8 — decision_review_json 关联 AgentRunLog 补 emit 拦截四分支

**Files:**
- Modify: `src/routes/shared.rs`(decision_review_json :1053-1083)
- Modify: `src/routes/reviews.rs`(list_decision_reviews :60-64 + get_decision_review :85,传入关联数据)
- Modify: `frontend/src/types/index.ts`(DecisionReview :285-299 加两字段)
- Modify: `frontend/src/features/operations/index.tsx`(reviews tab 展示)/ 或 `legacy.tsx:563`
- Test: 后端集成测 + 前端 vitest

**Interfaces:**
- Consumes: `AgentRunLog.final_review_status`(typed,snake BSON);`AgentRunLog.review` doc 内 `holdCategory`(camelCase,因 DecisionReviewResult `rename_all="camelCase"` types.rs:1163);`db.agent_run_logs()`(db/mod.rs:179);`reviewLabels.ts`(Task 2 已建)。
- Produces: decision_review_json 输出加 `finalReviewStatus` / `holdCategory`(camelCase 键,与既有出口一致);前端 DecisionReview 加同名 optional 字段。

**关键数据源(已 grep 实证)**:`AgentDecisionReview`(decision_review_json 输入)无这两字段;`send_gateway_result` doc(`SendGatewayResult` types.rs:1496)只有 `status`/`policy_blocks`,**无** final_review_status/hold_category。两值在 **AgentRunLog**(同 run_id):`final_review_status` 顶层(snake)、`hold_category` 在 `review` doc 内(序列化为 `holdCategory`)。**否决**在 AgentDecisionReview 冗余写字段(漂移风险)。

- [ ] **Step 1: 改 decision_review_json 签名(接收可选关联值)**

`shared.rs:1053` 把:
```rust
pub(super) fn decision_review_json(review: AgentDecisionReview) -> Value {
```
改为:
```rust
pub(super) fn decision_review_json(
    review: AgentDecisionReview,
    final_review_status: Option<String>,
    hold_category: Option<String>,
) -> Value {
```
在 json! 块(:1080 `"status"` 行附近)加两键:
```rust
        "finalReviewStatus": final_review_status,
        "holdCategory": hold_category,
```

- [ ] **Step 2: reviews.rs 关联 AgentRunLog 后传值**

`reviews.rs` list_decision_reviews 循环(:60-64)改为:对每条 review,按 `run_id` 查 AgentRunLog 取两值。新增 helper(放 reviews.rs 文件内):
```rust
async fn fetch_run_status(
    state: &AppState,
    run_id: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(run_id) = run_id.filter(|s| !s.is_empty()) else {
        return (None, None);
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
            (frs, hc)
        }
        _ => (None, None),
    }
}
```
循环体(:61-63):
```rust
    while let Some(review) = cursor.try_next().await? {
        let (frs, hc) = fetch_run_status(&state, review.run_id.as_deref()).await;
        items.push(decision_review_json(review, frs, hc));
    }
```
get_decision_review(:85):
```rust
    let (frs, hc) = fetch_run_status(&state, review.run_id.as_deref()).await;
    Ok(Json(json!({ "item": decision_review_json(review, frs, hc) })))
```
确认 reviews.rs 顶部已 `use mongodb::bson::doc;`(若无则加)。

- [ ] **Step 3: 后端编译 + check**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5`
Expected: 0 警告 0 错误(注意:若有其它 decision_review_json 调用点,grep `decision_review_json(` 全部补参数——已知只有 reviews.rs:62/85)。

- [ ] **Step 4: 后端集成测(append,#[ignore] 留 CI)**

在 `tests/` 既有 decision review 集成测(grep `decision-reviews` / `decision_reviews`;无则新建 `tests/decision_review_status_e2e.rs`)append `#[ignore]` 测试:写一条 AgentDecisionReview(run_id=R)+ 一条 AgentRunLog(run_id=R, final_review_status="held_by_ai_policy", review doc 含 holdCategory="held_by_ai_policy")→ GET /api/decision-reviews?contactId= → 断言返回 item 含 `finalReviewStatus=="held_by_ai_policy"` + `holdCategory=="held_by_ai_policy"`。

- [ ] **Step 5: 前端 DecisionReview 类型加两字段**

`types/index.ts` DecisionReview(:285-299)在 `status: string;` 前加:
```ts
  finalReviewStatus?: string;
  holdCategory?: string;
```

- [ ] **Step 6: 前端 reviews tab 展示四分支**

在 operations/index.tsx 的 reviews tab(渲染 DecisionReview 的位置,grep `review.approved` / `通过` / `拦截`)把二元展示扩为:approved 时显"通过";未通过时用 reviewLabels 显具体分支。**注意 `labelOf` 对空值返回 `"—"`(truthy),不能用 `||` 串联兜底**——须显式判断字段是否存在:
```tsx
import { FINAL_REVIEW_STATUS_LABELS, HOLD_CATEGORY_LABELS, labelOf } from "../../lib/reviewLabels";

// helper(放组件外或文件内):未通过时按可用字段选标签,都缺则回落"拦截"。
function blockedLabel(review: DecisionReview): string {
  if (review.finalReviewStatus) return labelOf(FINAL_REVIEW_STATUS_LABELS, review.finalReviewStatus);
  if (review.holdCategory) return labelOf(HOLD_CATEGORY_LABELS, review.holdCategory);
  return "拦截";
}

// 渲染:
{review.approved ? "通过" : blockedLabel(review)}
```
(具体 JSX 位置实现期对齐 reviews tab 既有渲染;`blockedLabel` 保证 finalReviewStatus/holdCategory 都缺失时回落原"拦截"二元兜底,不回退成 `"—"`。)

- [ ] **Step 7: 前端 vitest**

append operations 测试:给一条 `approved:false, finalReviewStatus:"blocked_unverified_product_claim"` 的 review,断言渲染含"未验证产品声明拦截"(而非裸"拦截")。

- [ ] **Step 8: 全量测 + commit**

Run: `cd frontend && npx vitest run` → 全绿;`cargo test --lib 2>&1 | tail -3` → ≥350/0。

```bash
git add src/routes/shared.rs src/routes/reviews.rs frontend/src/types/index.ts frontend/src/features/operations/index.tsx frontend/src/__tests__/features/operations/operations.test.tsx tests/
git commit -m "feat(reviews): decision_review_json 关联 AgentRunLog 补 emit finalReviewStatus/holdCategory 拦截四分支(C8)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: E10 — 关系类型建议富投影(审核反盲批)

**Files:**
- Modify: `src/routes/ask_human_inbox.rs`(InboxItem struct :16-37 加 3 字段 + 9 个构造点 + collect_relationship_suggestions :191-212 填值)
- Modify: `frontend/src/lib/inboxApi.ts`(InboxItem :3-18 加 3 字段)
- Modify: `frontend/src/features/ask-human/inline/SimpleApproveReject.tsx`(富展示)
- Test: 后端集成测 + 前端 vitest

**Interfaces:**
- Consumes: `RelationshipTypeSuggestion`(models.rs:2823)的 `evidence: Option<String>`(:2832)、`confidence: i32`(:2834)、`occurrences: i32`(:2838)、`contact_id`(:2828,已有 contact_wxid 字段承载需另查 wxid——见 Step 注)。
- Produces: InboxItem 加 `evidence?: Option<String>` / `confidence?: Option<i32>` / `occurrences?: Option<i32>`(后端 camelCase 序列化 → `evidence`/`confidence`/`occurrences`);前端 InboxItem 同名 optional 字段。

**为何后端必须改**:`collect_relationship_suggestions`(:191-212)只投影 suggested_value(title/summary 都是它),rich_params=None,证据全丢 → 决策人盲批改写 relationship_type。证据字段 RelationshipTypeSuggestion **已全持久化**(纯投影,零模型改动)。

- [ ] **Step 1: 后端 InboxItem 加 3 可选字段**

`ask_human_inbox.rs` InboxItem struct(principal_wxid :后)加:
```rust
    // 关系类型建议富字段（仅 relationship_suggestion 来源填充）:决策人需看到
    // AI 判断依据/置信度/出现次数,避免盲批改写 relationship_type。其余来源恒 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrences: Option<i32>,
```

- [ ] **Step 2: 9 个构造点补字段**

InboxItem 无 `Default`,struct 字面量须列全字段。**8 个非 relationship 构造点**(:62/114/155/235/275/315/359 + escalation helper)在 `principal_wxid: ...,` 后加:
```rust
                evidence: None,
                confidence: None,
                occurrences: None,
```
(注意各点缩进对齐;escalation_to_inbox_item helper 同样补 None。)

- [ ] **Step 3: collect_relationship_suggestions 填证据值(:195-210)**

把该构造点(:195-209)的 `rich_params: None` 及富字段改为:
```rust
            InboxItem {
                source: "relationship_suggestion".into(),
                id,
                title: format!("关系类型建议:{}", r.suggested_value),
                summary: r.evidence.clone().unwrap_or_else(|| r.suggested_value.clone()),
                severity: "low".into(),
                created_at: Some(r.last_seen_at),
                age_hours: age_hours_of(Some(r.last_seen_at), now_ms),
                action_kind: "inline".into(),
                rich_component: None,
                rich_params: None,
                category: Some(r.suggested_value.clone()),
                question_for_principal: None,
                contact_wxid: non_empty(&r.contact_id),
                principal_wxid: None,
                evidence: r.evidence.clone(),
                confidence: Some(r.confidence),
                occurrences: Some(r.occurrences),
            }
```
注:`r.contact_id` 是 contact 的 id(非 wxid);本批先直接投 contact_id 入 contact_wxid 字段(前端展示"客户标识")。**若要真 wxid 需额外查 contact**——本批 YAGNI 不做,标注后续。`non_empty` helper 批次1已在文件内(B2 引入);若签名不符则用 `Some(r.contact_id.clone()).filter(|s| !s.is_empty())`。

- [ ] **Step 4: 后端编译 + check**

Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -5`
Expected: 0 警告 0 错误。

- [ ] **Step 5: 后端集成测(append,#[ignore] 留 CI)**

在 `tests/` 既有 ask_human inbox 集成测(grep `relationship_suggestion` / `collect_relationship`;无则新建)append `#[ignore]`:插一条 RelationshipTypeSuggestion(evidence="多次自称同行", confidence=80, occurrences=3, status="pending")→ GET 聚合收件箱 → 断言对应 item 含 `evidence` / `confidence==80` / `occurrences==3`。

- [ ] **Step 6: 前端 InboxItem 加 3 字段**

`frontend/src/lib/inboxApi.ts` InboxItem(:3-18,principalWxid 后)加:
```ts
  evidence?: string;
  confidence?: number;
  occurrences?: number;
```

- [ ] **Step 7: SimpleApproveReject 富展示**

`SimpleApproveReject.tsx` 在 summary div(:23)后、buttons 前加富字段展示(仅在有值时显示):
```tsx
      {(item.evidence || item.confidence !== undefined || item.occurrences !== undefined) && (
        <div className="simpleActionEvidence" style={{ fontSize: 12, color: "#666", marginTop: 4 }}>
          {item.evidence && <div>判断依据:{item.evidence}</div>}
          {item.confidence !== undefined && <div>置信度:{item.confidence}</div>}
          {item.occurrences !== undefined && <div>出现次数:{item.occurrences}</div>}
          {item.contactWxid && <div>客户标识:{item.contactWxid}</div>}
        </div>
      )}
```

- [ ] **Step 8: 前端 vitest(富展示)**

新建 `frontend/src/__tests__/features/ask-human/inline/SimpleApproveReject.test.tsx`:渲染带 `evidence/confidence/occurrences` 的 item,断言"判断依据""置信度""出现次数"文本出现;再渲染无这些字段的 item,断言富区块不出现(不破其余 InboxItem 来源,如 knowledgeReview)。

- [ ] **Step 9: 全量测 + commit**

Run: `cd frontend && npx vitest run` → 全绿;`cargo test --lib 2>&1 | tail -3` → ≥350/0;`RUSTFLAGS=-Dwarnings cargo check --tests` → 0/0。

```bash
git add src/routes/ask_human_inbox.rs frontend/src/lib/inboxApi.ts frontend/src/features/ask-human/inline/SimpleApproveReject.tsx frontend/src/__tests__/features/ask-human/inline/SimpleApproveReject.test.tsx tests/
git commit -m "feat(ask-human): 关系类型建议富投影证据/置信度/出现次数,审核反盲批(E10 数字分身)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 任务依赖与执行顺序

- **组一(Task 1/2/3)**:纯前端,互相独立。**Task 2 必须先于 Task 8**(C8 复用 Task 2 建的 reviewLabels.ts)。
- **组二(Task 4/5)**:Task 5 依赖 Task 4(都改 types/index.ts DomainProfile + strategyStore editDomainProfile + ProfileEditor,Task 5 在 Task 4 加的字段之后追加,避免行号冲突)。建议 4→5 顺序。
- **组三(Task 6/7/8/9)**:各自前后端成对,互相独立;Task 8 依赖 Task 2。
- **文件重叠提示**:Task 4/5 都改 `types/index.ts` + `strategyStore.ts` + `system-strategy/index.tsx`,Task 7 也改 `system-strategy/index.tsx`,Task 1/8 都改 `operations/index.tsx`,Task 3/6 都改 `legacy.tsx`。subagent-driven 串行执行,每 task 基于上一 task 的 commit,无并行冲突。
- **推荐执行序**:1 → 2 → 3 → 6 → 4 → 5 → 7 → 8 → 9(组一打底 + 共享模块先行,A5 与 A4 同改 legacy.tsx 相邻,C8 在 reviewLabels 之后)。

## 验收清单(交付时人肉浏览器验收)

标 spec ✅需浏览器 的条目:B 组无(批次1),本批 A5/C8/D6/E10 + A4 需起 dev server 验收:
- A4:选客户 → Planner 视角显示多维度中文标签(含非销售域走灰显)。
- A5:对话模式显示中文(非英文 key)。
- C8:reviews tab 拦截记录显示四分支中文(非二元"拦截")。
- D6:字典管理 create/edit 表单可勾选两 flag,保存后 GET 回读 isTerminal/isReactivationTarget。
- E10:请示收件箱关系建议卡显示判断依据/置信度/出现次数。
- D7/D8:profile 编辑器折叠面板可编辑 5 字段 + per_relationship map。
