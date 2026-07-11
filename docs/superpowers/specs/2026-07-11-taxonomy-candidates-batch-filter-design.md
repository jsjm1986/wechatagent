# F-007 设计：标签候选批量驳回 + 按类型筛选

- 日期：2026-07-11
- 台账来源：`docs/superpowers/specs/2026-07-10-full-system-test-findings.md` → [F-007]（Low / UX）
- 批次：全量系统测试台账修复 batch5

## 问题

系统策略 → taxonomy tab → 候选区（`TaxonomyCandidatesAdmin`）在候选量大时（实测 256 条 `taxonomy_candidates`）只能按 `PANEL_PAGE_SIZE=20` 逐页翻 13 页，既无批量操作、也无法按维度（kind）筛选。管理员要清理堆积的候选时逐条操作，效率低。

## 代码现状（已亲验）

- 前端组件：`frontend/src/features/system-strategy/index.tsx` 的 `TaxonomyCandidatesAdmin`（约 :1031）。
  - 已有 **status 筛选**（`CANDIDATE_STATUS_FILTERS = pending/approved/rejected/all`），通过 `?status=` 传给后端。
  - 分页是**纯前端** `usePagedList`（后端一次返回，前端 slice）。
  - 审核**逐条**：每个 pending 项渲染一个 `TaxonomyCandidateReviewCard`。
- 后端端点：`src/routes/admin_taxonomy_candidates.rs`
  - `GET /api/admin/taxonomy-candidates?status=&scope=&kind=` —— **已支持 `kind` 过滤**（:101-103），前端目前未传 `kind`。limit=500。
  - `POST /api/admin/taxonomy-candidates/:id/approve` —— body 需 `canonicalValue { id, label, aliases?, description? }`，把新词并入 `system_taxonomies`。**每条 id/label 不同，需人工填写**。
  - `POST /api/admin/taxonomy-candidates/:id/reject` —— body 仅需 `{ reason }`（非空）。幂等：只匹配 `status:"pending"`。
  - **无批量端点**。

## 关键取舍（澄清已定）

1. **批量范围 = 只做批量驳回**。批量采纳与"逐条填 canonicalValue（英文标识 + 显示名并入字典）"天然冲突——批量无法自动生成每条不同的标识，强行默认填充会往字典塞中文 id 污染质量。故本批不做批量采纳，采纳仍走单条 `TaxonomyCandidateReviewCard`。
2. **批量驳回实现 = 前端循环调单条 `:id/reject`**。零后端改动，复用现有审计（`reviewed_by`/`rejection_reason`）与幂等语义。
3. **kind 筛选 = 前端加 kind 下拉，服务端过滤**（传 `?kind=`）。后端已支持。
4. **选择交互 = 复选框 + 确认弹窗**。驳回不可逆丢弃候选，属危险动作，须二次确认（与台账危险动作确认家族 F-004/F-018 一致）。

## 设计

改动**全部在前端** `TaxonomyCandidatesAdmin` 单组件，零后端改动。

### 1. 按类型（kind）筛选

- 新增 state `kindFilter: string`（空串 = 全部）。
- 在现有 status tab 行旁加 kind 下拉：选项 = `全部` + 6 个已知 kind（复用 `TAXONOMY_KIND_LABELS`：客户阶段/意向强度/异议类型/顾虑类型/情绪状态/关系类型），option value 为 kind key。
- `reload()` 拼 query：`?status=${statusFilter}` + 非空时追加 `&kind=${encodeURIComponent(kindFilter)}`。
- `useEffect` 依赖数组加入 `kindFilter`，切换即重拉。
- 服务端过滤后列表总量变小，翻页压力自然缓解。

### 2. 批量驳回（复选框 + 确认弹窗）

仅当 `statusFilter === "pending"` 时启用（其余为终态，无批量意义）。

- **选择态**：新增 `selectedIds: Set<string>`。
  - pending 视图下每条候选头部加复选框（勾选/取消维护 `selectedIds`）。
  - 顶部加工具区：`全选本页 / 清空` + `已选 N 条` 计数 + 「批量驳回」按钮（`selectedIds.size === 0` 时 disabled）。
  - **一致性**：切换 status/kind filter 或翻页时清空 `selectedIds`，避免选中不可见条目的幽灵选择。
- **确认弹窗**：点「批量驳回」弹确认弹窗（复用系统现有确认组件/模式）。
  - 内含"驳回原因"输入框（共用，非空校验，与单条 reject 语义一致）。
  - 文案："将驳回选中的 N 条候选，操作不可撤销。"
- **执行**：确认后前端循环 `await api.post('/api/admin/taxonomy-candidates/:id/reject', { reason })` 逐条调用。
  - 失败不中断其余（幂等：非 pending 的匹配失败直接跳过）。
  - 汇总结果（成功 X 条 / 失败 Y 条）落到组件的 info/error 展示区。
  - 完成后清空 `selectedIds` + `reload()`。

### 3. 数据流

```
用户勾选 pending 候选 → selectedIds
  → 点「批量驳回」→ 确认弹窗（输入 reason）
    → 确认 → for id of selectedIds: POST :id/reject { reason }
      → 汇总成功/失败 → 清空 selectedIds → reload()
切换 status/kind filter 或翻页 → 清空 selectedIds
```

### 4. 错误处理

- kind 下拉切换重拉沿用现有 `reload()` 的 try/catch → `setError`。
- 批量驳回逐条失败不抛断整体：累计失败数，末尾汇总提示；单条失败不阻塞后续条目。
- reason 空 → 确认按钮 disabled（前端校验，与后端 `reason 不能为空` 双保险）。

## 测试（增量叠加，不改旧断言）

`frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`：
- kind 下拉切换 → 断言 reload 请求 URL 带 `kind=` 参数。
- 勾选 2 条 pending + 点批量驳回 → 断言弹确认窗；填原因确认 → 断言发出 2 次 reject POST（带 reason）+ reload。
- 批量驳回按钮在未选时 disabled。
- 非 pending filter（如 approved）下不渲染复选框、不出现批量驳回入口。
- 切换 filter 后 selectedIds 清空（批量按钮回到 disabled）。

## 不做（YAGNI + 红线）

- **不做批量采纳** —— 字典质量红线（采纳需逐条人工填英文标识）。
- **不加后端批量端点** —— 前端循环足够，零后端改动面。
- **不改分页机制** —— kind 服务端过滤后列表量已可控。
- **不动 approved/rejected 视图交互** —— 只在 pending 视图加批量能力。

## 验收

- `cargo test --lib`（后端零改动，仅确认无回归）+ 前端 `vitest run` + `tsc --noEmit` 全绿。
- 前端契约对账门（tsc + vitest）+ Baseline gate + Integration tests CI 全绿。
- no-human-takeover lint 无禁词（新增文案走 AI 自主语义）。

## 关联

- 台账 [F-007]；危险动作确认家族 [F-004]/[F-018]（同一确认模式）。
- 前一批 batch4（PR #172）已合并 origin/main 20814fe；batch5 从此基点开分支。
