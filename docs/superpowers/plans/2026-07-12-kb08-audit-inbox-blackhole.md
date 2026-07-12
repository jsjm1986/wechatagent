# KB-08 修复实施计划：auto_verify 降级切片人审黑洞

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。Steps 用 checkbox 跟踪。**红线：改任何代码前必 100% 读懂相关代码，引用必当场 Read/Grep 亲验 file:line，不猜。**

**Goal:** 让 auto_verify 分诊出的 `needs_human_audit` 切片重新出现在人审收件箱（并可区分"AI 预审通过·待复核"），关闭台账 KB-08 黑洞。

**Architecture:** 两个审核收件箱查询从只认 `needs_review` 扩为 `{needs_review, needs_human_audit}`：统一收件箱（ask_human_inbox.rs）抽纯函数 `knowledge_review_statuses()` 供列表查询 + summary 计数共用（列表/计数同源不漂移），InboxItem 加 `integrity_status` 字段供前端区分；AI digest 收件箱（digest_inbox.rs）扩 pending_review 分支 + 抽纯函数 `pending_review_card_labels` 区分 title/origin。前端 InboxRow 加可选 `tag` prop，对 needs_human_audit 渲染 held 色小 pill。写端 verify.rs 分诊逻辑不动（红线未破）。

**Tech Stack:** Rust 2021 / Axum / MongoDB；React 19 + TS + Vite（frontend/）。后端纯函数 `cargo test --lib`（无需 Docker）；前端 vitest + `npm run build`（不占 Rust target）。

## Global Constraints
- **改前必 100% 读懂 + 引用必亲验 file:line**（CLAUDE.md 最高红线）。行号会漂——每个改码 Task 的 Step 1 必先 Read/Grep 亲验当前真实行号再改。
- **严格限定范围**：只改 `ask_human_inbox.rs`（纯函数+查询+计数+InboxItem 字段）、`digest_inbox.rs`（分支+纯函数）、`ask-human/index.tsx`（InboxRow tag+renderItem）、`AskHuman.css`（tag 样式）、`lib/inboxApi.ts`（类型）。**不改** `verify.rs` 写端分诊逻辑、前端计数展示（AutoVerifyPanel.tsx / quality/index.tsx）、迁移、配置。
- **baseline 不回退**：`cargo test --lib` ≥ 350 passed / 0 failed。新增测试只增不减。
- **前端色纪律**：tag tone 只用现有 inboxBadge tones（`brand/scheduled/held/running/neutral/blocked`），**不新造颜色**；needs_human_audit 用 `held`（琥珀=等待/待复核），避开"主操作蓝/AI 身份紫"红线。
- **no-human-takeover lint**：src/ + frontend/src/ 新增行不得含 `人工接管/takeover/hand-off/人工介入/人工托管/接管/人工`。本修复用「预审/复核/待审/审核」措辞。
- **设计文档**：`docs/superpowers/specs/2026-07-12-kb08-audit-inbox-blackhole-design.md`。
- **台账**：`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KB-08。

## 亲验的现有代码事实（实现者仍须自己 Read 确认当前行号）
- `src/routes/ask_human_inbox.rs`：`InboxItem` 结构体 `:16`（`#[serde(rename_all="camelCase")]`，字段用 `#[serde(skip_serializing_if="Option::is_none")]` 惯例）；`collect_knowledge_review` `:114`，查询 `doc!{"workspace_id":ws,"integrity_status":"needs_review"}` 于 `:124`，其 InboxItem 构造 `:133`（source="knowledge_review"）；`ask_human_summary` 知识计数 `doc!{...,"integrity_status":"needs_review"}` 于 `:520`。**8 处 InboxItem 构造点**（:76/:133/:180/:251/:280/:345/:390/:439，另注意 :657 `source:"rule"` 若也是 InboxItem 构造需一并补字段）——加结构体字段后 `cargo check` 会用 E0063 标出每个缺字段处。
- `OperationKnowledgeChunk` 有 `integrity_status: Option<String>` 字段（digest_inbox.rs:423 `c.integrity_status.clone().unwrap_or_default()` 亲证）。
- `src/routes/knowledge/digest_inbox.rs`：pending_review 分支 `if integrity == "needs_review" && updated_ms >= cutoff_ms {`（`:431`）；分支内构造 `InboxCardView`（结构体 `:220`），title `"待审反例：…"`/`"待审切片：…"`、origin `"negative_example_review"`/`"pending_review"`；`is_negative_example = c.chunk_type == "negative_example"`；`inbox_pending_review_priority(&c.chunk_type)` 决定 priority（不动）；`integrity = c.integrity_status.clone().unwrap_or_default()`（:423）。
- `src/routes/knowledge/verify.rs`（**不改**，仅背景）：`decide_auto_verify_status`（:534-539）只在 quote+anchor 都真时返 "verified"；`enforce_verified_needs_human_audit`（:554-557）把 "verified" 强制降级 "needs_human_audit" → 每个 needs_human_audit 必有 quote+anchor。
- `frontend/src/features/ask-human/index.tsx`：`InboxRow`（`:110`，props `{ badge:{label,tone}, title, preview, children }`，badge 渲染 `<span className={"inboxBadge inboxBadge--"+badge.tone}>` 于 `:125`）；renderItem（`:217`）传 `badge={{label: meta?.label ?? item.source, tone: SOURCE_TONE[item.source] ?? "neutral"}}`、`title={item.title}`。
- `frontend/src/lib/inboxApi.ts`：`InboxItem` 接口 `:3`（含 `source/title/actionKind/richParams?` 等，camelCase）。
- `frontend/src/features/ask-human/AskHuman.css`：inboxBadge tone 类 `:228-233`（`.inboxBadge--held { background: var(--fill-held); }` 等，均引 tokens.css `--fill-*` 语义色）。
- vitest 范式：`frontend/src/__tests__/features/ask-human/InboxRow.collapse.test.tsx`（`import { InboxRow } from "../../../features/ask-human/index"`，`render(<InboxRow badge={{label,tone}} title=... preview=...>...`）。

---

## Task 1: ask_human_inbox.rs —— 纯函数 + 查询/计数同源 + InboxItem 字段

**Files:**
- Modify: `src/routes/ask_human_inbox.rs`

**Interfaces:**
- Produces: `pub(crate) fn knowledge_review_statuses() -> [&'static str; 2]`（返回 `["needs_review", "needs_human_audit"]`）；`InboxItem.integrity_status: Option<String>`（序列化 `integrityStatus`，Task 3 前端消费）。

- [ ] **Step 1: 先读懂（红线）**

Read `src/routes/ask_human_inbox.rs:1-160`（InboxItem 结构体全字段 + collect_knowledge_review）+ `:505-522`（ask_human_summary 知识计数）。Grep `grep -nE "InboxItem \{" src/routes/ask_human_inbox.rs` 亲验所有构造点当前行号。确认 InboxItem 字段用 `#[serde(skip_serializing_if="Option::is_none")]` 惯例、`OperationKnowledgeChunk` 有 `integrity_status: Option<String>`。**说不清就继续读。**

- [ ] **Step 2: 写失败的纯函数单测**

在 `ask_human_inbox.rs` 末尾加（若已有 `#[cfg(test)] mod tests` 则并入）：

```rust
#[cfg(test)]
mod kb08_tests {
    use super::*;

    #[test]
    fn knowledge_review_statuses_includes_needs_human_audit() {
        // KB-08 病根锚死：审核收件箱必须同时认 needs_human_audit,
        // 否则 auto_verify 分诊出的切片从收件箱消失(黑洞)。防回退成只查 needs_review。
        let s = knowledge_review_statuses();
        assert!(s.contains(&"needs_review"), "必须含 needs_review");
        assert!(s.contains(&"needs_human_audit"), "必须含 needs_human_audit(KB-08 黑洞根因)");
    }
}
```

- [ ] **Step 3: 跑测试确认失败（编译错误）**

Run: `cargo test --lib knowledge_review_statuses 2>&1 | tail -15`
Expected: 编译错误 `cannot find function knowledge_review_statuses`（TDD red）。

- [ ] **Step 4: 实现纯函数**

在 `ask_human_inbox.rs`（`collect_knowledge_review` 上方或文件顶部函数区）加：

```rust
/// KB-08：审核收件箱认可的知识切片 integrity_status 集合。
/// needs_review = AI 起草待审；needs_human_audit = auto_verify 预审通过、待人复核。
/// 二者都须进人审收件箱(否则 needs_human_audit 切片成黑洞)。列表查询与 summary 计数共用本函数,防漂移。
pub(crate) fn knowledge_review_statuses() -> [&'static str; 2] {
    ["needs_review", "needs_human_audit"]
}
```

- [ ] **Step 5: 查询 + 计数改用 $in（同源）**

`collect_knowledge_review` 的 `:124` 查询改为：

```rust
            doc! { "workspace_id": ws, "integrity_status": { "$in": knowledge_review_statuses().to_vec() } },
```

`ask_human_summary` 的 `:520` 计数改为：

```rust
        .count_documents(doc! { "workspace_id": ws, "integrity_status": { "$in": knowledge_review_statuses().to_vec() } }, None)
```

（`&[&str]` 转 BSON 数组：`.to_vec()` 得 `Vec<&str>`，`doc!` 宏可接受。若类型不符，用 `bson::to_bson(&knowledge_review_statuses().to_vec())` 或显式 `mongodb::bson::Bson::Array(knowledge_review_statuses().iter().map(|s| Bson::from(*s)).collect())`——实现者按 cargo check 报错择一，务必编译通过。）

- [ ] **Step 6: InboxItem 加字段 + 填充**

InboxItem 结构体（`:16` 起）在末字段后加：

```rust
    // KB-08：知识切片核验状态(needs_review / needs_human_audit),供前端区分"待审"vs"AI预审通过·待复核"。
    // 仅 knowledge_review 来源填充,其余来源恒 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity_status: Option<String>,
```

`collect_knowledge_review` 的 InboxItem 构造（`:133`）加 `integrity_status: c.integrity_status.clone(),`。

- [ ] **Step 7: 补齐其余构造点的 None（靠 cargo check 兜底）**

Run: `cargo check --lib 2>&1 | grep -E "E0063|missing field|integrity_status" | head -20`
对每个 E0063「missing field `integrity_status`」的 InboxItem 构造点补 `integrity_status: None,`。重跑直到 0 error。

- [ ] **Step 8: 跑测试确认通过 + baseline**

Run: `cargo test --lib knowledge_review_statuses 2>&1 | tail -8` → PASS。
Run: `cargo test --lib 2>&1 | tail -8` → `ok. N passed; 0 failed`，N ≥ 350。
（若本地磁盘满编译失败，记录并交 CI baseline gate 验证。）

- [ ] **Step 9: Commit**

```bash
git add src/routes/ask_human_inbox.rs
git commit -m "fix(inbox): 统一收件箱认 needs_human_audit,查询/计数同源(KB-08)

抽 knowledge_review_statuses()={needs_review,needs_human_audit},collect_knowledge_review 查询+summary 计数共用($in,防列表/计数漂移)。
InboxItem 加 integrity_status 字段供前端区分。根治 auto_verify 分诊切片从人审收件箱消失的黑洞。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: digest_inbox.rs —— pending_review 分支扩展 + 区分标签纯函数

**Files:**
- Modify: `src/routes/knowledge/digest_inbox.rs`

**Interfaces:**
- Produces: `fn pending_review_card_labels(integrity_status: &str, base_title: &str, is_negative_example: bool) -> (String, String)`（返回 `(title, origin)`）。

- [ ] **Step 1: 先读懂（红线）**

Read `src/routes/knowledge/digest_inbox.rs:410-462`（pending_review 分支全貌 + title/origin 现有分支）。确认 `integrity`（:423）、`is_negative_example`（:432）、`inbox_pending_review_priority`（:435 调用，不动）、cutoff_ms 窗口（:431）。**说不清就继续读。**

- [ ] **Step 2: 写失败的纯函数单测**

在 `digest_inbox.rs` 的 `#[cfg(test)] mod tests`（无则新建）加：

```rust
    #[test]
    fn pending_review_card_labels_distinguishes_human_audit() {
        // needs_human_audit → "AI预审通过待复核" + origin human_audit_pending
        let (title, origin) = pending_review_card_labels("needs_human_audit", "价格政策", false);
        assert!(title.contains("预审") && title.contains("价格政策"), "title={title}");
        assert_eq!(origin, "human_audit_pending");
        // needs_review 反例 → 保持原"待审反例"/negative_example_review
        let (t2, o2) = pending_review_card_labels("needs_review", "反面话术", true);
        assert!(t2.contains("待审反例") && t2.contains("反面话术"), "t2={t2}");
        assert_eq!(o2, "negative_example_review");
        // needs_review 普通 → 保持原"待审切片"/pending_review
        let (t3, o3) = pending_review_card_labels("needs_review", "常规切片", false);
        assert!(t3.contains("待审切片") && t3.contains("常规切片"), "t3={t3}");
        assert_eq!(o3, "pending_review");
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test --lib pending_review_card_labels 2>&1 | tail -15`
Expected: `cannot find function pending_review_card_labels`（TDD red）。

- [ ] **Step 4: 实现纯函数**

在 `digest_inbox.rs`（`inbox_pending_review_priority` 附近）加：

```rust
/// KB-08：pending_review 卡片的 title/origin。needs_human_audit = auto_verify 预审通过待人复核,
/// 与 needs_review(未审/反例) 区分。返回 (title, origin)。
fn pending_review_card_labels(
    integrity_status: &str,
    base_title: &str,
    is_negative_example: bool,
) -> (String, String) {
    if integrity_status == "needs_human_audit" {
        return (
            format!("AI预审通过待复核：{base_title}"),
            "human_audit_pending".to_string(),
        );
    }
    if is_negative_example {
        return (
            format!("待审反例：{base_title}"),
            "negative_example_review".to_string(),
        );
    }
    (format!("待审切片：{base_title}"), "pending_review".to_string())
}
```

- [ ] **Step 5: 分支条件扩展 + 改用纯函数**

`:431` 条件从 `if integrity == "needs_review" && updated_ms >= cutoff_ms {` 改为：

```rust
        if ["needs_review", "needs_human_audit"].contains(&integrity.as_str())
            && updated_ms >= cutoff_ms
        {
            let is_negative_example = c.chunk_type == "negative_example";
            let (card_title, card_origin) =
                pending_review_card_labels(&integrity, &title, is_negative_example);
```

分支内 InboxCardView 的 `title:` 改用 `card_title`、`origin:` 改用 `card_origin.into()`（或 `card_origin`——按字段类型），`context_summary` 保持原逻辑不变，priority 保持 `inbox_pending_review_priority(&c.chunk_type)` 不变。删除原 `is_negative_example` 内联 if/else title 与 origin（已被纯函数取代），避免重复。

- [ ] **Step 6: 跑测试确认通过 + baseline**

Run: `cargo test --lib pending_review_card_labels 2>&1 | tail -8` → PASS。
Run: `cargo test --lib 2>&1 | tail -8` → N ≥ 350 / 0 failed。

- [ ] **Step 7: Commit**

```bash
git add src/routes/knowledge/digest_inbox.rs
git commit -m "fix(inbox): AI digest 收件箱 pending_review 认 needs_human_audit(KB-08)

分支条件扩为 {needs_review,needs_human_audit};抽 pending_review_card_labels 纯函数区分
title/origin(human_audit_pending vs pending_review/negative_example_review)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: 前端 —— InboxRow tag prop + renderItem + 样式 + 类型

**Files:**
- Modify: `frontend/src/features/ask-human/index.tsx`（InboxRow + renderItem）
- Modify: `frontend/src/features/ask-human/AskHuman.css`（tag 样式）
- Modify: `frontend/src/lib/inboxApi.ts`（InboxItem 类型加 integrityStatus?）
- Test: `frontend/src/__tests__/features/ask-human/InboxRow.collapse.test.tsx`（或新建 InboxRow.tag.test.tsx）

**Interfaces:**
- Consumes: 后端 InboxItem 的 `integrityStatus`（Task 1 产出）。

- [ ] **Step 1: 先读懂（红线）**

Read `frontend/src/features/ask-human/index.tsx:108-135`（InboxRow 定义）+ `:212-232`（renderItem）；`frontend/src/lib/inboxApi.ts:1-25`（InboxItem 接口）；`frontend/src/features/ask-human/AskHuman.css:220-235`（inboxBadge tone 类）。确认 tone `held` 存在（`.inboxBadge--held`）。**说不清就继续读。**

- [ ] **Step 2: inboxApi.ts 加类型**

`InboxItem` 接口加：`integrityStatus?: string;`

- [ ] **Step 3: InboxRow 加 tag prop + 写 vitest（红）**

在 `InboxRow.collapse.test.tsx` 加用例：

```tsx
  it("传 tag 时渲染 pill,不传时不渲染", () => {
    const { rerender } = render(
      <InboxRow badge={{ label: "知识核验", tone: "brand" }} title="切片A" preview="" tag={{ label: "AI预审通过·待复核", tone: "held" }}>
        <div>body</div>
      </InboxRow>,
    );
    expect(screen.getByText("AI预审通过·待复核")).toBeInTheDocument();
    rerender(
      <InboxRow badge={{ label: "知识核验", tone: "brand" }} title="切片B" preview="">
        <div>body</div>
      </InboxRow>,
    );
    expect(screen.queryByText("AI预审通过·待复核")).toBeNull();
  });
```

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/InboxRow.collapse.test.tsx 2>&1 | tail -15`
Expected: 新用例 FAIL（tag prop 不存在 / pill 未渲染）。

- [ ] **Step 4: InboxRow 实现 tag**

InboxRow props 加可选 `tag?: { label: string; tone: string }`；在 `inboxRowTitle`（`:126`）后渲染：

```tsx
        {tag && <span className={`inboxTag inboxTag--${tag.tone}`}>{tag.label}</span>}
```

props 解构与类型同步加 `tag`。

- [ ] **Step 5: renderItem 传 tag**

renderItem（`:217`）的 `<InboxRow ...>` 加：

```tsx
                  tag={
                    item.source === "knowledge_review" && item.integrityStatus === "needs_human_audit"
                      ? { label: "AI预审通过·待复核", tone: "held" }
                      : undefined
                  }
```

- [ ] **Step 6: AskHuman.css 加 tag 样式**

在 inboxBadge tone 类附近加（小 pill，复用 held 琥珀 token，不新造色）：

```css
.inboxTag {
  display: inline-flex;
  align-items: center;
  padding: 1px 8px;
  margin-left: 6px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 500;
  white-space: nowrap;
}
.inboxTag--held { background: var(--fill-held); color: var(--ink-1); }
```

（若 `.inboxBadge` 基类已有等价 padding/radius/font，实现者可让 `.inboxTag` 复用其声明、只补 margin-left——按现有文件择优，不重复造轮子。）

- [ ] **Step 7: vitest 转绿 + build**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/InboxRow.collapse.test.tsx 2>&1 | tail -10` → 全 PASS。
Run: `cd frontend && npm run build 2>&1 | tail -15` → tsc + vite 编译 0 error。

- [ ] **Step 8: Commit**

```bash
git add frontend/src/features/ask-human/index.tsx frontend/src/features/ask-human/AskHuman.css frontend/src/lib/inboxApi.ts frontend/src/__tests__/features/ask-human/InboxRow.collapse.test.tsx
git commit -m "fix(inbox-ui): needs_human_audit 切片显 AI预审通过·待复核 held徽章(KB-08)

InboxRow 加可选 tag prop;renderItem 对 knowledge_review+needs_human_audit 传 held 色 pill;
AskHuman.css 加 .inboxTag--held(复用 --fill-held 不新造色);inboxApi.ts 类型加 integrityStatus。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: 全量验证 + lint + push + PR

**Files:** 无改动（纯验证 + 交付）

- [ ] **Step 1: 后端 baseline**

Run: `cargo test --lib 2>&1 | tail -10` → N ≥ 350 / 0 failed。（磁盘满则交 CI。）

- [ ] **Step 2: 前端 build + vitest**

Run: `cd frontend && npm run build 2>&1 | tail -8` → 0 error。
Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/ 2>&1 | tail -10` → 全 PASS。

- [ ] **Step 3: no-human-takeover lint**

Run: `git diff origin/main -- src/ frontend/src/ | grep -nE "人工接管|takeover|hand.?off|人工介入|人工托管|接管|人工" || echo "lint clean"`
Expected: `lint clean`。

- [ ] **Step 4: push + 开 PR**

```bash
git push -u origin fix/kb08-audit-inbox-blackhole
gh pr create --title "fix: auto_verify 降级切片重回人审收件箱 (KB-08)" --body "$(cat <<'EOF'
## Summary
修复深度审查批B [KB-08]（本批最有业务价值的 finding）：auto_verify 分诊出的 needs_human_audit 切片无任何审核入口查询 → 从收件箱消失、只剩计数，人审漏斗黑洞。

- **后端**：统一收件箱（ask_human_inbox.rs）抽 `knowledge_review_statuses()={needs_review,needs_human_audit}`，列表查询 + summary 计数共用（同源防漂移）；InboxItem 加 integrityStatus。AI digest 收件箱（digest_inbox.rs）pending_review 分支同步扩展 + 抽 pending_review_card_labels 区分 title/origin。
- **前端**：InboxRow 加 tag prop，needs_human_audit 显 held 色"AI预审通过·待复核"徽章。
- 写端 verify.rs 分诊逻辑不动（红线未破：切片仍不被自动 verified）。

## Test plan
- [x] cargo test --lib（2 后端纯函数单测：statuses 含 needs_human_audit / card_labels 区分；baseline ≥350 不回退）
- [x] frontend vitest（InboxRow tag 渲染）+ npm run build
- [x] no-human-takeover lint clean
- [ ] 端到端（needs_human_audit 切片出现在收件箱并可 verify 晋升）：CI integration + 后续浏览器验证

设计：docs/superpowers/specs/2026-07-12-kb08-audit-inbox-blackhole-design.md
台账：docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md KB-08

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review 结论
- **Spec coverage**：设计后端改动1（statuses 纯函数+查询/计数同源+InboxItem 字段）↔ Task1；后端改动2（digest 分支+card_labels 纯函数）↔ Task2；前端（InboxRow tag+renderItem+css+类型）↔ Task3；验证+PR ↔ Task4。全覆盖。
- **Placeholder scan**：无 TBD/TODO；每 Step 给完整代码+命令+预期。`$in` BSON 转换给了 3 种择一方案 + cargo check 兜底。
- **Type consistency**：`knowledge_review_statuses() -> [&'static str;2]` Task1 定义、Task1 Step5 与 Task2 均用一致语义；`pending_review_card_labels(&str,&str,bool)->(String,String)` Task2 定义与单测调用一致；前端 `tag?:{label,tone}` Task3 定义与 renderItem/vitest 调用一致；`integrityStatus?` 后端 `integrity_status` serde camelCase ↔ 前端类型一致。
- **TDD**：Task1/2 先写失败纯函数单测→实现→绿；Task3 先写失败 vitest→实现→绿；Task4 全量。
- **红线**：每个改码 Task Step1 先读懂+亲验行号；InboxItem 加字段靠 cargo check E0063 兜底补全所有构造点；写端 verify.rs 明确不动。
