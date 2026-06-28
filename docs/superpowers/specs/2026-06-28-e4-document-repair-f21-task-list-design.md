# E4 文档级批量修复 + F21 任务总览列表 设计

- 日期：2026-06-28
- 来源：前后端业务对齐工程收尾。批次 1-4（PR#44/46/47/48）+ chunk 修复闭环（PR#49）落地后，对剩余开放条目做严格代码核实，确认实质待办仅 E4、F21 两条（C10 已由 C9/C6 用 agent-runs decision 闭环，其余 F 系列为 spec 标注的有意取舍/冗余）。
- 关联 memory：`project_knowledge_repair_dispatch_architecture`（本次深读产出，每条 file:line 实证）、`project_frontend_backend_alignment_audit_2026_06_26`（76→67 路线图）、`frontend_follow_design_system`、`feedback_no_overfitting`。
- 状态：设计稿，待用户审。

## 背景与问题

前后端对齐审查路线图（`docs/superpowers/specs/2026-06-26-frontend-backend-alignment-fixes-design.md`）中两条剩余条目：

- **E4（pack 级 AI 修复入口）**：spec 原描述为"后端 `mod.rs:631 items/:id/repair` + `repair/applied` 存在，前端无调用"。**深读后证伪了"后端存在"这一前提**：`propose_pack_repair`（`src/routes/knowledge/repair.rs:543-553`）是死桩，永远返回 `400 "operation_knowledge_items has been removed; pack repair temporarily disabled"`。它依赖的 `operation_knowledge_items` 集合已被 migration `m011_drop_legacy_sales_collections.rs:29` / `m014_drop_trigger_keywords.rs:24` 物理删除（属"清理遗留销售域集合"，不会回归）。给死桩补前端入口 = 造一个点了就报错的按钮，是错误做法。
- **F21（知识长任务跟踪/取消）**：派工创建（E14）+ 单任务跟踪 + 取消已在 PR#44/47 落地（`today.tsx` TaskRail）。唯一残留：后端无"列出全部任务"的 list 端点（`src/routes/mod.rs:673-682` 仅 create/`:id`/cancel），前端 TaskRail 只能按单个 taskId 拉取（手工粘贴 input + 监听 `wikiTrackTask` 自动跟踪）。

**用户决策**：E4 不保留死桩、连后端一起"重建"，但因 pack 实体已不存在，重建实为**重新定义这个能力修什么对象** → 选定 **文档级批量修复**（document 是当前唯一仍活跃写入的"一对多 chunk 容器"）。

## 关键代码事实（决定设计的约束，均经 file:line 核实）

1. **无活跃 pack/分组层**：知识库当前只有 `operation_knowledge_documents → operation_knowledge_chunks` 两层。chunk 仍有 `item_id` 字段（`models.rs:1333`）但写恒 None、读被显式丢弃（`repair.rs:238`）。唯一活跃的一对多容器是 `chunk.document_id`（`models.rs:1332`）。
2. **chunk 级修复闭环已成（PR#49）**：`propose_chunk_repair`（`repair.rs:201`）/ `answer_chunk_repair`（`repair.rs:380`）调 LLM 产 patch 不写库；前端 `ChunkRepairPanel.tsx` 逐字段勾选 → `applyAiRepairPatch.ts` 走 PUT 落库（draft+needs_review）+ POST `/repair/applied` 审计。红线 `thenVerify: false` 写死。
3. **`record_repair_apply`（`repair.rs:596`）已支持 chunk targetKind**：文档级批量在审计层就是 N 条独立的 chunk 级 `knowledge_repair_applied` 事件，无需新审计形态。
4. **现成端点 `GET /operation-knowledge/documents/:id/chunks`**（`crud.rs:173` `list_operation_knowledge_document_chunks`）：按 document_id 拉名下全部 chunk。**document 级批量的数据来源现成，零新增后端端点。**
5. **该端点的 query 只过滤 `status`（draft/active），不过滤 `integrity_status`（needs_review）**（`mod.rs:994`）：故 needs_review 的筛选放前端做（文档下 chunk 有 300 上限，前端筛轻量）。
6. **F21 后端确无 list 端点**：`chat.rs` 只有 `chat_task_create`（`:1868`）/ `chat_task_get`（`:2014`）/ `chat_task_cancel`（`:2052`）。`KnowledgeChatTask` 模型字段见 `models.rs:4576-4605`，`ALLOWED_TASK_STATUS=[pending,running,completed,failed,cancelled]`（`models.rs:4571`）。
7. **worker `execute_step` 是 Phase4 占位桩**（`knowledge_task/mod.rs:437`），6 种 action 全不真改 chunk。故本设计**不走 worker 自动批量落库路线**（那既碰"AI 不自动 verify"红线，又要重写整个占位桩），而是沿用"AI 产 patch、运营逐 chunk 勾选落库"的人在环内闭环。

## 架构与组件

### E4：文档级批量修复（视角聚合，复用单 chunk 闭环）

**交互形态**：document 详情/列表项新增"批量 AI 修复"入口 → 打开聚合视图 `DocumentRepairPanel` → 列出该 document 下所有 `integrityStatus==needs_review` 的 chunk → 每个 chunk 可展开，内嵌**复用现有 `ChunkRepairPanel`** 走单 chunk 的 propose/answer/逐字段勾选落库 → 落库后该 chunk 标记完成/移出待修列表 → 运营逐个处理完即闭环。

**为何不做"一次性批量产 patch 大表"**：那需新建 batch propose 端点 + budget×N 分页限流 + 新审计形态，且重复造 `ChunkRepairPanel` 已解决的逐字段勾选/多轮追问/防清空。视角聚合用最小改动达成"批量"体验，budget 天然按单 chunk 隔离（仍 token≤4000/LLM≤4）。

**后端**：零改动。复用 `GET /operation-knowledge/documents/:id/chunks`，前端筛 needs_review；逐 chunk 落库复用 `applyAiRepairPatch`（targetKind="chunk"）。

**前端**：
- 新增 `frontend/src/features/knowledge/DocumentRepairPanel.tsx`：props 接 documentId（+ document 标题等展示元数据）；mount 时 fetch `/api/operation-knowledge/documents/:id/chunks`，前端 filter `integrityStatus==="needs_review"`；渲染待修 chunk 列表，每项可展开内嵌 `ChunkRepairPanel`（复用其 `chunkId`/`originalChunk`/`onApplied`）；`onApplied` 回调刷新本地列表（把已落库 chunk 移出/标记）+ 透传 `wikiChunkRevised` 事件。
- 入口挂载：在 document 列表/详情（`steward.tsx` 或 document 项）加"批量 AI 修复"按钮，仅当该 document 有 needs_review chunk 时显示（或显示待修计数）。具体落点实现时按现有 document UI 结构定，遵守设计系统。
- `Knowledge.css` 追加 `DocumentRepairPanel` 所需 class（plain .css，紫 `--fill-brand` 仅用于 AI 身份元素）。

### F21：任务总览列表

**后端**：
- `chat.rs` 新增 `chat_task_list`（`GET /knowledge/chat/tasks`）：filter `workspace_id`（+ 可选 `status` query + `limit` clamp，默认按 `created_at` 倒序）；投影与 `chat_task_get` 对齐的精简列表项（taskId/sessionId/status/totalSteps/completedSteps 计数/createdAt/finishedAt，列表不必带完整 plannedSteps/cards 全文以控体积）。
- `mod.rs` 路由：现有 `.route("/knowledge/chat/tasks", post(chat_task_create))` 改为 `.get(chat_task_list).post(chat_task_create)`。

**前端**：
- `today.tsx` TaskRail：把"手工粘贴 taskId"单 input 升级为任务列表——mount/可见时 GET `/knowledge/chat/tasks` 拉列表，点选某任务 → 复用现有 `loadTask` 展示详情/进度/取消；保留手工输入框作 fallback（修正变量名 `sessionId`→实为 taskId 的命名瑕疵可顺手改，低优）。
- 列表项展示 status 中文标签（复用 `taskStatusLabel`）+ 步数进度 + 创建时间。

## 数据流

```
E4 文档级批量修复：
  document 入口（有 needs_review chunk）
    → DocumentRepairPanel mount
    → GET /documents/:id/chunks（现成端点）
    → 前端 filter integrityStatus==needs_review
    → 列表逐项展开 ChunkRepairPanel（复用 PR#49）
       → POST /chunks/:id/repair (propose, 调 LLM 产 patch)
       → POST /chunks/:id/repair/answer (多轮追问)
       → applyAiRepairPatch: PUT /chunks/:id (draft+needs_review, thenVerify=false)
                            + POST /repair/applied (targetKind=chunk, 审计)
    → onApplied 刷新列表，移出已落库 chunk

F21 任务总览：
  TaskRail mount → GET /knowledge/chat/tasks (新增 list)
    → 列表点选 → loadTask(taskId) → GET /chat/tasks/:id（现有）
    → 取消 → POST /chat/tasks/:id/cancel（现有）
```

## 错误处理

- `DocumentRepairPanel` fetch 失败：显示错误态（非静默空态，呼应审查反模式"错误态静默吞成空态"）。无 needs_review chunk 时显示"该文档无待修切片"正常空态。
- 单 chunk 修复失败不拖垮其余 chunk（各 `ChunkRepairPanel` 独立）。
- `chat_task_list` 空结果返回 `{items:[]}`；前端区分加载中/空/错误三态。
- 后端 list 端点 status query 非法值：不报错，忽略该过滤（与现有 chunk 列表 query 宽松风格一致）或按需 400；实现时与现有 query 处理保持一致。

## 测试

- **后端**（守 baseline ≥350/0）：`chat_task_list` 集成测试——多 task 落库后 list 返回、status 过滤、workspace 隔离、投影字段正确。
- **前端**（vitest）：
  - `DocumentRepairPanel`：mock `/documents/:id/chunks` 返回混合 integrityStatus → 断言只渲染 needs_review；展开渲染 ChunkRepairPanel 入口；空/错误态。
  - TaskRail 列表：mock `/knowledge/chat/tasks` → 断言列表渲染、点选触发 loadTask、保留手工输入。
- **回归**：禁词 lint（`check-no-human-takeover`）；tsc 0 error；前端 build + vitest 三连。

## 不变量（全程守住）

- AI 永不自动 verify：所有落库走 PUT（draft + needs_review），`thenVerify` 恒 false。本设计不引入任何 worker 自动落库路径。
- 新增前后端代码新行不含禁词（CI 门）。
- 前端新组件遵守设计系统（tokens.css / .module.css 或 knowledge 频道既有 plain .css / 4 级层级 / 紫仅 AI 身份）。
- 后端 list 端点加 query 过滤须向后兼容，不破现有 create/get/cancel 消费方。
- 不复活 `operation_knowledge_items`；不碰 chunk 级修复已闭环的 PR#49 代码（仅复用）。

## 任务拆分（SDD，每任务独立 subagent + review）

1. 后端 `chat_task_list` 端点 + `mod.rs` 路由（`.get` 挂上）+ 集成测试。
2. 前端 TaskRail 列表化（GET list + 点选 loadTask + 保留手工 fallback）+ vitest。
3. 前端 `DocumentRepairPanel`（聚合视图 + 内嵌复用 ChunkRepairPanel）+ document 入口挂载 + Knowledge.css + vitest。
4. whole-branch 终审（最强模型）：红线/禁词/baseline/复用正确性。

## 范围与 YAGNI

- 不做 worker `execute_step` 真实落库（占位桩保持，避免碰红线 + 大重写）。
- 不做"一次性批量产 patch 大表"（视角聚合已达批量体验，避免 budget×N 与重复造轮子）。
- 不复活 pack/items 集合。
- F21 list 端点只做列表投影，不附带分页基础设施（300/limit clamp 足够，YAGNI）。
- 不借机重构无关 document/steward UI；入口挂载聚焦最小改动。
