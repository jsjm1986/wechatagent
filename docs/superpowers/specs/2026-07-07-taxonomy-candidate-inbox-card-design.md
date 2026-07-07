# 统一收件箱「标签候选」卡片小白化改造设计

- 日期：2026-07-07
- 状态：设计待批
- 范围：后端 `src/routes/ask_human_inbox.rs` 数据构造 + 前端新增 rich 卡片组件 + `system-strategy` 复用重构

## 一、问题

统一收件箱（ask-human）的「标签候选」项对非技术运营完全不可用。运营看到的是一张零上下文的卡：

- 标题 `标签候选：emotional_state`——`emotional_state` 是裸维度键（英文 canonical id）。
- 正文 `anxious` / `sad`——AI 新造的裸候选值。
- 两个按钮「通过 / 拒绝」。

运营不知道这是什么、AI 为什么建议、通过后会发生什么。更严重的是，「通过」按钮**必然失败**：它走 `SimpleApproveReject`，approve 时发空 body `{}`，而后端 `approve_taxonomy_candidate` 要求 `canonicalValue`（含非空 `id` + `label`），空 body → 400。这条路从 UI 根本走不通。

### 三层缺陷（均已 file:line 亲验，基于 worktree HEAD c15bc26 = origin/main）

1. **展示层裸字段（`ask_human_inbox.rs:182-183`）**：`title = format!("标签候选：{}", c.kind)` 直渲维度键；`summary = c.raw_value` 直渲裸候选值。无任何中文框定、无「AI 为什么建议」「通过后会怎样」的说明。

2. **富字段被丢弃（`ask_human_inbox.rs:187, 194-196`）**：`collect_taxonomy_candidates` 把 `action_kind` 定为 `"inline"`，并把 `evidence` / `confidence` / `occurrences` 全硬编码成 `None`——而 `TaxonomyCandidate` 模型（`models.rs:2898-2921`）里这些字段**本就有值**（`evidence: Option<String>`、`confidence: i32`、`occurrences: i32`，另有 `suggested_display_name: Option<String>` 供预填中文名）。同文件的 `collect_relationship_suggestions`（`:220-239`）是正确参照——它把 `evidence` / `confidence` / `occurrences` 都接了出来。

3. **功能层根本走不通 + 已有重复 UI**：approve 需要 `canonicalValue`（`admin_taxonomy_candidates.rs:57-74, 123-131`，`ApproveCandidateRequest.canonical_value` 无 serde default = 必填），而 inline 的 `SimpleApproveReject` 只发空 body。同时 `system-strategy` 频道里**已存在**一个完整可用的 `TaxonomyCandidatesAdmin`（含命名表单：canonical id / 显示名 / 别名 / 描述 + 采纳/驳回 + 409 处理），运营真正需要的交互早已实现，只是收件箱没接上。

结论：标签候选不是「简单二元通过/拒绝」项——审核它 = 给 AI 新造的取值**命名并归档进字典**，需要一个带命名表单的 rich 卡，而非 inline 按钮。

## 二、目标

让收件箱的标签候选项：

1. 用小白能懂的中文框定「这是什么、AI 为什么这么建议、你通过后会发生什么」。
2. 展示 AI 的判断依据（evidence）、置信度、出现次数，让运营有据可依。
3. 提供与 `system-strategy` 一致的命名表单（canonical id / 显示名 / 别名 / 描述），一步完成「采纳进字典」或「驳回」。
4. 采纳/驳回真实走通后端（不再 400），完成后刷新收件箱。

## 三、方案（已选：收件箱内嵌 rich 命名卡）

把标签候选从 inline 二元项**重新归类为 rich 卡**，复用 `system-strategy` 已验证的命名表单逻辑，抽成共享组件。

### 3.1 后端 `collect_taxonomy_candidates`（`ask_human_inbox.rs:160-202`）

改动点（仅数据构造，不动任何写路径）：

- `action_kind`：`"inline"` → `"rich"`。
- `rich_component`：`None` → `Some("taxonomyCandidateReview")`。
- `rich_params`：`None` → 携带前端渲染卡所需的全部数据（数据此刻已在手，**无需新增 get-by-id 端点**）：
  ```
  {
    "candidateId": <hex id>,
    "scope":       c.scope,
    "kind":        c.kind,
    "rawValue":    c.raw_value,
    "evidence":    c.evidence,               // Option → 缺省不放
    "confidence":  c.confidence,             // i32
    "occurrences": c.occurrences,            // i32
    "suggestedDisplayName": c.suggested_display_name  // Option → 缺省不放
  }
  ```
- `title`：改为人话——`format!("AI 新识别标签：{}", c.raw_value)`（前面加维度中文名的工作放前端做，见 3.4；后端标题至少不再暴露裸维度键作主语）。
- `summary`：`c.evidence` 有值时用它，否则给一句通用框定文案（如「AI 从对话里识别到一个尚未收录的取值，请确认是否纳入标签字典」）。
- `evidence` / `confidence` / `occurrences` 顶层字段：与 relationship_suggestion 对称地接出来（`evidence: c.evidence.clone()`、`confidence: Some(c.confidence)`、`occurrences: Some(c.occurrences)`）。成本极低，且列表折叠预览 `summary` 与卡内证据口径一致。

`rich_params` 是 `bson::Document`，`Option` 字段用 `if let Some` 条件插入，避免写入 null 键。

### 3.2 前端新增共享组件 `TaxonomyCandidateReviewCard`

位置：`frontend/src/components/review/TaxonomyCandidateReviewCard.tsx`（与其它 rich 卡 `ChunkReviewCard` / `ProfilePublishCard` 等同目录，架构一致）。

Props（纯 props，组件内不自行 fetch 候选——数据由调用方从 `rich_params` 或 `system-strategy` 列表传入）：
```ts
interface TaxonomyCandidateReviewCardProps {
  candidate: {
    id: string;
    scope: string;
    kind: string;
    rawValue: string;
    evidence?: string;
    confidence?: number;
    occurrences?: number;
    suggestedDisplayName?: string;
  };
  onDone: () => void;   // 采纳/驳回成功后回调（收件箱→refreshAll，system-strategy→reload 列表）
}
```

卡内容：

1. **小白框定区**（顶部说明文案）：解释「AI 在和客户对话时识别到一个『{维度中文名}』维度上尚未收录的取值：`{rawValue}`。通过后它会作为一个正式标签存入字典，今后 AI 可稳定使用；驳回则丢弃这次建议。」维度中文名由前端维度键→中文映射给出（见 3.4）。
2. **证据区**：判断依据 `evidence`、置信度 `confidence`、出现次数 `occurrences`（复用现有 `evidenceMetrics.ts` 的展示口径，与 relationship_suggestion 卡一致）。
3. **命名表单**：canonical id（预填 `rawValue`）、显示名（预填 `suggestedDisplayName || rawValue`）、别名（逗号分隔）、描述（预填 `evidence`）。逻辑照搬 `system-strategy` 的 `openApprove` / `submitApprove`（`canonicalValue: {id, label, aliases, description}`，id/label 非空校验，409 冲突提示）。
4. **采纳 / 驳回按钮**：采纳 POST `/api/admin/taxonomy-candidates/:id/approve`；驳回 POST `.../reject`（带 `reason`）。成功后调 `onDone()`。

组件使用 CSS module（与 review 目录其它卡一致），不复用 `system-strategy` 的样式表——接受少量 CSS 适配工作，换取 ask-human 不耦合 system-strategy 的样式。

### 3.3 前端 ask-human `renderRich` 接线（`ask-human/index.tsx:42-56`）

新增分支：
```tsx
case "taxonomyCandidateReview": {
  const p = item.richParams ?? {};
  return (
    <TaxonomyCandidateReviewCard
      candidate={{
        id: String(p.candidateId),
        scope: String(p.scope ?? ""),
        kind: String(p.kind ?? ""),
        rawValue: String(p.rawValue ?? ""),
        evidence: p.evidence != null ? String(p.evidence) : undefined,
        confidence: p.confidence != null ? Number(p.confidence) : undefined,
        occurrences: p.occurrences != null ? Number(p.occurrences) : undefined,
        suggestedDisplayName: p.suggestedDisplayName != null ? String(p.suggestedDisplayName) : undefined,
      }}
      onDone={onDone}
    />
  );
}
```
同时从 `renderInline` 删除 `taxonomy_candidate` 分支（`:63-73`）——它不再走 inline。`SimpleApproveReject` 的其它使用者（relationship_suggestion / gap_signal）不受影响。

### 3.4 维度键 → 中文名映射

`kind`（如 `emotional_state` / `customer_stage`）需要中文展示名。先查前端是否已有维度键中文字典（如 `knowledge/labels.ts` 或 `reviewLabels.ts`）；有则复用，无则在组件内用一个小映射兜底（缺失键回落显示原 `kind`，不硬失败）。**实现阶段确认，不在设计里臆断字典路径**。

### 3.5 `system-strategy` 复用重构（DRY）

`TaxonomyCandidatesAdmin`（`system-strategy/index.tsx`）现有内联的 approve 表单 + 采纳/驳回逻辑，与新组件重复。重构：让 `TaxonomyCandidatesAdmin` 渲染新的 `TaxonomyCandidateReviewCard`（每个 pending 候选一张卡），删除其内联表单副本。保留 `system-strategy` 特有的外层（状态筛选 tab、列表容器）。目标：命名表单逻辑只有一份。

若重构 `system-strategy` 牵动过大（其表单与列表状态耦合较深），**降级方案**：新组件先只服务 ask-human，`system-strategy` 暂不动，作为已知重复记入后续清理项——不阻塞收件箱修复。

### 3.6 关键决策记录

- **顶层 evidence/confidence/occurrences 接出**：接（与 relationship_suggestion 对称，列表折叠预览 `summary` 可用），成本极低。
- **组件 CSS 归属**：review 目录自带 CSS module，不耦合 system-strategy 样式。
- **system-strategy 同步重构复用**：优先做（DRY）；若耦合过深则降级为「新组件先服务 ask-human + 记后续清理」，不阻塞收件箱修复。
- **rich_params 携带全量数据**：不新增 get-by-id 端点，候选数据聚合时已在手，一次投影带全。

## 四、错误处理

- 后端 `rich_params` 构造：`Option` 字段缺省不写键，不产生 null。
- 前端 approve：id/label 空 → 前端拦截给提示（不发请求）；后端 409（canonical id 重复）→ 卡内提示"该标签已存在"。
- approve/reject 网络失败 → 卡内错误提示，不静默吞空。
- `renderRich` 未知 `richComponent` 已有兜底分支（`:53-54`），无需改。

## 五、测试

- **后端**：`collect_taxonomy_candidates` 投影单测——给定一条含 evidence/confidence/occurrences/suggested_display_name 的 `TaxonomyCandidate`，断言产出 `action_kind=="rich"`、`rich_component==Some("taxonomyCandidateReview")`、`rich_params` 含全部键且值正确、`title` 不含裸维度键作主语。（参照同文件请示卡已有的具名单测模式。）
- **前端**：`TaxonomyCandidateReviewCard` 渲染 + 提交测试——渲染断言显示 rawValue/证据/命名表单预填；模拟 approve 断言 POST body 为 `canonicalValue:{id,label,...}`；模拟 409 断言提示。
- **回归**：`system-strategy` 的 `taxonomyFlags.test.tsx`（已存在）在重构后不回归。
- **三闸全绿**：`cargo test --lib`（≥350/0 基线不回归）、`npx vitest run`、`bash scripts/check-no-human-takeover.sh`（新增文案无禁用词）。

## 六、红线合规

- **AI 全自治红线**：本卡是"运营给 AI 新造标签命名归档"，不是人工接管客户对话。文案措辞用「确认纳入标签字典」「AI 今后可稳定使用」，不出现「人工接管/介入/托管/转人工」等禁用词；check-no-human-takeover lint 会在 CI 拦截。
- **AI 不自动核验红线**：候选进字典仍需运营点击采纳（保持"AI 提议 + 人工确认"闭环），本改动不改这一点，只是把确认动作从"走不通的空按钮"换成"可用的命名卡"。
- **零侵入写路径**：后端只改 `collect_taxonomy_candidates` 的只读投影，不动 approve/reject 的写逻辑与校验。

## 七、不做（YAGNI）

- 不新增 get-candidate-by-id 端点（`rich_params` 已携带全部数据）。
- 不改 approve/reject 后端契约（`canonicalValue` 必填保持不变——它是正确的业务约束）。
- 不动 `SimpleApproveReject` 组件本身（relationship_suggestion / gap_signal 继续用）。
- 不处理收件箱其它来源的问题（用户提过"还有很多问题"，另开工单）。
