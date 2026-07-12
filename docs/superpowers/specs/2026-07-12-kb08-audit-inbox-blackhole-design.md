# KB-08 修复设计：auto_verify 降级切片的人审黑洞

- 日期：2026-07-12
- 分支：`fix/kb08-audit-inbox-blackhole`（将从最新 origin/main 新开）
- 来源：深度审查批B [KB-08]（台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`）
- 优先级：P1（台账评"本批迄今最有业务价值的 finding"——状态机新增态与消费方脱节）
- 方案：后端查询扩展 + 区分 styled 徽章（用户裁定，保留 needs_human_audit 与 needs_review 的分诊语义区别）

## 问题（KB-08 根因，已主控逐条亲验最新 main 成立）

auto_verify 批处理对"过闸"切片强制写 `integrity_status="needs_human_audit"`（预审分诊：过闸的挑出来等运营重点看），但**没有任何审核入口查询这个状态**，切片从人审界面消失、只剩一个计数。

**已亲验的事实链**：
- **写端**（`src/routes/knowledge/verify.rs`）：
  - auto_verify 输入过滤 `integrity_status ∈ {needs_review, null}`（verify.rs:270）。
  - `decide_auto_verify_status`（verify.rs:534-539）**只在 `has_source_quote && has_source_anchor` 都真**（且 confidence≥threshold、model=verified）时返 `"verified"`。
  - `enforce_verified_needs_human_audit`（verify.rs:554-557）把所有 `"verified"` 强制降级 `"needs_human_audit"`，写回（verify.rs:426）。
  - **推论（关键）**：每个 `needs_human_audit` 切片必然同时有 source_quote + source_anchor。
- **读端黑洞**：
  - 统一收件箱 `collect_knowledge_review`（`src/routes/ask_human_inbox.rs:124`）只查 `needs_review`；其 summary 计数（ask_human_inbox.rs:520）同样只查 `needs_review`。
  - AI digest 收件箱 `knowledge_inbox`（`src/routes/knowledge/digest_inbox.rs`）三分支：pending_review（:431 要 `needs_review`）、quote_missing（:465 要缺 quote）、anchors_missing（:481 要缺 anchor）——`needs_human_audit` 非 needs_review 且必有 quote+anchor，三分支全不匹配。
  - 前端仅计数展示（`AutoVerifyPanel.tsx:170` / `quality/index.tsx:238`），无过滤视图。
- **晋升可行**：人审 `verify_operation_knowledge_chunk`（verify.rs:104）的 D2 gate 要求 quote+anchor——needs_human_audit 都满足，一旦可见即能被运营 verify 晋升。

**红线字面未破**（切片没被自动 verified、精确 `=="verified"` 召回背书已核），但人审漏斗有洞：needs_human_audit 无限期停留、无入口晋升。

## 设计

### 后端改动 1：统一收件箱 `ask_human_inbox.rs`

- **抽纯函数** `knowledge_review_statuses() -> [&'static str; 2]` 返回 `["needs_review", "needs_human_audit"]`。
- `collect_knowledge_review` 查询（:124）改为 `doc! { "workspace_id": ws, "integrity_status": { "$in": knowledge_review_statuses() } }`。
- `ask_human_summary` 知识计数（:520）**共用同一函数**同样 `$in`——保证列表与计数同源、不再漂移（KB-08 与 PR#177 同属"count/list drift"病根，此处一并杜绝）。
- `InboxItem` 结构体新增字段 `integrity_status: Option<String>`（`#[serde(skip_serializing_if = "Option::is_none")]`，序列化为 `integrityStatus`）；仅 `collect_knowledge_review` 填充（其余来源恒 None）。前端据它区分两态。

### 后端改动 2：AI digest 收件箱 `digest_inbox.rs`

- pending_review 分支（:431）条件从 `integrity == "needs_review"` 扩为 `["needs_review", "needs_human_audit"].contains(integrity.as_str())`（保持 `&& updated_ms >= cutoff_ms` 7d 窗口不变）。
- 抽纯函数 `pending_review_card_labels(integrity_status, base_title, is_negative_example) -> (title, origin)`：
  - `needs_human_audit` → title `"AI预审通过待复核：{base_title}"`、origin `"human_audit_pending"`。
  - `needs_review` + negative_example → 保持原 `"待审反例：…"` / origin `"negative_example_review"`。
  - `needs_review` 其它 → 保持原 `"待审切片：…"` / origin `"pending_review"`。
- 分支内改用该函数产出的 title/origin（现有 priority 逻辑 `inbox_pending_review_priority` 不变）。

### 前端改动：styled 徽章

- `InboxRow`（`ask-human/index.tsx:110`）新增可选 prop `tag?: { label: string; tone: string }`，在 `inboxRowTitle` 旁渲染一枚小 pill（`<span className="inboxTag inboxTag--{tone}">{label}</span>`），无 tag 时不渲染（保持现有单 badge 行为）。
- renderItem（:217）：当 `item.source === "knowledge_review" && item.integrityStatus === "needs_human_audit"` 时传 `tag={{ label: "AI预审通过·待复核", tone: "held" }}`。
- `AskHuman.css` 加 `.inboxTag` + `.inboxTag--held` 样式。**tone 用 `held`（复用现有 `--fill-held` 琥珀色，语义=等待/待复核）**——现有 inboxBadge tones 只有 `brand/scheduled/held/running/neutral/blocked`，不新造颜色；避开"主操作蓝/AI 身份紫"色纪律红线，held 琥珀恰表"预审通过、悬置待人复核"。
- `lib/inboxApi.ts` 的 `InboxItem` 类型加 `integrityStatus?: string`。

## 不改动的（严格限定范围）

- 写端 `verify.rs` 分诊逻辑（decide_auto_verify_status / enforce_verified_needs_human_audit 正确，红线未破）。
- 前端计数展示（AutoVerifyPanel.tsx / quality/index.tsx 保留）。
- 迁移 / 配置 / .env / API 契约形状（仅 InboxItem 加可选字段，前端可选消费，向后兼容）。

## 测试策略

- **后端纯函数进 lib 单测**（`cargo test --lib`，无需 Docker，进 baseline）：
  1. `knowledge_review_statuses` 必含 `needs_human_audit`（防回退成只查 needs_review——这是 KB-08 的病根，锚死它）。
  2. `pending_review_card_labels`：needs_human_audit → 区分 title + origin=human_audit_pending；needs_review 两分支保持原样。
- **前端 vitest**：InboxRow 传 tag 时渲染 pill、不传时不渲染；ask-human renderItem 对 needs_human_audit item 传对 tag。
- **集成/浏览器**：本地磁盘紧 + 无 Docker，端到端（needs_human_audit 切片出现在收件箱并可 verify 晋升）留 CI integration + 后续浏览器验证；前端 `npm run build` 本地跑（frontend 构建不占 Rust target）。

## 验证

- `cargo check` + `cargo test --lib`（baseline lib ≥ 350 / 0 failed 不回退）。
- 前端 `cd frontend && npm run build`（tsc + vite 编译通过）。
- no-human-takeover lint：新增行用「预审/复核/待审/审核」措辞，无禁词（`人工接管/takeover/hand-off/人工介入/人工托管/接管/人工`）。

## 交付

- 后端：`ask_human_inbox.rs`（纯函数 + 查询/计数共用 + InboxItem 字段）、`digest_inbox.rs`（分支扩展 + 纯函数）。
- 前端：`ask-human/index.tsx`（InboxRow tag prop + renderItem）、`AskHuman.css`（tag 样式）、`lib/inboxApi.ts`（类型）。
- 独立修复 PR（基于最新 main）。台账 KB-08 标 Closed。
