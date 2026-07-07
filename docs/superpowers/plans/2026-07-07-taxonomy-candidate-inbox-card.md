# 标签候选收件箱卡片小白化改造 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把统一收件箱「标签候选」项从走不通的 inline 二元按钮，改造成携带证据 + 命名表单的 rich 卡片，让小白运营能理解并一步完成采纳/驳回。

**Architecture:** 后端 `collect_taxonomy_candidates` 把候选投影成 `action_kind="rich"` + `rich_component="taxonomyCandidateReview"`，并把模型里已有的 evidence/confidence/occurrences/suggestedDisplayName 经 `rich_params` 带全（无需新增 get-by-id 端点）；前端新增共享组件 `TaxonomyCandidateReviewCard`（纯 props，含小白框定 + 证据区 + 命名表单 + 采纳/驳回），ask-human 的 `renderRich` 接线，`system-strategy` 复用它去重。

**Tech Stack:** Rust (Axum) + MongoDB 后端；React 19 + TypeScript + Vite + vitest + @testing-library/react 前端。

## Global Constraints

- 分支基线：本 worktree `worktree-taxonomy-inbox-card`，HEAD `c15bc26`（含 origin/main）。所有改动在此 worktree 内完成。
- AI 全自治红线：新增文案不得含 `human_takeover|takeover|hand-off|人工接管|人工介入|人工托管|接管|人工` 等禁用词；CI `scripts/check-no-human-takeover.sh` 扫 `src/`、`frontend/src/` 新增行。措辞用「纳入标签字典」「AI 今后可稳定使用」。
- AI 不自动核验红线：候选进字典仍需运营点击采纳（保持"AI 提议 + 人工确认"闭环）；本改动不改这一点。
- 零侵入写路径：后端只改 `collect_taxonomy_candidates` 的只读投影，不动 approve/reject 写逻辑与 `canonicalValue` 必填校验。
- 测试只增量叠加：绝不删改既有测试维度；新增用例 append。
- 三闸全绿：`cargo test --lib`（≥350 passed / 0 failed 基线不回归）、前端 `npx vitest run`、`bash scripts/check-no-human-takeover.sh`（0 violations）。
- 未经用户明确许可绝不 commit 到远程 / 开 PR；本地 commit 已获本任务授权。

## File Structure

- 后端 `src/routes/ask_human_inbox.rs`：新增具名纯函数 `taxonomy_candidate_to_inbox_item(&TaxonomyCandidate, now_ms) -> InboxItem`（镜像既有 `escalation_to_inbox_item` / `gap_to_inbox_item` 模式，便于单测）；`collect_taxonomy_candidates` 改为调它；`#[cfg(test)] mod tests` append 投影单测。
- 前端 `frontend/src/components/review/TaxonomyCandidateReviewCard.tsx`（新建）：与其它 rich 卡同目录的共享组件，纯 props + 内部 approve/reject fetch。
- 前端 `frontend/src/components/review/TaxonomyCandidateReviewCard.module.css`（新建）：卡片自带样式，不耦合 system-strategy 样式表。
- 前端 `frontend/src/features/ask-human/index.tsx`：`renderRich` 加 `taxonomyCandidateReview` 分支；`renderInline` 删 `taxonomy_candidate` 分支。
- 前端 `frontend/src/features/system-strategy/index.tsx`：`TaxonomyCandidatesAdmin` 复用新组件去重（若耦合过深则降级：新组件先只服务 ask-human，system-strategy 暂不动，记后续清理）。
- 前端测试 `frontend/src/__tests__/components/review/TaxonomyCandidateReviewCard.test.tsx`（新建）：渲染 + 提交断言。

---

### Task 1: 后端 — 标签候选投影成 rich（具名纯函数 + 单测）

**Files:**
- Modify: `src/routes/ask_human_inbox.rs`（新增 `taxonomy_candidate_to_inbox_item`；`collect_taxonomy_candidates` 改调它，`:160-202`）
- Test: `src/routes/ask_human_inbox.rs` 的 `#[cfg(test)] mod tests`（`:551-658` append）

**Interfaces:**
- Consumes: `crate::models::TaxonomyCandidate`（`models.rs:2898-2921`：`id: Option<ObjectId>`, `scope: String`, `kind: String`, `raw_value: String`, `evidence: Option<String>`, `confidence: i32`, `occurrences: i32`, `last_seen_at: DateTime`, `suggested_display_name: Option<String>`）；`InboxItem`（`:16-53`）；helper `age_hours_of`（`:55`）、`non_empty`（`:62`）。
- Produces: `fn taxonomy_candidate_to_inbox_item(c: &crate::models::TaxonomyCandidate, now_ms: i64) -> InboxItem`——`rich_params` 里键名（供 Task 3 前端消费）：`candidateId`、`scope`、`kind`、`rawValue`、`evidence`、`confidence`、`occurrences`、`suggestedDisplayName`。

- [ ] **Step 1: 写失败单测**

在 `mod tests`（`:657` 的 `}` 之前）append。fixture 直接构造 `TaxonomyCandidate`：

```rust
    fn test_candidate_fixture() -> crate::models::TaxonomyCandidate {
        let now = DateTime::now();
        crate::models::TaxonomyCandidate {
            id: None,
            scope: "global".into(),
            kind: "emotional_state".into(),
            raw_value: "anxious".into(),
            evidence: Some("客户连续两条消息表达担心".into()),
            confidence: 7,
            first_seen_at: now,
            last_seen_at: now,
            occurrences: 3,
            status: "pending".into(),
            reviewed_at: None,
            reviewed_by: None,
            suggested_display_name: Some("焦虑".into()),
        }
    }

    #[test]
    fn taxonomy_candidate_projected_as_rich() {
        let c = test_candidate_fixture();
        // id=None 时 hex 落空串；本测聚焦 rich 分类与富字段，用 None 固定路径。
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        assert_eq!(item.action_kind, "rich");
        assert_eq!(item.rich_component.as_deref(), Some("taxonomyCandidateReview"));
        // title 不再以裸维度键作主语（回归防护：不得再出现 "标签候选：emotional_state"）。
        assert!(!item.title.contains("emotional_state"), "title 不应暴露裸维度键: {}", item.title);
        // 顶层富字段与 relationship_suggestion 对称接出。
        assert_eq!(item.evidence.as_deref(), Some("客户连续两条消息表达担心"));
        assert_eq!(item.confidence, Some(7));
        assert_eq!(item.occurrences, Some(3));
    }

    #[test]
    fn taxonomy_candidate_rich_params_carry_all_fields() {
        let c = test_candidate_fixture();
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        let params = item.rich_params.expect("rich_params 应存在");
        assert_eq!(params.get_str("scope").unwrap(), "global");
        assert_eq!(params.get_str("kind").unwrap(), "emotional_state");
        assert_eq!(params.get_str("rawValue").unwrap(), "anxious");
        assert_eq!(params.get_str("evidence").unwrap(), "客户连续两条消息表达担心");
        assert_eq!(params.get_i32("confidence").unwrap(), 7);
        assert_eq!(params.get_i32("occurrences").unwrap(), 3);
        assert_eq!(params.get_str("suggestedDisplayName").unwrap(), "焦虑");
    }

    #[test]
    fn taxonomy_candidate_optional_fields_omitted_when_absent() {
        let mut c = test_candidate_fixture();
        c.evidence = None;
        c.suggested_display_name = None;
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        let params = item.rich_params.expect("rich_params 应存在");
        // evidence / suggestedDisplayName 缺省时不写键（不产生 null）。
        assert!(params.get("evidence").is_none());
        assert!(params.get("suggestedDisplayName").is_none());
        assert_eq!(item.evidence, None);
        // confidence / occurrences 是非 Option i32，恒写入。
        assert!(params.get("confidence").is_some());
        assert!(params.get("occurrences").is_some());
    }

    #[test]
    fn taxonomy_candidate_serializes_camel_case() {
        let c = test_candidate_fixture();
        let item = taxonomy_candidate_to_inbox_item(&c, 0);
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["actionKind"], "rich");
        assert_eq!(v["richComponent"], "taxonomyCandidateReview");
        assert_eq!(v["confidence"], 7);
        assert_eq!(v["occurrences"], 3);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `touch src/lib.rs && cargo test --lib taxonomy_candidate 2>&1 | tail -15`
Expected: 编译失败 `cannot find function taxonomy_candidate_to_inbox_item`。

- [ ] **Step 3: 写具名纯函数**

在 `collect_taxonomy_candidates`（`:160`）之前插入。镜像 `escalation_to_inbox_item`（`:72-97`）。`rich_params` 用 `Document`，`Option` 字段条件插入：

```rust
/// 单条标签候选 → InboxItem（具名以便单测）。归类 rich：审核 = 给 AI 新造取值
/// 命名并纳入字典，需命名表单，不是简单二元通过/拒绝。富字段 evidence/confidence/
/// occurrences 与 relationship_suggestion 对称接出；rich_params 带全前端渲染所需数据。
fn taxonomy_candidate_to_inbox_item(
    c: &crate::models::TaxonomyCandidate,
    now_ms: i64,
) -> InboxItem {
    let id = c.id.map(|o| o.to_hex()).unwrap_or_default();
    let mut params = doc! {
        "candidateId": id.clone(),
        "scope": c.scope.clone(),
        "kind": c.kind.clone(),
        "rawValue": c.raw_value.clone(),
        "confidence": c.confidence,
        "occurrences": c.occurrences,
    };
    if let Some(ev) = &c.evidence {
        params.insert("evidence", ev.clone());
    }
    if let Some(name) = &c.suggested_display_name {
        params.insert("suggestedDisplayName", name.clone());
    }
    InboxItem {
        source: "taxonomy_candidate".into(),
        id,
        // 人话标题：以 AI 新识别的取值为主语，不暴露裸维度键（维度中文名前端补）。
        title: format!("AI 新识别标签：{}", c.raw_value),
        // 折叠预览：优先 evidence，无则通用框定。
        summary: c
            .evidence
            .clone()
            .unwrap_or_else(|| "AI 在对话中识别到一个尚未收录的取值，请确认是否纳入标签字典".into()),
        severity: "low".into(),
        created_at: Some(c.last_seen_at),
        age_hours: age_hours_of(Some(c.last_seen_at), now_ms),
        action_kind: "rich".into(),
        rich_component: Some("taxonomyCandidateReview".into()),
        rich_params: Some(params),
        category: None,
        question_for_principal: None,
        contact_wxid: None,
        principal_wxid: None,
        evidence: c.evidence.clone(),
        confidence: Some(c.confidence),
        occurrences: Some(c.occurrences),
        kind: None,
        signal_severity: None,
    }
}
```

- [ ] **Step 4: `collect_taxonomy_candidates` 改调纯函数**

把 `:175-201` 的 `.map(|c| { InboxItem {...} })` 整块替换为：

```rust
    Ok(rows
        .into_iter()
        .map(|c| taxonomy_candidate_to_inbox_item(&c, now_ms))
        .collect())
```

- [ ] **Step 5: 跑测试确认通过**

Run: `touch src/lib.rs && cargo test --lib taxonomy_candidate 2>&1 | tail -15`
Expected: 4 个 `taxonomy_candidate_*` 单测 PASS。

- [ ] **Step 6: 基线不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥350 passed / 0 failed。

- [ ] **Step 7: 提交**

```bash
git add src/routes/ask_human_inbox.rs
git commit -m "feat(inbox): 标签候选投影成 rich 卡并带全证据字段"
```

---

### Task 2: 前端 — 新增共享组件 TaxonomyCandidateReviewCard

**Files:**
- Create: `frontend/src/components/review/TaxonomyCandidateReviewCard.tsx`
- Create: `frontend/src/components/review/TaxonomyCandidateReviewCard.module.css`
- Test: `frontend/src/__tests__/components/review/TaxonomyCandidateReviewCard.test.tsx`

**Interfaces:**
- Consumes: `api` from `../../lib/api`（`api.postRaw<T>(url, body)` 返回 `{ ok: boolean; status: number; data: T }`，见 `lib/api.ts:98`；`api.post(url, body)`）。维度中文名映射复用 `../../lib/reviewLabels` 的 `labelOf`（`reviewLabels.ts:334`）+ 本文件新增的 `TAXONOMY_KIND_LABELS`。
- Produces: `export function TaxonomyCandidateReviewCard(props: { candidate: TaxonomyCandidate; onDone: () => void })`，其中
  ```ts
  interface TaxonomyCandidate {
    id: string;
    scope: string;
    kind: string;
    rawValue: string;
    evidence?: string;
    confidence?: number;
    occurrences?: number;
    suggestedDisplayName?: string;
  }
  ```
  approve POST body：`{ canonicalValue: { id, label, aliases: string[], description?: string } }`；reject POST body：`{ reason: string }`。

- [ ] **Step 1: 写失败测试**

Create `frontend/src/__tests__/components/review/TaxonomyCandidateReviewCard.test.tsx`。参照 `ChunkReviewCard.test.tsx` 与 `taxonomyFlags.test.tsx` 的 mock 模式：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { TaxonomyCandidateReviewCard } from "../../../components/review/TaxonomyCandidateReviewCard";
import { api } from "../../../lib/api";

vi.mock("../../../components/review/TaxonomyCandidateReviewCard.module.css", () => ({
  default: new Proxy({}, { get: (_t, key) => String(key) }),
}));

vi.mock("../../../lib/api", () => ({
  api: {
    postRaw: vi.fn().mockResolvedValue({ ok: true, status: 200, data: {} }),
    post: vi.fn().mockResolvedValue({}),
  },
}));

const candidate = {
  id: "cand1",
  scope: "global",
  kind: "emotional_state",
  rawValue: "anxious",
  evidence: "客户连续两条消息表达担心",
  confidence: 7,
  occurrences: 3,
  suggestedDisplayName: "焦虑",
};

describe("TaxonomyCandidateReviewCard", () => {
  beforeEach(() => vi.clearAllMocks());

  it("渲染 rawValue、证据、命名表单预填（显示名取 suggestedDisplayName）", () => {
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={() => {}} />);
    expect(screen.getByText(/anxious/)).toBeTruthy();
    expect(screen.getByText(/客户连续两条消息表达担心/)).toBeTruthy();
    const label = screen.getByLabelText(/显示名/) as HTMLInputElement;
    expect(label.value).toBe("焦虑");
    const idInput = screen.getByLabelText(/canonical id/i) as HTMLInputElement;
    expect(idInput.value).toBe("anxious");
  });

  it("采纳发 canonicalValue body 并在成功后回调 onDone", async () => {
    const onDone = vi.fn();
    const postRaw = vi.spyOn(api, "postRaw").mockResolvedValue({ ok: true, status: 200, data: {} } as never);
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={onDone} />);
    fireEvent.click(screen.getByRole("button", { name: "采纳" }));
    await waitFor(() => expect(postRaw).toHaveBeenCalled());
    const [url, body] = postRaw.mock.calls[0] as [string, { canonicalValue: Record<string, unknown> }];
    expect(url).toContain("/api/admin/taxonomy-candidates/cand1/approve");
    expect(body.canonicalValue.id).toBe("anxious");
    expect(body.canonicalValue.label).toBe("焦虑");
    await waitFor(() => expect(onDone).toHaveBeenCalled());
  });

  it("id 或显示名清空后采纳被拦截，不发请求", async () => {
    const postRaw = vi.spyOn(api, "postRaw");
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={() => {}} />);
    fireEvent.change(screen.getByLabelText(/显示名/), { target: { value: "  " } });
    fireEvent.click(screen.getByRole("button", { name: "采纳" }));
    expect(postRaw).not.toHaveBeenCalled();
    expect(screen.getByText(/不能为空/)).toBeTruthy();
  });

  it("409 视为已存在提示，不当错误", async () => {
    vi.spyOn(api, "postRaw").mockResolvedValue({ ok: false, status: 409, data: { message: "该字典条目已存在" } } as never);
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "采纳" }));
    await waitFor(() => expect(screen.getByText(/已存在/)).toBeTruthy());
  });

  it("驳回需填原因，填后 POST reason 并回调 onDone", async () => {
    const onDone = vi.fn();
    const post = vi.spyOn(api, "post").mockResolvedValue({} as never);
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={onDone} />);
    fireEvent.click(screen.getByRole("button", { name: "驳回" }));
    fireEvent.change(screen.getByLabelText(/驳回原因/), { target: { value: "无业务相关性" } });
    fireEvent.click(screen.getByRole("button", { name: "确认驳回" }));
    await waitFor(() => expect(post).toHaveBeenCalledWith(
      expect.stringContaining("/api/admin/taxonomy-candidates/cand1/reject"),
      { reason: "无业务相关性" },
    ));
    await waitFor(() => expect(onDone).toHaveBeenCalled());
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/components/review/TaxonomyCandidateReviewCard.test.tsx 2>&1 | tail -15`
Expected: FAIL — 找不到模块 `TaxonomyCandidateReviewCard`。

- [ ] **Step 3: 写 CSS module**

Create `frontend/src/components/review/TaxonomyCandidateReviewCard.module.css`（自带样式，不依赖 system-strategy）：

```css
.card { display: flex; flex-direction: column; gap: 10px; }
.intro { font-size: 13px; color: #444; line-height: 1.6; }
.rawValue { font-weight: 600; color: #1a1a1a; }
.evidence { font-size: 12px; color: #666; display: flex; flex-direction: column; gap: 2px; }
.form { display: flex; flex-direction: column; gap: 8px; margin-top: 4px; }
.field { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: #555; }
.input, .textarea { border: 1px solid #d0d0d0; border-radius: 6px; padding: 6px 8px; font-size: 13px; }
.textarea { min-height: 60px; resize: vertical; }
.buttons { display: flex; gap: 8px; }
.error { color: #c0392b; font-size: 12px; }
.info { color: #1e7e34; font-size: 12px; }
```

- [ ] **Step 4: 写组件**

Create `frontend/src/components/review/TaxonomyCandidateReviewCard.tsx`。命名/采纳/驳回逻辑照搬 `system-strategy` 的 `openApprove`/`submitApprove`/`submitReject`（`index.tsx:1044-1127`），改为纯 props + 内部 state：

```tsx
import { useState } from "react";
import { api } from "../../lib/api";
import { labelOf } from "../../lib/reviewLabels";
import styles from "./TaxonomyCandidateReviewCard.module.css";

// 维度键 → 中文名。取值来自运行时产候选的写入点（decision/reaction/gateway）：
// customer_stage / intent_level / objection_type / concern_type / emotional_state
// / relationship_type。未收录经 labelOf 回落原 key，不硬失败。
export const TAXONOMY_KIND_LABELS: Record<string, string> = {
  customer_stage: "客户阶段",
  intent_level: "意向强度",
  objection_type: "异议类型",
  concern_type: "顾虑类型",
  emotional_state: "情绪状态",
  relationship_type: "关系类型",
};

export interface TaxonomyCandidate {
  id: string;
  scope: string;
  kind: string;
  rawValue: string;
  evidence?: string;
  confidence?: number;
  occurrences?: number;
  suggestedDisplayName?: string;
}

export function TaxonomyCandidateReviewCard({
  candidate,
  onDone,
}: {
  candidate: TaxonomyCandidate;
  onDone: () => void;
}) {
  const [mode, setMode] = useState<"approve" | "reject">("approve");
  const [id, setId] = useState(candidate.rawValue);
  const [label, setLabel] = useState(candidate.suggestedDisplayName || candidate.rawValue);
  const [aliases, setAliases] = useState("");
  const [description, setDescription] = useState(candidate.evidence ?? "");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [acting, setActing] = useState(false);

  const kindLabel = labelOf(TAXONOMY_KIND_LABELS, candidate.kind);

  async function submitApprove() {
    if (!id.trim() || !label.trim()) {
      setError("canonical id 与显示名不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      const aliasList = aliases.split(/[,，]/).map((a) => a.trim()).filter((a) => a.length > 0);
      const res = await api.postRaw<{ error?: string; message?: string }>(
        `/api/admin/taxonomy-candidates/${candidate.id}/approve`,
        {
          canonicalValue: {
            id: id.trim(),
            label: label.trim(),
            aliases: aliasList,
            description: description.trim() || undefined,
          },
        },
      );
      if (res.status === 409) {
        setInfo(res.data?.message ?? "该字典条目已存在，候选已标记采纳。");
      } else if (!res.ok) {
        setError(res.data?.message ?? res.data?.error ?? `HTTP ${res.status}`);
        return;
      } else {
        setInfo(`已采纳：${id.trim()}`);
      }
      onDone();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  async function submitReject() {
    if (!reason.trim()) {
      setError("驳回原因不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    try {
      await api.post(`/api/admin/taxonomy-candidates/${candidate.id}/reject`, { reason: reason.trim() });
      onDone();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  return (
    <div className={styles.card}>
      <p className={styles.intro}>
        AI 在和客户对话时，识别到一个「{kindLabel}」维度上尚未收录的取值：
        <span className={styles.rawValue}> {candidate.rawValue}</span>
        。采纳后它会作为正式标签存入字典，今后 AI 可稳定使用；驳回则丢弃这次建议。
      </p>
      {(candidate.evidence || candidate.confidence !== undefined || candidate.occurrences !== undefined) && (
        <div className={styles.evidence}>
          {candidate.evidence && <span>判断依据：{candidate.evidence}</span>}
          {candidate.confidence !== undefined && <span>置信度：{candidate.confidence}</span>}
          {candidate.occurrences !== undefined && <span>出现次数：{candidate.occurrences}</span>}
        </div>
      )}
      {error && <div className={styles.error}>{error}</div>}
      {info && <div className={styles.info}>{info}</div>}

      {mode === "approve" && (
        <div className={styles.form}>
          <label className={styles.field}>
            <span>canonical id（建议英文 slug，如 price_objection）</span>
            <input className={styles.input} value={id} onChange={(e) => setId(e.target.value)} />
          </label>
          <label className={styles.field}>
            <span>显示名</span>
            <input className={styles.input} value={label} onChange={(e) => setLabel(e.target.value)} />
          </label>
          <label className={styles.field}>
            <span>别名（逗号分隔，可空；rawValue 会自动并入）</span>
            <input className={styles.input} value={aliases} onChange={(e) => setAliases(e.target.value)} />
          </label>
          <label className={styles.field}>
            <span>描述（可空）</span>
            <textarea className={styles.textarea} value={description} onChange={(e) => setDescription(e.target.value)} />
          </label>
          <div className={styles.buttons}>
            <button type="button" onClick={() => void submitApprove()} disabled={acting}>采纳</button>
            <button type="button" onClick={() => { setMode("reject"); setError(null); }} disabled={acting}>驳回</button>
          </div>
        </div>
      )}

      {mode === "reject" && (
        <div className={styles.form}>
          <label className={styles.field}>
            <span>驳回原因</span>
            <input
              className={styles.input}
              value={reason}
              placeholder="如：无业务相关性 / 与现有条目重复"
              onChange={(e) => setReason(e.target.value)}
            />
          </label>
          <div className={styles.buttons}>
            <button type="button" onClick={() => void submitReject()} disabled={acting}>确认驳回</button>
            <button type="button" onClick={() => { setMode("approve"); setError(null); }} disabled={acting}>取消</button>
          </div>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/components/review/TaxonomyCandidateReviewCard.test.tsx 2>&1 | tail -20`
Expected: 5 个用例 PASS。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/components/review/TaxonomyCandidateReviewCard.tsx frontend/src/components/review/TaxonomyCandidateReviewCard.module.css frontend/src/__tests__/components/review/TaxonomyCandidateReviewCard.test.tsx
git commit -m "feat(review): 新增标签候选命名审核卡 TaxonomyCandidateReviewCard"
```

---

### Task 3: 前端 — ask-human 收件箱接线（rich 分派 + 删 inline 分支）

**Files:**
- Modify: `frontend/src/features/ask-human/index.tsx`（import 新组件；`renderRich` 加分支 `:44-55`；`renderInline` 删 `taxonomy_candidate` 分支 `:63-73`）

**Interfaces:**
- Consumes: `TaxonomyCandidateReviewCard`（Task 2）；`InboxItem`（`lib/inboxApi.ts:3`，`richParams?: Record<string, unknown>`）；`renderRich(item, onDone)` 签名不变。
- Produces: `richComponent === "taxonomyCandidateReview"` 在收件箱内渲染命名卡。

- [ ] **Step 1: import 新组件**

在既有 rich 卡 import 组（`:10-13`）后追加：

```tsx
import { TaxonomyCandidateReviewCard } from "../../components/review/TaxonomyCandidateReviewCard";
```

- [ ] **Step 2: `renderRich` 加分支**

在 `renderRich` 的 `switch (item.richComponent)`（`:44`）里，`default` 之前插入：

```tsx
    case "taxonomyCandidateReview": {
      const c = item.richParams ?? {};
      return (
        <TaxonomyCandidateReviewCard
          candidate={{
            id: String(c.candidateId ?? item.id),
            scope: String(c.scope ?? ""),
            kind: String(c.kind ?? ""),
            rawValue: String(c.rawValue ?? ""),
            evidence: c.evidence != null ? String(c.evidence) : undefined,
            confidence: c.confidence != null ? Number(c.confidence) : undefined,
            occurrences: c.occurrences != null ? Number(c.occurrences) : undefined,
            suggestedDisplayName: c.suggestedDisplayName != null ? String(c.suggestedDisplayName) : undefined,
          }}
          onDone={onDone}
        />
      );
    }
```

- [ ] **Step 3: 删 `renderInline` 的 taxonomy_candidate 分支**

删除 `renderInline`（`:59`）里整个 `case "taxonomy_candidate":`（`:63-73`）——它不再走 inline。`relationship_suggestion` / `gap_signal` 分支保持不动。

- [ ] **Step 4: 类型检查 + 全量前端测试**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -10`
Expected: 0 errors。

Run: `cd frontend && npx vitest run 2>&1 | tail -15`
Expected: 全绿（含 Task 2 新用例；既有 ask-human 测试不回归）。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/ask-human/index.tsx
git commit -m "feat(inbox): 标签候选走 rich 命名卡，删除 inline 空按钮分支"
```

---

### Task 4: 前端 — system-strategy 复用新组件去重 + 三闸终验

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx`（`TaxonomyCandidatesAdmin` 渲染新组件，删内联表单副本）
- Verify: 全仓三闸

**Interfaces:**
- Consumes: `TaxonomyCandidateReviewCard`（Task 2）。
- Produces: `TaxonomyCandidatesAdmin` 的每个 pending 候选卡改用共享组件；`reload()` 作为 `onDone`。

> **降级条款（Global Constraints 已列）**：若 `TaxonomyCandidatesAdmin` 的表单 state 与列表状态耦合过深、复用会牵动 approved/rejected/all 三个 tab 的展示逻辑，则**跳过本 Task 的重构**，改为在计划末尾「后续清理」记一条「system-strategy 与 ask-human 命名表单重复，待统一」，直接进入 Step 4 三闸终验。判断标准：若删内联表单后 `taxonomyFlags.test.tsx` 无法在不改断言的前提下通过，即视为耦合过深，走降级。

- [ ] **Step 1: import 新组件**

在 `system-strategy/index.tsx` 顶部 import 组追加：

```tsx
import { TaxonomyCandidateReviewCard } from "../../components/review/TaxonomyCandidateReviewCard";
```

- [ ] **Step 2: pending 候选改用共享卡**

在 `TaxonomyCandidatesAdmin` 的 `items.map`（`:1159`）里，把 `item.status === "pending"` 时的内联「采纳/驳回按钮 + expandedId 表单 + rejectingId 表单」（`:1191-1267`）整块替换为：

```tsx
            {item.status === "pending" && (
              <TaxonomyCandidateReviewCard
                candidate={{
                  id: item.id,
                  scope: item.scope,
                  kind: item.kind,
                  rawValue: item.rawValue,
                  evidence: item.evidence ?? undefined,
                  confidence: item.confidence,
                  occurrences: item.occurrences,
                  suggestedDisplayName: item.suggestedDisplayName ?? undefined,
                }}
                onDone={() => void reload()}
              />
            )}
```

随后删除仅服务旧内联表单的 state 与函数：`approveDraft`/`setApproveDraft`、`expandedId`/`setExpandedId`、`rejectingId`/`setRejectingId`、`rejectReason`/`setRejectReason`、`openApprove`、`openReject`、`submitApprove`、`submitReject`（`:1013-1127` 范围内）。保留 `statusFilter`、`reload`、`error`/`info`、`acting`（仍被列表容器/其它 tab 用到——逐一确认后再删，不留 `unused var`）。

- [ ] **Step 3: system-strategy 测试不回归**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/taxonomyFlags.test.tsx 2>&1 | tail -15`
Expected: 全绿。若因内联表单被移除而失败 → 触发降级条款：`git checkout frontend/src/features/system-strategy/index.tsx` 还原本 Task 改动，改记「后续清理」，进 Step 4。

- [ ] **Step 4: 三闸终验**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -5`
Expected: 0 errors。

Run: `cd frontend && npx vitest run 2>&1 | tail -15`
Expected: 全绿。

Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥350 passed / 0 failed。

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -5`
Expected: 0 violations。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/system-strategy/index.tsx
git commit -m "refactor(system-strategy): 标签候选复用共享命名卡去重"
```

（若走降级条款，本 Step 改为提交计划文档的「后续清理」更新，不动 system-strategy。）

---

## 后续清理（非本计划范围）

- 收件箱其它来源（用户提过"还有很多问题"）——另开工单逐一排查。
- 若 Task 4 走降级条款：system-strategy 与 ask-human 的命名表单仍有一份重复，待后续统一。
