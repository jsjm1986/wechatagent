# F22 + F12-provenance:chunk AI 修复落库闭环 设计

- 日期：2026-06-27
- 来源：前后端业务对齐 76→67 路线图收尾后，用「当前 main 视角」复核被剔除的 LOW/INFO 尾项（2 个 subagent 实证）。F22 当初剔除理由「依赖不存在的 applyAiRepairPatch，价值低」**已过期**：后端 chunk 修复三端点（propose/answer/applied）当前**全部 alive 且完全空转**（前端 `/repair` 0 命中），是已落地资产；F12-provenance（chunk 主表 provenance 字段前端只定型不渲染）顺带一并补。
- 关联 memory：`project_frontend_backend_alignment_audit_2026_06_26`、`frontend_follow_design_system`、`feedback_no_overfitting`、`project_real_business_flow_focus`。
- 状态：设计稿，待用户审。

## 一、目标与性质

给 `integrity_status = needs_review` 的知识切片接上 AI 修复闭环：运营在 chunk 详情面板点「AI 修复建议」→ AI 提议 patch（多轮可追问）→ 运营逐字段勾选 → 落库为 draft → 闭账上报。顺带渲染 chunk provenance（来源溯源）。

**性质：纯前端**。后端三端点（propose/answer/applied）+ 落库 PUT + verify 全部现成，**零后端改动、零 migration、零集成测**。本设计是把后端已落地却空转的能力在前端兑现。

**不做**：pack 维度修复（E4，后端 `propose_pack_repair` 已下线返回 400，items 集合已删，无对接对象）。

## 二、后端契约（已定，前端遵守，不改）

`src/routes/knowledge/repair.rs` 头部注释明示铁律：**AI 永远只输出 patch，不写库**；落库走现有 `PUT /chunks/:id`（+ 可选 `/verify`）；前端落库成功后再 POST `/repair/applied` 闭账。

| 端点 | 路由 | 输入 | 输出 |
|---|---|---|---|
| propose | `POST /api/operation-knowledge/chunks/:id/repair`（mod.rs:492） | 无 body（仅 path id） | `{ sessionId, interpretation, patch, missingFields:[{field,reason}], followupQuestions, stillMissing, confidenceHint }` |
| answer | `POST /api/operation-knowledge/chunks/:id/repair/answer`（mod.rs:496） | `ChunkRepairAnswerBody`（camelCase）：`sessionId` / `previousPatch` / `answers:[{id,field,text}]` / `turn` | 同 propose 形态（`parse_repair_response`，repair.rs:77-154） |
| applied | `POST /api/operation-knowledge/repair/applied`（mod.rs:643） | `RepairApplyBody`（camelCase，repair.rs:53-75）：`targetKind`("chunk") / `targetId` / `sessionId` / `turn` / `acceptedFields` / `skippedFields` / `confidenceHint` / `extras` / `thenVerify` | 闭账 AgentEvent `kind=knowledge_repair_applied`，不写知识库 |
| 落库（现有） | `PUT /api/operation-knowledge/chunks/:id`（mod.rs:474） | chunk 字段（camelCase） | 落 `status=draft + integrity_status=needs_review`（红线，后端强制） |
| verify（现有，本设计不调） | `POST /api/operation-knowledge/chunks/:id/verify`（shared.tsx:876 已有按钮） | — | 转 verified |

**budget**：propose/answer 各开独立 RUN_BUDGET（单轮 token ≤ 4000，LLM ≤ 4）。超预算返回 `BudgetExceeded`（200 + 字段，非 5xx）——前端识别后显友好提示。

## 三、边界决策（用户拍板）

1. **多轮**：propose + answer 都做（followupQuestions 非空时运营可回答，answer 产修订版 patch）。
2. **逐字段勾选落库**：对齐后端 `acceptedFields`/`skippedFields` 设计。patch 每字段一个复选框，运营可只接受部分字段（如要 summary 不要 AI 脑补的 sourceQuote）。
3. **不提供一键 verify**：落库只到 draft+needs_review，`thenVerify` 恒传 false。运营要放行另走现有 verify 按钮——修复与核验两步分离，守「AI 永不自动 verify」红线。
4. **顺带渲染 provenance**：ChunkInspector 加来源展示区。

## 四、组件与数据流

**新增 2 文件**：
- `frontend/src/lib/applyAiRepairPatch.ts` — 落库工具：接收（chunkId / 原 chunk / patch / 勾选字段名集 / sessionId / turn / confidenceHint / extras）→ 拼 `PUT /chunks/:id` body（**从原 chunk 值出发，只用勾选字段覆盖**，防清空）→ 成功后 `POST /repair/applied`（acceptedFields=勾选 / skippedFields=patch 有但没勾 / thenVerify=false）。抽独立工具便于单测拼 body。
- `frontend/src/features/knowledge/ChunkRepairPanel.tsx` — 修复面板组件 + 状态机。

**改动 2 文件**：
- `frontend/src/features/knowledge/shared.tsx`（ChunkInspectorPane，:162-340 区）— needs_review chunk 加「AI 修复建议」入口 + 挂 `<ChunkRepairPanel>`；加 provenance 展示区块（读 `chunk.provenance` 五字段）。
- `frontend/src/features/knowledge/trustTypes.ts` — **新建** propose/answer 返回结构 TS 类型（已实证：前端无 repair 提案类型；today.tsx 的 `missingFields: string[]` 是对话工坊 ChatTurnView 的字段，形态不同——repair 返回的是 `missingFields: [{field, reason}]`，不可复用，须新建 `ChunkRepairProposal` 等类型）。

**面板状态机**：
```
idle ─点"AI修复建议"→ proposing ─propose返回→ reviewing
reviewing（展示 interpretation / patch逐字段+复选框 / missingFields / confidenceHint / followupQuestions）
reviewing ─[有followup且运营填答]→ answering ─answer返回→ reviewing（刷新patch）
reviewing ─点"落库勾选字段"→ applying（PUT + repair/applied）─成功→ done（提示"已落库为草稿，可去核验"+刷新chunk）
任何阶段失败 → error（横幅可重试；BudgetExceeded 显"AI修复预算用尽，请稍后重试"）
```

**数据流关键**：
- `sessionId`（propose 返回）→ answer 与 applied 都带回，串审计链。
- answer 的 `previousPatch` = 上轮 patch，`answers:[{id,field,text}]` 对应 followupQuestions 的项。
- 落库 `acceptedFields` = 勾选字段名；`skippedFields` = patch 有但未勾；`extras` = patch.extras（schema 无容器的领域建议，透传闭账审计）。

## 五、错误处理

- `BudgetExceeded`（propose/answer，200+字段）→ 显"AI 修复预算用尽，请稍后重试"，不崩。
- PUT 落库失败 → **不**发 `/repair/applied`（闭账只在落库成功后），错误横幅，勾选状态保留可重试。
- `/repair/applied` 失败 → patch **已落库成功**，提示"已落库，审计记录写入失败"，**不回滚落库**（知识已正确写入，审计缺失次要，对齐项目「送达后 DB 失败降级审计绝不返 Err」既有原则）。
- followupQuestions 为空 → 不显追问区，直接进字段勾选。

## 六、红线与全局约束

- **AI 永不自动 verify**：落库 PUT 强制 `status=draft + integrity_status=needs_review`（后端保证），面板不传 verified，`thenVerify` 恒 false。
- **无人工接管 lint**：面板文案/注释避禁词（`人工/接管/takeover/hand-off/转人工/人工介入/人工托管`）。用业务语义："AI 修复建议"/"落库为草稿"/"去核验"。
- **设计系统**：knowledge 频道，复用 wiki* class（plain css 非 module，避 tree-shake 坑）；蓝（主操作）仅"落库"按钮，AI 提议区可用紫系标 AI 身份，其余中性。
- **防清空**：PUT body 从 chunk 原值出发只覆盖勾选字段（复用批次3 E6 防清空模式，避免"表单驱动整 body 清空 rawContent"反模式）。
- **serde 命门**：前端提交 wire 键 camelCase 对齐 repair.rs 的 `rename_all`：`sessionId`/`previousPatch`/`answers`/`targetKind`/`targetId`/`acceptedFields`/`skippedFields`/`confidenceHint`/`thenVerify`。
- **git**：只 `git add` 具名文件，绝不 `git add -A`（工作区有并行会话未提交改动）；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **测试基线不回退**：后端无改动，`cargo test --lib` 不回退；前端 vitest 全绿 + tsc 0。本地只跑前端测。

## 七、测试策略（纯前端 vitest）

- `applyAiRepairPatch.test.ts`（核心命门）：勾选 2/4 字段 → 断言 PUT body 只含勾选字段 + 原 chunk 其余字段保留（防清空）；`/repair/applied` 的 acceptedFields/skippedFields 正确分组；落库失败不发 applied；applied 失败不误报"落库失败"。
- `ChunkRepairPanel.test.tsx`：状态机——propose 渲染 patch 逐字段复选；有 followup 走 answer 多轮刷新 patch；BudgetExceeded 显友好提示；落库后 done 态。
- provenance：ChunkInspector 测断言五字段渲染 + provenance 为 null 时不崩。
- wire 键 camelCase 对齐 repair.rs（serde 命门，测试锁 `not.toHaveProperty` snake 形态）。

## 八、不变量

- 落库恒 draft+needs_review，AI 永不自动 verify。
- PUT body 防清空（从原值出发只覆盖勾选字段）。
- sessionId 贯穿 propose→answer→applied 审计链。
- 闭账失败不回滚落库（审计次要于知识正确写入）。
- wire 键 camelCase 对齐后端 rename_all。
- 零后端改动（pack 维度 E4 不碰，已下线）。

## 九、工作量

约 5 task：①applyAiRepairPatch 工具 + 单测 ②ChunkRepairPanel propose + reviewing UI ③answer 多轮追问 ④逐字段勾选落库 + 闭账 ⑤provenance 展示 + 接入 ChunkInspectorPane。纯前端，无后端/migration/集成测。
