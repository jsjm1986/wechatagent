# 前后端业务对齐 批次4（P3 收尾：通用化维度补全 + 可观测细节 + SSE 韧性 + D9）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 收尾前后端业务对齐 76→67 路线图剩余 9 条（D10/D11/F11/F14/F13/F15/F16/F17/D9），把"配维度/改字典/写字段表即通用"在 UI 走通，补可观测细节与 SSE 韧性。

**Architecture:** 几乎纯前端。除 D9 复用已存在的后端 `domain_schemas` CRUD（无后端逻辑改动）外，无任何新端点、新 migration、新集成测。组一补通用化维度编辑入口（后端字段早就绪）；组二打磨可观测/加载态/SSE 重连；组三 D9 把"有意缺口"转做（补写表单 + 改文案承诺）。

**Tech Stack:** React 19 + TypeScript + Vite + Zustand + CSS Modules（`.module.css` + tokens.css 变量）；vitest + @testing-library/react；后端 Rust/Axum（仅 D9 复用现有 `src/routes/domain_schemas.rs` CRUD，不改）。

## Global Constraints

> 每个 task 的要求都隐含包含本节，逐条 verbatim 来自 spec 第六节。

- 子 agent 一律 `model:"opus"`；回复中文。
- **无人工接管 CI lint**：`src/agent|routes|evolution` + `frontend/src` 新增行（含注释/JSX 文案）禁词 `人工/接管/takeover/hand-off/人工接管/转人工/人工介入/人工托管`（测试目录除外）。F13 gatewayStatus 中文标签、F11 来源徽标文案、D9 文案改动一律用业务语义措辞避开禁词。
- 测试基线不回退：`cargo test --lib ≥350/0`、4 PBT ≥33/0、`RUSTFLAGS=-Dwarnings cargo check --tests` 0/0。**本批纯前端无后端逻辑改动**，cargo 侧只需不回退（不新增集成测）。前端 `npx vitest run` 全绿 + `npx tsc --noEmit` 0 + lint 0。本地只跑 `cargo test --lib` + 前端测（无本地 Docker，集成测交 CI）。
- 测试只增量叠加，不删改旧维度/旧弧/旧金标。既有测试文件（`systemStrategy.test.tsx` / `tagTrustPanel.test.tsx` / `commandCenter.test.tsx` / `operations.test.tsx` / `knowledge.test.tsx`）只 append 用例，不动既有用例。
- **AI 永不自动验证知识红线**：本批 D9 不碰此红线（`domain_schemas` 是字段表定义非知识 chunk）；若任何改动触及知识 chunk 写入须守 `status=draft` + `needs_review`。
- 前端遵守现有设计系统：tokens.css 变量、`.module.css`、4 级层级、蓝=主操作专属、紫=AI 身份专属（见 `docs/frontend-design-system.md`）。**F11 在 TagTrustPanel 紫色 AI 身份区，徽标用紫系/中性不误用蓝；F16 复用退避工具避免重复**。
- git：仅在用户要求时提交；只 `git add` 具名文件，绝不 `git add -A`（工作区有并行会话未提交的 `scripts/biz-test/*` + `scripts/_remote_run.py`，绝不 stash/checkout/覆盖）；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`；破坏性 gitops 须显式授权。

## 不变量（全程守住）

- D9 create 落 `is_active=false`，靠 activate 切换，保持"同 workspace 至多一条 active"不变量；delete 不删 active（后端已拦，前端给提示）。
- D9 前端 wire 键 camelCase 对齐 `UpsertRequest` rename_all。
- D11 先补 types `initial_signal` 字段，否则后端发出被前端丢弃。
- **F16 关键技术分野**：`explore.tsx` 的 `/api/knowledge/ask/stream` 是**一次性 RPC 流**（提问 → trace/token → answer → close），给它加自动重连=断连后重发查询、重复扣 LLM token（有害），**不接入重连**，只修 F17 stale closure。只有 `today.tsx` 两处**长连接会话监听流**（`/chat/sessions/:id/stream`，幂等只触发 reload）接入退避重连。
- F16 重连正确清理旧 EventSource + 组件卸载/主动取消停止重连（不泄漏、不竞争）。
- F14 移除死控件后，切租户走 `/api/auth/workspace`（批次3 E15 已有切换器）。
- 所有新增前端文案不含禁词（CI 门）；新组件遵守设计系统，优先复用现有组件与模式。

---

## 执行顺序

组一（低风险打底，任意序）→ 组二（体验）→ 组三（D9 最重，独立 task）。共 **9 个 task**：
- 组一：Task 1 (D10) / Task 2 (D11) / Task 3 (F11) / Task 4 (F14)
- 组二：Task 5 (F13) / Task 6 (F15) / Task 7 (F16) / Task 8 (F17)
- 组三：Task 9 (D9)

> F16 与 F17 同改 explore.tsx 的 SSE 区，但语义正交（F16 只接入 today 长连接流、F17 只修 explore 一次性流闭包）。拆为相邻两 task（Task 7→8），互不冲突。

---

## Task 1: D10 — ProfileDimension.participates_in_decision 复选框

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx:1388-1446`（维度配置编辑器）
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`（append，不动既有用例）

**Interfaces:**
- Consumes: `ProfileDimension`（`frontend/src/types/index.ts:526-531`：`kind:string` / `display_name:string` / `participates_in_decision:boolean` / `description:string`）；编辑器 `draft.profile_dimensions: ProfileDimension[]`，`update(patch)` 合并 draft。
- Produces: 维度行可读写 `participates_in_decision`；"+添加维度"保持默认 `true`。

- [ ] **Step 1: 写失败的组件测**

在 `systemStrategy.test.tsx` append（沿用文件既有 render/mock 套路，先读文件头确认 mock 形态）：

```tsx
it("D10: 维度行可切换 participates_in_decision 为只观测维度", async () => {
  // 渲染编辑器（profile 含一条 participates_in_decision=true 的维度），
  // 找到该行的 participates_in_decision 复选框，断言初始 checked，
  // 点击后断言 update/draft 收到该维度 participates_in_decision=false。
  // 具体 render/查询沿用本文件既有 systemStrategy 编辑器测的套路。
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx`
Expected: FAIL（复选框不存在 / 找不到 participates_in_decision 控件）

- [ ] **Step 3: 维度行加复选框**

在 `index.tsx` 维度行（`:1410-1419` 的 description input 之后、`:1420` 删除按钮之前）插入复选框，照同区 coverage 维度的 `.inlineCheckbox` 样板（:1673-1684）：

```tsx
              <label className={styles.inlineCheckbox}>
                <input
                  type="checkbox"
                  checked={dim.participates_in_decision}
                  onChange={(e) => {
                    const dims = [...(draft.profile_dimensions ?? [])];
                    dims[i] = { ...dim, participates_in_decision: e.target.checked };
                    update({ profile_dimensions: dims });
                  }}
                />
                进决策
              </label>
```

`:1440` "+添加维度"新建项保持 `participates_in_decision: true` 不变（现有语义）。

- [ ] **Step 4: 跑测确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx`
Expected: PASS（含既有用例不回退）

- [ ] **Step 5: tsc + lint + commit**

Run: `cd frontend && npx tsc --noEmit && npx eslint src/features/system-strategy/index.tsx`
Expected: 0 错误

```bash
git add frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "feat(strategy): D10 维度配置加 participates_in_decision 复选框(支持建只观测维度)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: D11 — CoverageDimension anchor_hint + initial_signal 编辑（+ types 补字段）

**Files:**
- Modify: `frontend/src/types/index.ts:547-552`（`CoverageDimension` 补 `initial_signal`）
- Modify: `frontend/src/features/system-strategy/index.tsx:1651-1711`（completeness 审计维度编辑器）
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`（append）

**Interfaces:**
- Consumes: `CoverageDimension`（`types/index.ts:547`：`key` / `display_name` / `required` / `anchor_hint?` → 本 task 补 `initial_signal?`）；后端 `models.rs:2230` `CoverageDimension` 含 `anchor_hint`(:2240) + `initial_signal`(:2251)，serde camelCase。
- Produces: 编辑器可读写 `anchor_hint` + `initial_signal`；type round-trip 不丢 `initial_signal`。

- [ ] **Step 1: 写失败的组件测**

append 到 `systemStrategy.test.tsx`：

```tsx
it("D11: completeness 维度可编辑 anchor_hint 与 initial_signal 并提交", async () => {
  // 渲染编辑器（profile 含一条 coverage_dimensions），
  // 在该行 anchor_hint 输入框输入文本、initial_signal 输入框输入文本，
  // 断言 draft.coverage_dimensions[0] 收到 anchor_hint / initial_signal 两字段。
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx`
Expected: FAIL（anchor_hint / initial_signal 输入框不存在）

- [ ] **Step 3: 补 type 字段**

`types/index.ts:547-552`：

```ts
export type CoverageDimension = {
  key: string;
  display_name: string;
  required: boolean;
  anchor_hint?: string | null;
  initial_signal?: string | null;
};
```

- [ ] **Step 4: 编辑器加两个文本输入**

在 `index.tsx` coverage 维度行（`:1684` 的 `.inlineCheckbox` 必备之后、`:1685` 删除按钮之前）插入两个 input，照同行 `styles.input` 样板：

```tsx
              <input
                className={styles.input}
                value={cov.anchor_hint ?? ""}
                placeholder="anchor_hint（锚点提示，可选）"
                onChange={(e) => {
                  const arr = [...(draft.coverage_dimensions ?? [])];
                  arr[i] = { ...cov, anchor_hint: e.target.value };
                  update({ coverage_dimensions: arr });
                }}
              />
              <input
                className={styles.input}
                value={cov.initial_signal ?? ""}
                placeholder="initial_signal（初始信号，可选）"
                onChange={(e) => {
                  const arr = [...(draft.coverage_dimensions ?? [])];
                  arr[i] = { ...cov, initial_signal: e.target.value };
                  update({ coverage_dimensions: arr });
                }}
              />
```

"+添加 completeness 维度"(`:1701-1706`)新建项保持 `{ key:"", display_name:"", required:false }`（两新字段可选，留空即可）。

- [ ] **Step 5: 跑测确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx`
Expected: PASS（含既有用例不回退）

- [ ] **Step 6: tsc + lint + commit**

Run: `cd frontend && npx tsc --noEmit && npx eslint src/features/system-strategy/index.tsx src/types/index.ts`
Expected: 0 错误

```bash
git add frontend/src/types/index.ts frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "feat(strategy): D11 completeness 维度补 anchor_hint+initial_signal 编辑(types 补 initial_signal 防丢字段)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: F11 — TagTrustPanel AI 确信层 confirmedBy 来源徽标 + tooltip

**Files:**
- Modify: `frontend/src/features/user-ops/TagTrustPanel.tsx:93-105`（aiChip 渲染）
- Modify: `frontend/src/features/user-ops/TagTrustPanel.module.css`（加徽标样式，紫系/中性，不用蓝）
- Test: `frontend/src/__tests__/features/user-ops/tagTrustPanel.test.tsx`（append）

**Interfaces:**
- Consumes: `ConfirmedTag`（`types/index.ts:51`：`value` / `evidences:Evidence[]` / `confirmedAt:string` / `confirmedBy:string`）；后端语义闭集 `strong_evidence`（强证据快通道）| `consolidation`（压缩重判）（`models.rs:256-257`）。
- Produces: aiChip 在 evidenceCount 旁多一个来源徽标，带 title tooltip。

- [ ] **Step 1: 写失败的组件测**

append 到 `tagTrustPanel.test.tsx`（沿用文件既有 render + Contact 构造套路）：

```tsx
it("F11: AI 确信标签显示 strong_evidence 来源徽标", () => {
  // contact.confirmedTags = [{ value:"VIP", evidences:[{turn:1,msgId:"m1"}], confirmedAt:"...", confirmedBy:"strong_evidence" }]
  // 渲染后断言出现「强证据」文案
});
it("F11: consolidation 来源显示压缩重判徽标", () => {
  // confirmedBy:"consolidation" → 断言出现「压缩重判」
});
it("F11: 未知/空 confirmedBy 不崩且不显徽标", () => {
  // confirmedBy:"" → 断言 value 仍渲染、无徽标文案、不抛错
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/tagTrustPanel.test.tsx`
Expected: FAIL（「强证据」/「压缩重判」文案不存在）

- [ ] **Step 3: 加来源映射 + 渲染徽标**

`TagTrustPanel.tsx` 组件函数外（文件顶部 import 之后）加纯映射：

```tsx
// AI 确信来源（后端 ConfirmedTag.confirmedBy 闭集）→ 中文标签 + tooltip 说明。
// strong_evidence = 直接证据快通道确信；consolidation = 记忆压缩时整体重新判定确信。
// 未知/缺省 → 返回 null（不显徽标，不崩）。
const CONFIRMED_BY_META: Record<string, { label: string; hint: string }> = {
  strong_evidence: { label: "强证据", hint: "直接证据快通道确信，可信度较高" },
  consolidation: { label: "压缩重判", hint: "记忆压缩时整体重新判定确信" },
};
```

`:96-99` aiChip 渲染内，evidenceCount 之后插入徽标（紫系/中性，复用 AI 身份区色，不用蓝）：

```tsx
            confirmedTags.map((tag) => {
              const meta = CONFIRMED_BY_META[tag.confirmedBy];
              return (
                <span key={tag.value} className={styles.aiChip}>
                  {tag.value}
                  <span className={styles.evidenceCount}>{tag.evidences.length} 条证据</span>
                  {meta ? (
                    <span className={styles.confirmedBySource} title={meta.hint}>
                      {meta.label}
                    </span>
                  ) : null}
                </span>
              );
            })
```

- [ ] **Step 4: 加徽标 CSS（紫系/中性，走 tokens 变量）**

`TagTrustPanel.module.css` append `.confirmedBySource`，配色用 AI 身份区紫系或中性 ink 变量（参照同文件 `.aiChip` / `.evidenceCount` 的现有变量），**不用蓝**。读现有规则确认变量名后再写。

- [ ] **Step 5: 跑测确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/tagTrustPanel.test.tsx`
Expected: PASS（既有用例不回退）

- [ ] **Step 6: tsc + lint + 禁词 + commit**

Run: `cd frontend && npx tsc --noEmit && npx eslint src/features/user-ops/TagTrustPanel.tsx`
Run: `bash scripts/check-no-human-takeover.sh <BASE> HEAD`（BASE=本 task 起点 commit）
Expected: 0 错误 0 禁词

```bash
git add frontend/src/features/user-ops/TagTrustPanel.tsx frontend/src/features/user-ops/TagTrustPanel.module.css frontend/src/__tests__/features/user-ops/tagTrustPanel.test.tsx
git commit -m "feat(user-ops): F11 AI 确信标签显示 confirmedBy 来源徽标+tooltip(强证据/压缩重判)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: F14 — 移除 explore 误导性租户输入框（死控件）

**Files:**
- Modify: `frontend/src/features/knowledge/explore.tsx`（删 `:46-61` workspaceId state+localStorage effect、`:97` body 携带、`:116-117` params.set、`:217-226` 输入框 UI）
- Test: `frontend/src/__tests__/features/knowledge/knowledge.test.tsx`（append）

**Interfaces:**
- Consumes: 无（纯删除）。后端 `sources_meta.rs:534/:634` 无条件 `let workspace_id = admin.current_workspace.clone()`，废弃字段 `#[allow(dead_code)]` 保留不动。
- Produces: explore 不再有租户输入框；请求不再带 `workspaceId`。

- [ ] **Step 1: 写失败的组件测**

append 到 `knowledge.test.tsx`（沿用文件既有 AskView 渲染套路；若文件未渲染 AskView，新建 `frontend/src/__tests__/features/knowledge/exploreNoTenant.test.tsx`）：

```tsx
it("F14: explore 不再渲染租户输入框", () => {
  // 渲染 AskView，断言 placeholder "default" 的租户输入框不存在（queryByPlaceholderText("default") 为 null）
  // 且「租户（可选）」label 文案不存在
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/knowledge.test.tsx`
Expected: FAIL（租户输入框仍存在）

- [ ] **Step 3: 删 state + localStorage effect**

删 `explore.tsx:46-61`（`workspaceId` useState + 持久化 useEffect 整块）。

- [ ] **Step 4: 删请求携带**

- `:97` 改为 `body: JSON.stringify({ query: q })`（去掉 `workspaceId ? {...} :` 三元）。
- `:116-117` 删 `if (workspaceId) params.set("workspaceId", workspaceId);`（保留 `const params = new URLSearchParams({ query: q });`）。

- [ ] **Step 5: 删输入框 UI**

删 `:217-226` 的 `<label className="wikiAskWsField">…</label>` 整块。

- [ ] **Step 6: 跑测确认通过 + tsc + lint**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/knowledge.test.tsx && npx tsc --noEmit && npx eslint src/features/knowledge/explore.tsx`
Expected: PASS + 0 错误（确认无 workspaceId 残留引用导致 TS 报未使用）

- [ ] **Step 7: commit**

```bash
git add frontend/src/features/knowledge/explore.tsx frontend/src/__tests__/features/knowledge/knowledge.test.tsx
git commit -m "fix(knowledge): F14 移除 explore 误导性租户输入框(后端恒忽略,切租户走 /api/auth/workspace)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: F13 — command-center gatewayStatus 32 值中文 label map

**Files:**
- Modify: `frontend/src/features/command-center/index.tsx:30-45`（callStatusLabel 旁加 gatewayStatusLabel）+ `:75`（用它替换裸 String）
- Test: `frontend/src/__tests__/features/command-center/commandCenter.test.tsx`（append）

**Interfaces:**
- Consumes: 后端闭集 `GATEWAY_STATUS_VALUES`（`src/agent/run_envelope.rs:86-135`，共 32 值）。
- Produces: `gatewayStatusLabel(status: string): string`，未知值回落原值。

后端 32 值闭集（逐一对照 run_envelope.rs:87-134，verbatim）：
`pending` `approved` `allowed` `sent` `no_reply` `review_blocked` `revision_failed` `revision_skipped_invalid_direction` `revision_skipped_budget_exceeded` `revision_llm_failure` `held_by_ai_policy` `blocked_by_safety_guard` `ai_waiting_for_more_context` `blocked_by_required_field` `blocked_by_budget` `blocked_unverified_product_claim` `tool_loop_timeout` `legacy_mode_unchecked` `not_managed` `cooldown` `rate_limited` `daily_limit` `expired` `context_changed` `policy_cooldown` `policy_wait_user_reply` `gateway_blocked` `precheck_blocked` `outbox_enqueued` `admin_cancelled` `superseded_by_new_inbound` `quiet_hours_deferred`

- [ ] **Step 1: 写失败的组件/单元测**

append 到 `commandCenter.test.tsx`：

```tsx
it("F13: gatewayStatus 显示中文标签", () => {
  // 渲染 command-center（命令结果含 response.gatewayStatus="held_by_ai_policy" 与 sentContent），
  // 断言出现「AI 策略主动暂缓」中文标签（不是裸 held_by_ai_policy）
});
it("F13: 未知 gatewayStatus 回落原值不崩", () => {
  // gatewayStatus="some_future_status" → 断言原样显示 some_future_status
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/command-center/commandCenter.test.tsx`
Expected: FAIL（中文标签不存在）

- [ ] **Step 3: 加 gatewayStatusLabel（32 值，业务语义，避禁词）**

在 `index.tsx` `callStatusLabel`(:45) 之后插入。中文措辞守无人工接管 lint（held_by_ai_policy 用「AI 策略主动暂缓」、ai_waiting_for_more_context 用「AI 等待更多上下文」、blocked_by_safety_guard 用「安全门拦截」，**禁词全避**）：

```tsx
// gateway 终态闭集（src/agent/run_envelope.rs GATEWAY_STATUS_VALUES，32 值）→ 中文业务语义标签。
// 措辞守 AI 自治定位（无禁词）：held_by_ai_policy=AI 策略主动暂缓 等。default 回落原值，未来新值不崩。
function gatewayStatusLabel(status: string): string {
  switch (status) {
    case "pending": return "待处理";
    case "approved": return "已批准";
    case "allowed": return "已放行";
    case "sent": return "已发送";
    case "no_reply": return "无需回复";
    case "review_blocked": return "Review 拦截";
    case "revision_failed": return "改写失败";
    case "revision_skipped_invalid_direction": return "改写跳过（方向无效）";
    case "revision_skipped_budget_exceeded": return "改写跳过（预算超限）";
    case "revision_llm_failure": return "改写时模型失败";
    case "held_by_ai_policy": return "AI 策略主动暂缓";
    case "blocked_by_safety_guard": return "安全门拦截";
    case "ai_waiting_for_more_context": return "AI 等待更多上下文";
    case "blocked_by_required_field": return "缺必填字段拦截";
    case "blocked_by_budget": return "预算超限拦截";
    case "blocked_unverified_product_claim": return "未核实产品主张拦截";
    case "tool_loop_timeout": return "工具循环超时";
    case "legacy_mode_unchecked": return "旧模式未校验";
    case "not_managed": return "未托管";
    case "cooldown": return "冷却中";
    case "rate_limited": return "限流中";
    case "daily_limit": return "已达每日上限";
    case "expired": return "已过期";
    case "context_changed": return "上下文已变更";
    case "policy_cooldown": return "策略冷却";
    case "policy_wait_user_reply": return "等待客户回复";
    case "gateway_blocked": return "网关拦截";
    case "precheck_blocked": return "预检拦截";
    case "outbox_enqueued": return "已入发件队列";
    case "admin_cancelled": return "管理员已取消";
    case "superseded_by_new_inbound": return "被更新消息取代";
    case "quiet_hours_deferred": return "作息时段顺延";
    default: return status;
  }
}
```

- [ ] **Step 4: :75 用 label 替换裸 String**

`:75` 的 `gatewayStatus ? \`网关：${String(gatewayStatus)}\` : ""` 改为：

```tsx
      gatewayStatus ? `网关：${gatewayStatusLabel(String(gatewayStatus))}` : "",
```

- [ ] **Step 5: 跑测 + tsc + lint + 禁词**

Run: `cd frontend && npx vitest run src/__tests__/features/command-center/commandCenter.test.tsx && npx tsc --noEmit && npx eslint src/features/command-center/index.tsx`
Run: `bash scripts/check-no-human-takeover.sh <BASE> HEAD`
Expected: PASS + 0 错误 + 0 禁词

- [ ] **Step 6: commit**

```bash
git add frontend/src/features/command-center/index.tsx frontend/src/__tests__/features/command-center/commandCenter.test.tsx
git commit -m "feat(command-center): F13 gatewayStatus 32 值中文 label map(避禁词,未知回落原值)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6: F15 — operationsStore 加载态

**Files:**
- Modify: `frontend/src/stores/operationsStore.ts`（state 加 `loading`，loadOperationsData 起置 true / finally 置 false）
- Modify: `frontend/src/features/operations/index.tsx:203-365`（各 tab loading 时显加载态而非 EmptyState）
- Test: `frontend/src/__tests__/features/operations/operations.test.tsx`（append）

**Interfaces:**
- Consumes: `useOperationsStore`（现有 `OperationsState`：events/tasks/decisionReviews/llmUsage/agentRuns/opsTab + loadOperationsData）。
- Produces: `OperationsState.loading: boolean`。

**边界**：与批次3 错误态（全局错误横幅 `useUiStore.setError`）正交——loading（拉取中）/ error（失败横幅）/ empty（成功无数据）三态清晰。

- [ ] **Step 1: 写失败的 store 测**

append 到 `operations.test.tsx`（沿用既有 store 测套路；若无 store 测则组件测）：

```tsx
it("F15: loadOperationsData 生命周期 loading 先 true 后 false", async () => {
  // mock api.get 返回可控 promise；调 loadOperationsData()，
  // await 微任务前断言 loading===true，await 完成后断言 loading===false
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/operations/operations.test.tsx`
Expected: FAIL（loading 字段不存在）

- [ ] **Step 3: store 加 loading**

`operationsStore.ts`：interface 加 `loading: boolean;`（:11 区，agentRuns 之后）；初值 `loading: false,`（:23 区）；`loadOperationsData` 开头 `set({ loading: true });`，并把现有 try/catch 包进 try…finally：

```ts
  loadOperationsData: async (accountId?: string) => {
    const accountParam = accountId ? `accountId=${encodeURIComponent(accountId)}` : "";
    set({ loading: true });
    try {
      const [eventsRes, tasksRes, reviewsRes, llmUsageRes, agentRunsRes] = await Promise.all([
        // ……（现有 5 个 api.get 不变）
      ]);
      set({ /* 现有 5 字段 set 不变 */ });
    } catch (error) {
      // ……（现有 catch 体不变：console.error + setError + 置空 5 字段）
    } finally {
      set({ loading: false });
    }
  },
```

> 注意：现有 catch 块里的置空 `set({...})` 与 finally 的 `set({loading:false})` 不冲突（zustand set 浅合并）。

- [ ] **Step 4: 各 tab loading 时显加载态**

`operations/index.tsx`：从 store 解构 `loading`（:150-154 区）。每个 tab 的 `length === 0 ? <EmptyState/>` 改为先判 loading。以 tasks tab（:203-205）为例：

```tsx
        {opsTab === "tasks" &&
          (loading ? (
            <EmptyState title="加载中…" hint="正在拉取运营数据。" />
          ) : tasks.length === 0 ? (
            <EmptyState title="暂无跟进任务" hint="Agent 排程的跟进会在这里按计划呈现。" />
          ) : (
            // ……现有 table 不变
          ))}
```

events(:250)/reviews(:278)/runs(:308)/llm(:342) 五处同样在 `length===0` 前插 `loading ? 加载态 :`。llm tab 是 `&& (` 结构，同样在内层 EmptyState(:363) 前插 loading 判断。

- [ ] **Step 5: 组件测加载态**

append 到 `operations.test.tsx`：

```tsx
it("F15: loading 时显加载态而非空态", () => {
  // 把 store 置 loading=true、tasks=[]，渲染 operations，
  // 断言出现「加载中…」，且不出现「暂无跟进任务」
});
```

- [ ] **Step 6: 跑测 + tsc + lint + commit**

Run: `cd frontend && npx vitest run src/__tests__/features/operations/operations.test.tsx && npx tsc --noEmit && npx eslint src/stores/operationsStore.ts src/features/operations/index.tsx`
Expected: PASS + 0 错误

```bash
git add frontend/src/stores/operationsStore.ts frontend/src/features/operations/index.tsx frontend/src/__tests__/features/operations/operations.test.tsx
git commit -m "feat(operations): F15 加载态(loading 三态区分 拉取中/失败/空,各 tab 显加载态)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 7: F16 — SSE 指数退避自动重连（共享 hook，接入 today 长连接流）

**Files:**
- Create: `frontend/src/lib/useSseReconnect.ts`（共享退避重连器）
- Modify: `frontend/src/features/knowledge/today.tsx:125-144`（ReviewChat 会话历史流接入）+ `:805-829`（TaskRail attachStream 接入）
- Test: `frontend/src/__tests__/lib/useSseReconnect.test.ts`（新建，fake timers 测退避）

**Interfaces:**
- Consumes: 浏览器 `EventSource`；两处长连接流 URL `/api/knowledge/chat/sessions/:id/stream`（`turn` 事件触发幂等 reload）。
- Produces: `createSseReconnector(url, { onEvent, onReconnecting?, onGaveUp?, maxRetries?, baseDelayMs?, capMs? }): { close(): void }`。

**关键技术分野（不变量）**：只接入 today.tsx 两处**长连接会话监听流**（断连重连只触发 reload，幂等安全）。**explore.tsx 的 `/api/knowledge/ask/stream` 是一次性 RPC 流，断连重连=重发查询、重复扣 LLM token，绝不接入**（F17 单独处理 explore）。

- [ ] **Step 1: 写失败的退避测（fake timers）**

新建 `frontend/src/__tests__/lib/useSseReconnect.test.ts`：

```ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createSseReconnector } from "../../lib/useSseReconnect";

class FakeES {
  static instances: FakeES[] = [];
  url: string;
  listeners: Record<string, ((ev: unknown) => void)[]> = {};
  closed = false;
  constructor(url: string) { this.url = url; FakeES.instances.push(this); }
  addEventListener(t: string, cb: (ev: unknown) => void) { (this.listeners[t] ||= []).push(cb); }
  close() { this.closed = true; }
  emit(t: string, ev?: unknown) { (this.listeners[t] || []).forEach((cb) => cb(ev)); }
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeES.instances = [];
  vi.stubGlobal("EventSource", FakeES as unknown as typeof EventSource);
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe("createSseReconnector", () => {
  it("error 触发指数退避重连（base×2^attempt）", () => {
    const r = createSseReconnector("/s", { onEvent: {}, baseDelayMs: 1000, capMs: 30000, maxRetries: 5 });
    expect(FakeES.instances).toHaveLength(1);
    FakeES.instances[0].emit("error");          // 第 1 次断连
    expect(FakeES.instances[0].closed).toBe(true);
    vi.advanceTimersByTime(999); expect(FakeES.instances).toHaveLength(1); // 还没到 1000ms
    vi.advanceTimersByTime(1);   expect(FakeES.instances).toHaveLength(2); // 1000ms 后重连
    FakeES.instances[1].emit("error");          // 第 2 次断连
    vi.advanceTimersByTime(2000); expect(FakeES.instances).toHaveLength(3); // 2000ms 后
    r.close();
  });

  it("达 maxRetries 停止重连", () => {
    const r = createSseReconnector("/s", { onEvent: {}, baseDelayMs: 100, capMs: 30000, maxRetries: 2 });
    for (let i = 0; i < 5; i++) {
      const es = FakeES.instances[FakeES.instances.length - 1];
      es.emit("error");
      vi.advanceTimersByTime(60000);
    }
    expect(FakeES.instances.length).toBeLessThanOrEqual(3); // 初次 + 最多 2 次重连
    r.close();
  });

  it("成功事件重置 attempt", () => {
    const r = createSseReconnector("/s", { onEvent: { turn: () => {} }, baseDelayMs: 1000, capMs: 30000, maxRetries: 5 });
    FakeES.instances[0].emit("error");
    vi.advanceTimersByTime(1000);                // 重连 → instances[1]
    FakeES.instances[1].emit("turn");            // 成功事件 → 重置 attempt
    FakeES.instances[1].emit("error");
    vi.advanceTimersByTime(1000);                // attempt 重置后仍是 base×2^0=1000ms
    expect(FakeES.instances).toHaveLength(3);
    r.close();
  });

  it("close() 后不再重连", () => {
    const r = createSseReconnector("/s", { onEvent: {}, baseDelayMs: 100, capMs: 30000, maxRetries: 5 });
    r.close();
    FakeES.instances[0].emit("error");
    vi.advanceTimersByTime(60000);
    expect(FakeES.instances).toHaveLength(1);
  });
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/lib/useSseReconnect.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现退避重连器**

新建 `frontend/src/lib/useSseReconnect.ts`：

```ts
// SSE 指数退避自动重连器。仅用于「长连接监听流」（断连重连幂等、只触发 reload）。
// 严禁用于一次性 RPC 流（如 /knowledge/ask/stream）——重连会重发查询、重复扣 token。
//
// 退避：delay = min(capMs, baseDelayMs × 2^attempt)。达 maxRetries 停止。任一注册事件触发即重置 attempt。
// 调用方负责在组件卸载 / 主动取消时调 close()，停止重连且清理 EventSource。
export interface SseReconnectOptions {
  // 事件名 → 回调。任一事件触发都视为「连接健康」，重置退避 attempt。
  onEvent: Record<string, (ev: MessageEvent) => void>;
  onReconnecting?: (attempt: number, delayMs: number) => void;
  onGaveUp?: () => void;
  baseDelayMs?: number; // 默认 1000
  capMs?: number;       // 默认 30000
  maxRetries?: number;  // 默认 6
}

export interface SseHandle {
  close: () => void;
}

export function createSseReconnector(url: string, opts: SseReconnectOptions): SseHandle {
  const base = opts.baseDelayMs ?? 1000;
  const cap = opts.capMs ?? 30000;
  const maxRetries = opts.maxRetries ?? 6;
  let attempt = 0;
  let stopped = false;
  let es: EventSource | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const clearTimer = () => { if (timer !== null) { clearTimeout(timer); timer = null; } };
  const cleanupEs = () => { if (es) { es.close(); es = null; } };

  function connect() {
    if (stopped) return;
    cleanupEs();
    const next = new EventSource(url);
    es = next;
    for (const [name, cb] of Object.entries(opts.onEvent)) {
      next.addEventListener(name, (ev) => {
        attempt = 0; // 收到业务事件 → 连接健康，重置退避
        cb(ev as MessageEvent);
      });
    }
    next.addEventListener("error", () => {
      if (stopped) return;
      cleanupEs();
      if (attempt >= maxRetries) { opts.onGaveUp?.(); return; }
      const delay = Math.min(cap, base * 2 ** attempt);
      attempt += 1;
      opts.onReconnecting?.(attempt, delay);
      clearTimer();
      timer = setTimeout(connect, delay);
    });
  }

  connect();
  return {
    close() { stopped = true; clearTimer(); cleanupEs(); },
  };
}
```

- [ ] **Step 4: 跑测确认通过**

Run: `cd frontend && npx vitest run src/__tests__/lib/useSseReconnect.test.ts`
Expected: PASS

- [ ] **Step 5: today.tsx ReviewChat 历史流接入**

先读 `today.tsx:79` 区确认 `esRef` 在 ReviewChat 组件内的所有使用点。`:125-144` 的 SSE useEffect 改用 reconnector（保留 `turn`→reload、卸载 close 语义）：

```tsx
  useEffect(() => {
    if (!sessionId || typeof window === "undefined" || typeof window.EventSource === "undefined") return;
    const handle = createSseReconnector(
      `/api/knowledge/chat/sessions/${encodeURIComponent(sessionId)}/stream`,
      { onEvent: { turn: () => { void loadHistory(sessionId); } } },
    );
    return () => handle.close();
  }, [sessionId, loadHistory]);
```

> 若 `esRef`(:79) 仅此 effect 使用则一并删除其声明；若 submit 等别处仍用（手写 EventSource 提交流）则保留 ref。读确认后再改，勿盲删。

- [ ] **Step 6: today.tsx TaskRail attachStream 接入**

`today.tsx:803-829` 的 `esRef` / `closeStream` / `attachStream` 改用 reconnector，ref 改存 `SseHandle`：

```tsx
  const sseRef = useRef<SseHandle | null>(null);

  function closeStream() {
    sseRef.current?.close();
    sseRef.current = null;
  }

  function attachStream(sid: string) {
    closeStream();
    if (!sid || typeof window === "undefined" || typeof window.EventSource === "undefined") return;
    sseRef.current = createSseReconnector(
      `/api/knowledge/chat/sessions/${encodeURIComponent(sid)}/stream`,
      { onEvent: { turn: (ev) => { const v = Number(ev.data); if (!Number.isNaN(v)) setLiveTurns((prev) => [...prev, v]); } } },
    );
  }

  useEffect(() => () => closeStream(), []);
```

import 顶部加 `import { createSseReconnector, type SseHandle } from "../../lib/useSseReconnect";`。

- [ ] **Step 7: 全前端测 + tsc + lint**

Run: `cd frontend && npx vitest run && npx tsc --noEmit && npx eslint src/lib/useSseReconnect.ts src/features/knowledge/today.tsx`
Expected: PASS + 0 错误（确认 today.tsx 无残留旧 esRef 引用）

- [ ] **Step 8: commit**

```bash
git add frontend/src/lib/useSseReconnect.ts frontend/src/features/knowledge/today.tsx frontend/src/__tests__/lib/useSseReconnect.test.ts
git commit -m "feat(knowledge): F16 SSE 指数退避自动重连(共享器,仅接入 today 长连接流;一次性流不接)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 8: F17 — explore stale closure 误抑制错误横幅（ref 修）

**Files:**
- Modify: `frontend/src/features/knowledge/explore.tsx`（加 `resultRef`，error handler 读 ref 最新值）
- Test: `frontend/src/__tests__/features/knowledge/knowledge.test.tsx`（append；或 Task 4 新建的 explore 测文件）

**Interfaces:**
- Consumes: 现有 `result` state（`AskResult | null`）。
- Produces: `resultRef` 跟踪 result 最新值；submitStream 的 error handler 用 `resultRef.current` 判断而非闭包捕获的 `result`。

**前置**：F17 在 Task 7 之后做。Task 7 不接入 explore 的一次性流（保持其手写 EventSource），F17 只修该一次性流 error handler 的 stale closure。两 task 都在 explore SSE 区但语义正交。

- [ ] **Step 1: 写失败的组件测**

append（连续两次提交：第一次成功拿到 result、第二次失败，断言第二次错误横幅出现不被旧 result 抑制）：

```tsx
it("F17: 上一轮有结果后再次提交失败,错误横幅不被旧 result 抑制", async () => {
  // mock EventSource：第一次 submit → emit answer（result 非空）；
  // 第二次 submit → emit error（无 answer）；
  // 断言第二次 error 后出现「流式连接错误」横幅（resultRef 已被 resetForSubmit 清空）
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/knowledge.test.tsx`
Expected: FAIL（旧闭包 result 非空 → `!result` 为 false → 横幅被抑制）

- [ ] **Step 3: 加 resultRef 并同步**

`explore.tsx`：`esRef`(:52) 旁加 `const resultRef = useRef<AskResult | null>(null);`。在 `resetForSubmit`(:69-77) 的 `setResult(null)` 后加同步 `resultRef.current = null;`（同步置空，error handler 同步触发时能读到最新）；answer 分支(:145) `setResult({...})` 后加 `resultRef.current = { ...data, tookMs: ... };`（或抽变量）；一次性 fetch 分支(:101) `setResult(data)` 后加 `resultRef.current = data;`。

> 不用 `useEffect(() => { resultRef.current = result; }, [result])` 单独同步——effect 异步，error handler 同步触发时可能未跑。须在每个 setResult 处同步置 ref。

- [ ] **Step 4: error handler 读 ref**

`:150-158` error handler 把 `if (!result)` 改为 `if (!resultRef.current)`：

```tsx
    es.addEventListener("error", () => {
      if (!resultRef.current) {
        setError("流式连接错误（请关闭实时模式或重试）");
      }
      es.close();
      esRef.current = null;
      setPending(false);
    });
```

- [ ] **Step 5: 跑测 + tsc + lint + commit**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/knowledge.test.tsx && npx tsc --noEmit && npx eslint src/features/knowledge/explore.tsx`
Expected: PASS + 0 错误

```bash
git add frontend/src/features/knowledge/explore.tsx frontend/src/__tests__/features/knowledge/knowledge.test.tsx
git commit -m "fix(knowledge): F17 explore error handler 用 resultRef 修 stale closure(失败横幅不被旧 result 抑制)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 9: D9 — domain-schemas 写表单（create/edit/delete）+ 改文案承诺

**Files:**
- Create: `frontend/src/features/knowledge/DomainSchemaEditor.tsx`（schema 编辑器：name + fields 动态数组 + aliasDict + guardDsl）
- Modify: `frontend/src/features/knowledge/atlas.tsx`（DomainSchemaTab 加 create/edit/delete 入口 + 改 :544/:552 文案）
- Test: `frontend/src/__tests__/features/knowledge/domainSchemaEditor.test.tsx`（新建）

**Interfaces:**
- Consumes 后端（**不改后端**）：
  - `GET /api/admin/domain-schemas`（list，现有 `:497` 已用）
  - `POST /api/admin/domain-schemas`（create，`domain_schemas.rs:197`，落 `is_active=false`）
  - `PUT /api/admin/domain-schemas/:id`（update，`:238`）
  - `DELETE /api/admin/domain-schemas/:id`（delete，`:294`，不允许删 active → 后端 400）
  - `POST /api/admin/domain-schemas/:id/activate`（activate，`:331`，现有 `:512` 已用）
- **serde 命门（`UpsertRequest`，domain_schemas.rs:93-105，`rename_all="camelCase"`）**：提交 body wire 键须 camelCase：
  - `schemaId`（必填非空）/ `name`（必填非空）
  - `fields`（数组，每项 `DomainFieldPayload` camelCase：`name` / `label` / `kind`(string|enum|number|date|reference) / `required`(bool) / `allowedValues`(enum 时必填非空数组) / `aliasOf`)
  - `aliasDict`（JSON object `{中文别名: canonical字段名}`）/ `guardDsl`（可选 string）/ `workspaceId`（可选，缺省用 session current_workspace）
- 现有前端类型：`DomainSchemaItem`(atlas.tsx:472-483) + `DomainSchemaField`(:464-471)，已 camelCase（schemaId/allowedValues/aliasOf/aliasDict/guardDsl/isActive）。

> **为何不用 FormDialog**：`FormDialog`(components/ui/FormDialog) 只支持扁平 key-value 字段，撑不住 D9 的嵌套 `fields` 动态数组（增删行、每行 6 子字段、enum 时 allowedValues 子数组）。故新建 `DomainSchemaEditor` 内联编辑器，参照 `system-strategy/index.tsx` 的 `profile_dimensions` 数组编辑模式（增删行 + 每行多输入 + “+添加”按钮）。复用 `wikiInput`/`wikiBtn` 等 knowledge 频道既有 class。

**复用后端校验**：`validate_schema_payload`(domain_schemas.rs:427) 已做 fields≤64 / 字段名黑名单 / 名唯一 / kind 合法 / enum allowed_values 非空 / alias 指向合法。前端做基本必填校验（schemaId/name 非空）即可，越界交后端 400 → 现有 `parseApiError` + error 横幅提示。

**不变量**：create 落 `is_active=false`（不自动激活，靠现有 activate 切换，保持“同 workspace 至多一条 active”）；delete active 后端拦 → 前端给提示。**不碰 AI 知识验证红线**（domain_schemas 是字段表定义非知识 chunk）。

- [ ] **Step 1: 写失败的编辑器测（camelCase 提交命门）**

新建 `frontend/src/__tests__/features/knowledge/domainSchemaEditor.test.tsx`：

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DomainSchemaEditor } from "../../../features/knowledge/DomainSchemaEditor";

describe("DomainSchemaEditor", () => {
  it("create 提交 body 为 camelCase 键（schemaId/fields[allowedValues]/aliasDict）", () => {
    const onSubmit = vi.fn();
    render(<DomainSchemaEditor mode="create" onSubmit={onSubmit} onCancel={vi.fn()} />);
    fireEvent.change(screen.getByPlaceholderText(/schemaId/i), { target: { value: "real_estate" } });
    fireEvent.change(screen.getByPlaceholderText(/字段表名称/), { target: { value: "房产销售" } });
    // 加一个 enum 字段
    fireEvent.click(screen.getByText(/添加字段/));
    fireEvent.change(screen.getByPlaceholderText(/字段名.*name/i), { target: { value: "stage" } });
    fireEvent.change(screen.getByPlaceholderText(/中文标签.*label/i), { target: { value: "阶段" } });
    // kind=enum 时填 allowedValues
    fireEvent.click(screen.getByText(/保存/));
    const body = onSubmit.mock.calls[0][0];
    expect(body).toHaveProperty("schemaId", "real_estate");
    expect(body).toHaveProperty("name", "房产销售");
    expect(Array.isArray(body.fields)).toBe(true);
    expect(body.fields[0]).toHaveProperty("name", "stage");
    // 关键：wire 键必须 camelCase，不是 allowed_values / alias_of
    expect(body.fields[0]).not.toHaveProperty("allowed_values");
    expect(body).toHaveProperty("aliasDict");
  });

  it("schemaId/name 为空时不提交（必填校验）", () => {
    const onSubmit = vi.fn();
    render(<DomainSchemaEditor mode="create" onSubmit={onSubmit} onCancel={vi.fn()} />);
    fireEvent.click(screen.getByText(/保存/));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("edit 模式回填已有 schema", () => {
    const existing = { schemaId: "x", name: "旧名", fields: [{ name: "f1", label: "字段1", kind: "string", required: false }], aliasDict: {}, guardDsl: null };
    render(<DomainSchemaEditor mode="edit" initial={existing as never} onSubmit={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByDisplayValue("旧名")).toBeInTheDocument();
    expect(screen.getByDisplayValue("字段1")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/domainSchemaEditor.test.tsx`
Expected: FAIL（DomainSchemaEditor 不存在）

- [ ] **Step 3: 实现 DomainSchemaEditor**

新建 `frontend/src/features/knowledge/DomainSchemaEditor.tsx`。结构：name/schemaId 顶部两输入（edit 模式 schemaId 只读）；fields 动态数组（每行 name/label/kind select/required 复选/kind=enum 时 allowedValues 逗号输入/aliasOf 输入 + 删除）；“+添加字段”；aliasDict 用一个 textarea 收“别名=>字段名”逐行（或 JSON）；guardDsl textarea；保存/取消。提交时组装 camelCase body：

```tsx
import { useState } from "react";

export interface DomainSchemaFieldDraft {
  name: string;
  label: string;
  kind: string;            // string|enum|number|date|reference
  required: boolean;
  allowedValues?: string[];
  aliasOf?: string;
}
export interface DomainSchemaUpsertBody {
  schemaId: string;
  name: string;
  fields: DomainSchemaFieldDraft[];
  aliasDict: Record<string, string>;
  guardDsl?: string;
}
interface InitialSchema {
  schemaId: string;
  name: string;
  fields: DomainSchemaFieldDraft[];
  aliasDict?: Record<string, string>;
  guardDsl?: string | null;
}

const KIND_OPTIONS = ["string", "enum", "number", "date", "reference"];

export function DomainSchemaEditor({
  mode,
  initial,
  onSubmit,
  onCancel,
}: {
  mode: "create" | "edit";
  initial?: InitialSchema;
  onSubmit: (body: DomainSchemaUpsertBody) => void;
  onCancel: () => void;
}) {
  const [schemaId, setSchemaId] = useState(initial?.schemaId ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [fields, setFields] = useState<DomainSchemaFieldDraft[]>(initial?.fields ?? []);
  const [aliasText, setAliasText] = useState(
    Object.entries(initial?.aliasDict ?? {}).map(([k, v]) => `${k}=${v}`).join("\n"),
  );
  const [guardDsl, setGuardDsl] = useState(initial?.guardDsl ?? "");

  function updateField(i: number, patch: Partial<DomainSchemaFieldDraft>) {
    setFields((arr) => arr.map((f, idx) => (idx === i ? { ...f, ...patch } : f)));
  }

  function submit() {
    if (!schemaId.trim() || !name.trim()) return; // 基本必填，越界交后端 400
    const aliasDict: Record<string, string> = {};
    for (const line of aliasText.split("\n")) {
      const [k, v] = line.split("=");
      if (k?.trim() && v?.trim()) aliasDict[k.trim()] = v.trim();
    }
    const body: DomainSchemaUpsertBody = {
      schemaId: schemaId.trim(),
      name: name.trim(),
      fields: fields.map((f) => ({
        name: f.name.trim(),
        label: f.label.trim(),
        kind: f.kind,
        required: f.required,
        ...(f.kind === "enum" && f.allowedValues?.length ? { allowedValues: f.allowedValues } : {}),
        ...(f.aliasOf?.trim() ? { aliasOf: f.aliasOf.trim() } : {}),
      })),
      aliasDict,
      ...(guardDsl.trim() ? { guardDsl: guardDsl.trim() } : {}),
    };
    onSubmit(body);
  }

  return (
    <div className="wikiSchemaEditor">
      <label className="wikiField">
        <span>schemaId（英文 id，唯一）</span>
        <input className="wikiInput" placeholder="schemaId 如 real_estate" value={schemaId}
          onChange={(e) => setSchemaId(e.target.value)} disabled={mode === "edit"} />
      </label>
      <label className="wikiField">
        <span>字段表名称</span>
        <input className="wikiInput" placeholder="字段表名称（中文，如 房产销售）" value={name}
          onChange={(e) => setName(e.target.value)} />
      </label>

      <div className="wikiSchemaEditorFields">
        {fields.map((f, i) => (
          <div className="wikiSchemaEditorRow" key={i}>
            <input className="wikiInput" placeholder="字段名 name（英文）" value={f.name}
              onChange={(e) => updateField(i, { name: e.target.value })} />
            <input className="wikiInput" placeholder="中文标签 label" value={f.label}
              onChange={(e) => updateField(i, { label: e.target.value })} />
            <select className="wikiInput" value={f.kind} onChange={(e) => updateField(i, { kind: e.target.value })}>
              {KIND_OPTIONS.map((k) => <option key={k} value={k}>{k}</option>)}
            </select>
            <label className="wikiInlineCheckbox">
              <input type="checkbox" checked={f.required} onChange={(e) => updateField(i, { required: e.target.checked })} />
              必填
            </label>
            {f.kind === "enum" ? (
              <input className="wikiInput" placeholder="可选值（逗号分隔）"
                value={(f.allowedValues ?? []).join(", ")}
                onChange={(e) => updateField(i, { allowedValues: e.target.value.split(/[,，]/).map((s) => s.trim()).filter(Boolean) })} />
            ) : null}
            <input className="wikiInput" placeholder="aliasOf（可选，指向另一字段名）" value={f.aliasOf ?? ""}
              onChange={(e) => updateField(i, { aliasOf: e.target.value })} />
            <button type="button" className="ghost" onClick={() => setFields((arr) => arr.filter((_, idx) => idx !== i))}>删除</button>
          </div>
        ))}
        <button type="button" className="ghost"
          onClick={() => setFields((arr) => [...arr, { name: "", label: "", kind: "string", required: false }])}>
          + 添加字段
        </button>
      </div>

      <label className="wikiField">
        <span>同义词识别（每行一条：别名=字段名）</span>
        <textarea className="wikiInput" rows={3} placeholder="例如：预算=budget" value={aliasText}
          onChange={(e) => setAliasText(e.target.value)} />
      </label>
      <label className="wikiField">
        <span>guardDsl（可选）</span>
        <textarea className="wikiInput" rows={2} value={guardDsl} onChange={(e) => setGuardDsl(e.target.value)} />
      </label>

      <div className="wikiSchemaEditorActions">
        <button type="button" className="ghost" onClick={onCancel}>取消</button>
        <button type="button" className="primary" onClick={submit} disabled={!schemaId.trim() || !name.trim()}>保存</button>
      </div>
    </div>
  );
}
```

> 若复用的 class（`wikiInput`/`wikiField`/`wikiInlineCheckbox`/`wikiSchemaEditor*`）在 knowledge 频道全局 css 中不存在，则补到 knowledge 频道现有 plain `.css`（**非 module.css**，遵守该频道既有约定 + 避免 tree-shake 坑），走 tokens 变量，不用蓝（蓝=主操作专属，仅“保存/设为当前使用”按钮的 `.primary` 用）。读 atlas 现有 `.wikiSchema*` 所在 css 确认文件后再补。

- [ ] **Step 4: 跑测确认编辑器通过**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/domainSchemaEditor.test.tsx`
Expected: PASS

- [ ] **Step 5: atlas.tsx DomainSchemaTab 接入 CRUD + 改文案**

`atlas.tsx` DomainSchemaTab(:485)：
- 加 state `const [editing, setEditing] = useState<{ mode: "create" | "edit"; initial?: DomainSchemaItem } | null>(null);`
- toolbar(:538-546) 加“+ 新建字段表”按钮（`setEditing({ mode: "create" })`）。
- editing 非空时渲染 `<DomainSchemaEditor>`，onSubmit 调 create/update：

```tsx
  async function saveSchema(body: DomainSchemaUpsertBody) {
    const isEdit = editing?.mode === "edit";
    const url = isEdit
      ? `/api/admin/domain-schemas/${encodeURIComponent(body.schemaId)}`
      : "/api/admin/domain-schemas";
    setError(null);
    try {
      const r = await fetch(url, {
        method: isEdit ? "PUT" : "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!r.ok) throw await parseApiError(r);
      toast.success(isEdit ? "已更新字段表" : "已新建字段表（未激活，可在列表设为当前使用）");
      setEditing(null);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteSchema(s: DomainSchemaItem) {
    if (s.isActive) { toast.error("使用中的字段表不能删除，请先切换到其它字段表"); return; }
    const ok = await confirm({ title: `删除字段表「${s.name}」？`, body: "删除后不可恢复。", tone: "danger", confirmText: "确认删除" });
    if (!ok) return;
    setError(null);
    try {
      const r = await fetch(`/api/admin/domain-schemas/${encodeURIComponent(s.schemaId)}`, { method: "DELETE" });
      if (!r.ok) throw await parseApiError(r);
      toast.success("已删除");
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }
```

- 每张卡片(:573-588 actions 区)加“编辑”按钮（`setEditing({ mode: "edit", initial: s })`）+“删除”按钮（`deleteSchema(s)`，active 时禁用或给提示）。
- **改文案承诺（自相矛盾否则）**：
  - `:544` 改为：`这里是 AI 判断客户用的「字段表」——它规定了 AI 在对话里会记录客户的哪些信息（如所处阶段、意向程度）。你可以在这里创建、编辑、删除字段表，并切换当前使用的一套。`（去掉“由系统管理员维护…不能直接改内容”）
  - `:552` 空态 hint 改为：`点击「+ 新建字段表」创建第一套，创建后可在这里编辑或设为当前使用。`（去掉“由系统管理员在后台创建后”）

import 顶部加 `import { DomainSchemaEditor, type DomainSchemaUpsertBody } from "./DomainSchemaEditor";`。

- [ ] **Step 6: atlas 接入测（POST/PUT/DELETE 路径 + 文案）**

append 到 `frontend/src/__tests__/features/knowledge/knowledge.test.tsx`（或新建 `domainSchemaTab.test.tsx`）：

```tsx
it("D9: 新建字段表 POST 到 /api/admin/domain-schemas", async () => {
  // mock fetch：GET list 返回空，点“+ 新建字段表”→ 填 schemaId/name → 保存
  // 断言 fetch 以 POST 调 /api/admin/domain-schemas，body 含 camelCase schemaId
});
it("D9: 文案不再承诺只读", () => {
  // 渲染 DomainSchemaTab，断言不出现“不能直接改内容”，出现“创建、编辑、删除”
});
```

- [ ] **Step 7: 全前端测 + tsc + lint + 禁词**

Run: `cd frontend && npx vitest run && npx tsc --noEmit && npx eslint src/features/knowledge/DomainSchemaEditor.tsx src/features/knowledge/atlas.tsx`
Run: `bash scripts/check-no-human-takeover.sh <BASE> HEAD`（新文案“系统管理员维护”已去掉，确认无禁词——注意“管理员”非禁词，禁的是“人工/接管/转人工”等）
Expected: PASS + 0 错误 + 0 禁词

- [ ] **Step 8: commit**

```bash
git add frontend/src/features/knowledge/DomainSchemaEditor.tsx frontend/src/features/knowledge/atlas.tsx frontend/src/__tests__/features/knowledge/domainSchemaEditor.test.tsx frontend/src/__tests__/features/knowledge/knowledge.test.tsx
git commit -m "feat(knowledge): D9 domain-schemas 写表单(create/edit/delete 对接现有 CRUD,camelCase 键)+ 改文案承诺

Co-Authored-By: Claude <noreply@anthropic.com>"
```

> 若 D9 的 css 补到独立文件或 knowledge 频道 css，git add 时一并具名加入。

---

## 完工后

全 9 task 完成后：
1. **whole-branch 终审**（最强模型，range = `git merge-base main HEAD`..HEAD）：重点查跨 task 一致性——D10/D11 同改 system-strategy/index.tsx（维度行 + coverage 行不互相破坏）；F14/F16/F17 同改 explore.tsx + today.tsx 的 SSE 区（F16 只接 today 长连接、F17 只修 explore 一次性流、F14 删租户控件，三者不冲突）；F13 措辞禁词复核；D9 camelCase wire 键 vs `UpsertRequest` rename_all 逐字段复核 + 红线（domain_schemas 非知识 chunk）+ create 落 is_active=false 不变量。+ 累积 Minor triage。
2. 终审有 Critical/Important findings → 派 **ONE** fix subagent（完整 findings 列表）。
3. `superpowers:finishing-a-development-branch`：push + 创建 PR（**不自动合并**，除非用户确认），等 CI 双门绿。

## 验收清单（交付时人肉浏览器验收）

- D10：维度配置可建“只观测”维度（participates_in_decision=false）。
- D11：completeness 维度可编辑 anchor_hint + initial_signal 并提交。
- F11：AI 确信标签显来源徽标（强证据/压缩重判）+ tooltip。
- F14：explore 无租户输入框（切租户走顶部 workspace 切换器）。
- F13：command-center gatewayStatus 显中文标签。
- F15：operations 各 tab 拉取中显加载态（区别于空态）。
- F16：today 会话流断连后自动退避重连（断网/服务重启场景）。
- F17：explore 上一轮有结果后再次提交失败，错误横幅正常出现。
- D9：✅domain-schemas 可创建/编辑/删除字段表 + 文案承诺一致（重点验收）。

---

## Self-Review（plan 自查）

**1. Spec 覆盖**：spec 9 条逐条有 task——D10→T1 / D11→T2 / F11→T3 / F14→T4 / F13→T5 / F15→T6 / F16→T7 / F17→T8 / D9→T9。spec 第六节全局约束 → 本 plan Global Constraints 逐条 verbatim。spec 第七节不变量 → 本 plan 不变量节 + 各 task 内联（F16 技术分野、D9 camelCase/is_active、D11 types 先补）。✅ 无遗漏。

**2. Placeholder 扫描**：各 step 含实际代码/命令/期望。测试 step 的 `// 注释` 描述了断言意图但给了可执行骨架（render/fireEvent/expect 真实）——属“沿用本文件既有套路”的合理留白（implementer 须读既有测试文件的 mock 形态），非 TBD。✅

**3. 类型一致性**：`SseHandle`/`createSseReconnector`（T7 定义）→ T7 接入处一致用。`DomainSchemaUpsertBody`/`DomainSchemaFieldDraft`（T9 定义）→ atlas saveSchema 一致用。`CoverageDimension.initial_signal`（T2 补）→ T2 编辑器一致用。`gatewayStatusLabel`（T5 定义）→ T5 :75 一致用。✅

**4. 行号实证**：所有 file:line 已在 writing-plans 阶段基于最新 main（fix/frontend-backend-align-batch4，base e8f353a）重新 grep/Read 实证：system-strategy:1388-1446(D10)/1651-1711(D11)、types:526-531(ProfileDimension)/547-552(CoverageDimension)/51(ConfirmedTag)、TagTrustPanel:93-105、explore:46-61/97/116-117/150-158/217-226、command-center:30-45/75、run_envelope:86-135(32值)、operationsStore 全文、operations:150-365、today:79/125-144/803-829、atlas:485-628(:544/:552 文案)、domain_schemas:93-119(UpsertRequest camelCase)。

