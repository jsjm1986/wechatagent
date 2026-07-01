# wiki 知识库三项 P0 修复设计（红线口子 / 类型透传 / 派工产草稿）

> 日期：2026-07-01
> 类型：后端为主（Rust/Axum）+ 抽取 prompt 调整；不碰前端组件逻辑（③仅复用现有 task turn 展示）
> 前置：本设计基于对整个 wiki 子系统（后端 ~21k 行 + 前端 ~12k 行）的 100% 逐行通读 + 主会话对三条修复路径全部相关代码/调用链/数据流的亲验（每条结论带 file:line）。

## 0. 背景与三条 P0 的由来

对整个 wiki 子系统做了全覆盖审查后，定位到三条 P0 级不足。审查中**纠正了初步报告（subagent）的多处夸大/误判**，最终范围以主会话亲验为准：

| 项 | 一句话 | 初判 → 亲验后的修正 |
| --- | --- | --- |
| ① 红线口子 | auto-verify 可让非产品类 chunk 在无人工把关下变 `integrity_status=verified`；冷启动 peer_case 推送只看 `status` 不看 `integrity_status` | 初判"一个口子"，亲验发现**两个独立松动点**（①-a auto-verify + ①-b 冷启动） |
| ② 类型透传 | 导入抽取的 chunk 一律落 `wiki_type=None + chunk_type=product_fact`，LLM 判定的类型无路径落库 | 初判"safeClaims 等富字段全线丢弃"→ **半误报**：那些字段是 2026-05-25 **有意删除**的死字段，grounding 已转 verified-chunk 语义闸，**不该加回**；真缺环只有**类型维度** |
| ③ 派工产草稿 | 派工 `fix_chunk`/`add_chunk` 只回一句文案，不产出任何可审草稿 | 初判"派工闭环名存实亡/execute_step 空壳是缺陷"→ **定性反转**：worker 不自动改库是**守红线的正确设计**，后端 summary 语义诚实；真缺失是 fix/add **不产可审草稿** |

**贯穿全设计的红线（CLAUDE.md 硬规则）**：AI 永不自动 verify 知识。三条修复都必须强化而非削弱它——①直接堵住绕过它的两个口子；②③新增/改动的任何写路径，chunk 一律 `status=draft` + `integrity_status=needs_review`，最终 verified 必经人工。

## 1. 已亲验的关键事实（设计地基，全部带 file:line）

### 1.1 双字段语义（正交双门）
- `integrity_status`（内容可信度门）：needs_review / verified / rejected / needs_human_audit。红线"AI 永不自动 verify"针对它。
- `status`（生命周期/启用门）：draft / active / archived / applied / discarded。
- **对客消费需双门都过**（用户已确认原则"对客内容必须 verified"）：
  - 检索注入客户对话：`status="active"` AND `integrity_status="verified"`（`knowledge_router.rs:70-71`）
  - 冷启动 peer_case 推送：**只看** `status ∈ {active, approved}`，**不看 integrity_status**（`cold_contact_worker.rs:331`）← ①-b 松动点

### 1.2 auto-verify 的能力边界（`verify.rs`）
- `decide_auto_verify_status`（:521-539）：source_quote ∧ source_anchor ∧ confidence≥threshold ∧ 模型自称 verified → verified；模型明确 rejected → rejected；其余 needs_review。
- 抽样降级（:386-388）：命中 sample_rate（硬下限 5%）→ needs_human_audit。
- `enforce_product_claim_human_audit`（:553-558）：**仅** product_fact 的 verified 强制降 needs_human_audit。← ①-a 松动点：其余三类不受此拦。
- auto-verify 写库走 `apply_chunk_revision` + `source=ProvenanceSource::Rule`（:418），**只 patch `integrity_status`，不碰 `status`**（:419-424）。

### 1.3 AI-draft 降级只认 Ai 不认 Rule（`chunk_revisions.rs:209`）
`if matches!(req.source, ProvenanceSource::Ai) { status=draft; integrity_status=needs_review }`。auto-verify 用 `Rule` → 不触发降级 → 这是 ①-a 口子成立的另一半。

### 1.4 类型字段现状
- `OperationKnowledgeChunk` model 有 `wiki_type: Option<String>`（`models.rs:1460`）、`chunk_type: String`（:1510，default `product_fact`）。
- 请求体 `OperationKnowledgeChunkRequest`（`mod.rs:169-212`）**没有** wiki_type/chunk_type 字段 → 导入/PUT 无路径设定。
- 转换函数 `operation_knowledge_chunk_from_request`（:463-509）末尾 `..Default::default()`（:508）→ wiki_type=None、chunk_type=product_fact。
- 类型稳定设计：`wiki_type` 在 `DEFAULT_LOCKED_FIELDS`（`page_merge.rs:35-43`，注释"类型永不变"）**且**在 `preserve_unmodeled_chunk_fields`（`mod.rs:532`）；`chunk_type` 仅在 preserve、**不在** locked_fields（既有小不对称）。改类型的设计出口 `Reclassify` 结构化提案（`structural_proposals.rs:31`）**无 apply worker，是死水**。

### 1.5 已删死字段（②不加回的依据）
`guards.rs:3-6` + `gates.rs:649-652` 注释：`chunk.safe_claims / forbidden_claims / evidence_items / routing_card / ProductClaimMarkers` **已于 2026-05-25 从 model 删除**，grounding 切换为 wiki + 3 闸（knowledge_grounding / hallucination / run_budget）。R5.4 grounding 闸（`gates.rs:634-686`）现在靠 `compute_verified_chunks`（读 chunk 的 verified 存在性），**不读 safe_claims**。这些字段在请求体里是没清干净的遗留死字段，在抽取 prompt 里是没同步精简的遗留指令。

### 1.6 派工执行现状（③依据）
- `execute_step`（`knowledge_task/mod.rs:437-486`）：6 个 action 全是文案桩，`fix_chunk`（:445-457）只回"请运营在编辑器审核"，**不产 patch**。注释明说这是"Phase 4 占位：worker 仅编排、真正 apply 走 chat_apply"。
- worker 收尾（:326-372）：task 标 `completed`，但 summary 明说"待运营审核 chunk N 个"、details 带 `needsReviewChunkIds`（:384）→ **后端语义诚实，不谎称已改好**。
- `propose_chunk_repair`（`repair.rs:201-378`）：axum handler，核心 = load prompt → `generate_agent_json`（:303）→ `parse_repair_response` → 返回 patch/missingFields JSON，**只产提案 + 写审计日志，不改 chunk**。依赖 `admin.current_workspace` 仅取 workspace 字符串，无真 auth 依赖 → 可抽 inner。

## 2. ① 红线口子（彻底修：①-a + ①-b 一起堵）

**目标**：让"任何影响发给客户内容的 chunk 都必须经人工 verified"成为硬防线，不再靠"某类不进某链路"的脆弱假设。

### 2.1 ①-a：auto-verify 对所有 chunk_type 都不自动 verified

**改动点**：`src/routes/knowledge/verify.rs`
- `enforce_product_claim_human_audit`（:553-558）从"只拦 product_fact"扩为"拦所有类型"。语义变为：只要 `final_status == "verified"`，一律降级 `needs_human_audit`（不再判 chunk_type）。
- 函数重命名为 `enforce_verified_needs_human_audit`（去掉"product_claim"语义，因为它现在拦所有类型），保留 `chunk_type` 参数位或删除——**删除参数**更诚实（不再按类型分支）。调用点 `verify.rs:395` 相应调整。
- 更新函数文档注释：说明红线依据从"product_fact 是唯一背书类"升级为"AI 永不自动 verify 适用所有类型"。

**改动后行为**：auto-verify 仍完整跑（LLM 自评 + source_quote/anchor 校验 + confidence 门 + rejected 判定），但"过闸"的结果从 `verified` 变 `needs_human_audit`——auto-verify 退化为**预审分诊器**：把"AI 认为 OK、请你重点看"的挑出来，把明显无源的判 rejected，绝不自动放行。功能不空转。

**为何不动 `decide_auto_verify_status`**：它是"证据强约束"判定纯函数，产出 verified 是"证据齐全"的中间信号，语义正确；把 verified→needs_human_audit 的降级放在其**下游** `enforce_*` 一处收口，逻辑更清晰、单测更聚焦。

### 2.2 ①-b：冷启动 peer_case 推送加 integrity_status 过滤

**改动点**：`src/cold_contact_worker.rs`
- `load_peer_case_hooks`（:322-345）的 mongo filter（:328-332）加一条 `"integrity_status": "verified"`：
  ```rust
  doc! {
      "workspace_id": workspace_id,
      "chunk_type": "peer_case",
      "status": { "$in": ["active", "approved"] },
      "integrity_status": "verified",   // ← 新增：对客推送必须内容已核实
  }
  ```
- 与 ①-a 叠加：peer_case 要进冷启动推送，需 status=active/approved **且** integrity=verified，而 integrity=verified 只能人工给（①-a 已堵死 auto-verify 自动给）→ 对客内容必经人工。

### 2.3 ① 测试
- `verify.rs` 纯函数单测：新增/改写 `enforce_*` 断言——product_fact / style_template / peer_case / negative_example 四类的 verified 全部降级 needs_human_audit；非 verified（rejected/needs_review）原样返回。遵守"测试只增量叠加"：保留原 product_fact 用例，补另三类。
- 集成测试（testcontainers，`#[ignore]`）：`load_peer_case_hooks` 只返回 integrity=verified 的 peer_case；needs_review 的 peer_case 即使 status=active 也不入 hook 池。

## 3. ② 类型透传 + 精简抽取 prompt

**目标**：让 LLM 抽取时判定的 `wiki_type`/`chunk_type` 能落库（填上"导入时无路径设类型"的缺环），并精简抽取 prompt（删掉让 LLM 产已删死字段的遗留指令）。**明确不做**：不给 model 加回 safe_claims/forbidden_claims/evidence_items 等 2026-05-25 已删字段（见 §1.5，加回=逆转架构决策）。

### 3.1 ②-a：请求体加类型字段 + 转换函数读取

**改动点**：`src/routes/knowledge/mod.rs`
- `OperationKnowledgeChunkRequest`（:169-212）新增两字段：
  ```rust
  #[serde(default)]
  wiki_type: Option<String>,
  #[serde(default)]
  chunk_type: Option<String>,
  ```
  用 `Option` + `#[serde(default)]`：老请求体不带这两字段时反序列化为 None，向后兼容（现有 create/PUT/import 调用方零破坏）。
- `operation_knowledge_chunk_from_request`（:463-509）在构造 `OperationKnowledgeChunk` 时读取：
  ```rust
  wiki_type: payload.wiki_type.filter(|s| !s.trim().is_empty()),
  chunk_type: payload
      .chunk_type
      .filter(|s| !s.trim().is_empty())
      .unwrap_or_else(default_chunk_type),
  ```
  （wiki_type 是 `Option<String>` 直接存；chunk_type 是 `String`，None/空 → `default_chunk_type()`="product_fact"，保持与现有缺省一致。）其余字段保持 `..Default::default()` 不变。

### 3.2 ②-b：类型稳定性（遵循现有"创建后锁定"设计）

**决策（用户确认）**：wiki_type/chunk_type 创建/导入时由 LLM 设定，**之后锁定不可改**——与现有 `page_merge.rs:31` 注释"类型永不变"、`DEFAULT_LOCKED_FIELDS` 含 wiki_type 一致。

- **create/import 路径**（不走 `apply_chunk_revision` 的字段锁，是 insert 新建）：能设类型。✅ 自洽。
- **PUT 路径**（`crud.rs` replace_one）：`preserve_unmodeled_chunk_fields`（`mod.rs:528-547`）已保护 wiki_type + chunk_type（从 existing 回填，请求体改不了）→ **保持不变**。已有 chunk 的类型不被 PUT 覆盖。
- **`apply_chunk_revision` 路径**（对话 update / AI 修复 / verify）：wiki_type 已在 `DEFAULT_LOCKED_FIELDS`（patch 改它 4xx 拒收）。**顺带把 `chunk_type` 也加入 `DEFAULT_LOCKED_FIELDS`（`page_merge.rs:35-43`）**，消除现有"chunk_type 不在 locked、wiki_type 在"的不对称。
- **未来若需改类型**：走 `Reclassify` 结构化提案接线（本次范围外，另开专题）。

> 注意 create 路径的一个边界：现有 `create_operation_knowledge_chunk`（`crud.rs:192`）直接用请求体构造并 insert，新字段自然生效。import 的两条落库路径（`import_operation_knowledge_apply` 的 chunks 分支 + `ingest_chunked_text` 的 fence 分支）都经 `serde_json::from_value::<OperationKnowledgeChunkRequest>` → 加了字段后 LLM fence JSON 里的 wikiType/chunkType 能被接住（此前被静默丢弃）。

### 3.3 ②-c：抽取 prompt 加类型输出 + 删死字段指令

**改动点**：`src/routes/knowledge/import.rs` 两处 prompt（+ 图片 vision prompt）

**长文本导入 prompt**（`import.rs:66-152`）：
- chunks JSON 模板加两字段（放在醒目决策位，不埋在字段尾部——依据 memory 记录的 A/B 铁证：结构化字段指令位置决定 LLM 是否认真填）：
  ```
  "wikiType": "9 类之一：source/entity/concept/comparison/synthesis/methodology/finding/query/thesis。按知识形态选：有步骤/分支的方法→methodology；具体数据点/案例事实→finding；纯定义→concept；多源综述→synthesis；带论据的判断/主张→thesis；FAQ→query；单一实体→entity；原始出处→source；对比→comparison",
  "chunkType": "4 类之一：product_fact（可对客户承诺的产品事实，需 verified 背书）/ style_template（语气模板）/ peer_case（同行案例参考，不作产品承诺）/ negative_example（不该做的反例）。绝大多数产品/服务事实类知识填 product_fact",
  ```
- **删除** items JSON 里的已删死字段指令：`safeClaims / forbiddenClaims / evidenceItems / customerStages / operationStates / intentLevels / commonQuestions / commonObjections / suitableFor / notSuitableFor`（这些字段 model 已不存在，让 LLM 产它们纯浪费 token + 误导）。同样 chunks JSON 里的 `routingCard`（对应 model 已无的 routing_card，已亲验 `models.rs` grep 零命中）也删。**保留** chunks JSON 里 model 仍有的字段：`title/summary/body/applicableScenes/notApplicableScenes/productTags/businessTopics/sourceQuote`（已亲验 applicable_scenes/not_applicable_scenes 在 `models.rs:1436-1438`）。
  - **实现前必做**：逐字段比对 model 现存字段（`OperationKnowledgeChunk` Default impl `models.rs:1519-1559` 为权威清单）与 prompt JSON 模板字段，**只删 model 确已无对应的**，保留 model 有的。已亲验删除清单：routing_card / safe_claims / forbidden_claims / evidence_items（§1.5 + 本次 grep 确认 model 无这些字段）。

**图片 vision prompt**（`import.rs:688-694`）：
- 加 wikiType/chunkType 输出要求（同上精简版）。
- 补"忠于原文/不推断"护栏（长文本 prompt 有 `import.rs:141`，图片 prompt 缺）：vision 更易脑补，更需这条。

**标签抽取 prompt**（`import.rs:216-235`）：不在本次核心，但 §1 审查发现的"businessTopics 至少抽 1 个"硬填倾向可顺带软化（可选，低优先）。

### 3.4 ② 测试
- 请求体 serde round-trip 单测：带 wikiType/chunkType 的 JSON → `OperationKnowledgeChunkRequest` → `operation_knowledge_chunk_from_request` → chunk 的 wiki_type/chunk_type 正确落值；不带这两字段的老 JSON → wiki_type=None、chunk_type=product_fact（向后兼容）。
- `chunk_type` 加入 locked_fields 后：`apply_chunk_revision` patch 改 chunk_type → LockedFieldInPatch 拒收（补 page_merge 单测）。
- prompt 精简：grep 断言精简后的 prompt 文本不含已删字段名（`safeClaims`/`customerStages` 等）——可用一个纯字符串测试锁定，防未来回退。

## 4. ③ 派工 fix/add 真产出可审草稿

**目标**：派工 `fix_chunk`/`add_chunk` 不再只回文案，而是真调 AI 修复生成 patch 草稿，落成 needs_review 可审记录，供运营在 TaskRail → chunk 编辑器审核。**守红线**：worker 产的草稿一律停 needs_review，绝不自动 verify/apply。

### 4.1 ③-a：抽 `propose_chunk_repair_inner` 纯业务函数

**改动点**：`src/routes/knowledge/repair.rs`
- 把 `propose_chunk_repair` handler（:201-378）的核心逻辑抽成不依赖 `AuthenticatedAdmin` 的纯业务函数：
  ```rust
  pub(crate) async fn propose_chunk_repair_inner(
      state: &AppState,
      workspace_id: &str,
      chunk_object_id: ObjectId,
  ) -> AppResult<Value>  // 返回 { interpretation, patch, missingFields, followupQuestions, stillMissing, confidenceHint }
  ```
  - handler 保留薄壳：解析 admin.current_workspace + path id → 调 inner → `Json(...)`。行为对现有前端 ChunkRepairPanel 零变化。
  - inner 内部：load chunk + parent document → load repair prompt → `generate_agent_json`（保持在 `RUN_BUDGET.scope` 里）→ `parse_repair_response` → 写 usage_log + repair_event（workspace_id 作参数）→ 返回 parsed JSON。
  - **红线不变**：inner 只产 JSON + 写审计，**不改 chunk 本身**（与现有 handler 完全一致）。

### 4.2 ③-b：execute_step 的 fix_chunk 调 inner 并落可审记录

**改动点**：`src/knowledge_task/mod.rs` `execute_step`（:437-486）
- `fix_chunk` 分支（:445-457）改为：
  1. 取 `targetChunkId`（无则返回原文案 fallback，不阻断）。
  2. 调 `propose_chunk_repair_inner(state, workspace_id, chunk_object_id)` 生成 patch 草稿。
  3. **落点（用户确认：落在 task turn/summary）**：把生成的 patch/missingFields/confidenceHint 写进本 step 的 `task_progress` turn 的 details，并把该 chunkId 加入 worker 收尾 summary 的 `needsReviewChunkIds`（现有机制 `mod.rs:355-384`）。
  4. StepOutcome.message 改为诚实文案，如"已为 chunk X 生成 AI 修复草稿（含 N 个待补字段），请在 chunk 编辑器审核后 apply"。
  5. **红线**：worker 绝不拿 patch 去改 chunk（那是 chat_apply / 前端 apply 的人工把关活）；仅把草稿呈现给运营。budget 用现有 STEP_TOKEN_BUDGET/STEP_MAX_LLM_CALLS（:30-31），超额 fail-soft。
- `add_chunk` 分支（:458-464）：本次**不强行接 inner**——add 需要"从什么源起草"，现有 chat draft_chunk 走的是对话上下文，worker 派工场景缺这个上下文。**保持 add_chunk 现状文案**，仅在 §5 记为后续。范围聚焦 fix_chunk（有明确 targetChunkId、可复用 repair inner）。

> 决策依据：fix_chunk 有明确修复目标（targetChunkId）且 repair inner 是现成合规逻辑，接入价值高、风险低。add_chunk 缺起草源，接入需额外设计（起草上下文从哪来），本次不做，避免范围膨胀。

### 4.3 ③ 测试
- `propose_chunk_repair_inner` 抽出后：现有 repair handler 集成测试（若有）不回归；补一个 inner 的直接单测/集成测试（mock LLM 返回 patch → inner 返回结构正确、写了 repair_event、未改 chunk）。
- execute_step fix_chunk 集成测试（testcontainers + mock LLM）：派一个 fix_chunk step → task 完成后 chunk **未被改动**（仍 needs_review）、summary 的 needsReviewChunkIds 含该 chunkId、turn details 含 patch 草稿。

## 5. 不做（YAGNI 边界，明确排除）
- **不给 model 加回** safe_claims/forbidden_claims/evidence_items/routing_card 等 2026-05-25 已删字段（§1.5：grounding 已转语义闸，加回=逆转架构决策）。
- **不改类型可变性设计**：wiki_type/chunk_type 创建后锁定；改类型走 Reclassify 接线是独立专题。
- **不接 add_chunk 的 worker 自动起草**（§4.2：缺起草源，后续专题）。
- **不接 Reclassify apply worker**（§1.4 死水，独立专题）。
- **不动前端组件逻辑**：③落在现有 task turn/summary 机制，TaskRail 展示已有；若要更醒目提示"待审 N 个"是纯前端优化，本次不含。
- **不碰召回向量化**（P1 级别的召回无算法底座，独立大专题）。

## 6. 全局约束（每个 task 隐含遵守）
- 版本：Rust 2021；无 Cargo workspace。
- 红线：AI 永不自动 verify——①③ 新增/改动写路径 chunk 一律 draft+needs_review；auto-verify 不对任何类型自动 verified。
- 禁用词 lint（`check-no-human-takeover`，扫 `src/agent`/`src/routes`/`src/evolution`/`frontend/src` 新增行；docs/ 不扫）：新增**代码内文案/prompt 字符串**不得含 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。关键陷阱：禁词集含**单字"人工"**——所以 ③ 的 StepOutcome 文案、②-c 的 prompt 文案都**不能出现"人工"二字**。统一改用「运营」：如"请运营在 chunk 编辑器审核后 apply""待运营审核""需运营核实"。（spec 文档本身在 docs/ 不被扫，但落地到 import.rs / knowledge_task 的字符串会被扫，实现时务必用 `check-no-human-takeover.sh` 本地自检。）
- 抽取 prompt 改动若涉及 seed prompt（prompts.rs 里的 knowledge.* 模板）需 bump PROMPT_PACK_VERSION；但本次 ②-c 改的是 import.rs 内联 prompt（非 seed pack），不涉及 bump——**实现前确认**：import.rs 的 system/user 是内联字符串还是 load_prompt(seed)。已亲验 `import.rs:61/688` 是**内联字符串**，不走 seed pack，不需 bump。
- 测试基线不回归：`cargo test --lib` ≥350/0；4 PBT 文件累计 ≥33/0。新增测试只 append。
- 序列化：新增请求体字段用 `#[serde(default)]` 保向后兼容。

## 7. 验证
1. `cargo check --tests`（RUSTFLAGS=-Dwarnings）→ 0 error 0 warning。
2. `cargo test --lib` → ≥350/0，含新增 verify.rs enforce 纯函数单测 + 请求体 serde round-trip 单测 + page_merge chunk_type locked 单测。
3. `bash scripts/check-baseline.sh` → 双门绿。
4. `bash scripts/check-no-human-takeover.sh <BASE> HEAD` → 0 violations。
5. 集成测试（CI / Docker）：①-b peer_case verified 过滤、③ fix_chunk 产草稿不改库——`cargo test --test <name> -- --ignored`（本机无 Docker，留 CI）。
6. `cargo test --lib decide_auto_verify` / `enforce_` → 四类 verified 全降 needs_human_audit。

## 8. 执行顺序建议
① → ② → ③（① 最小最独立先落地验证红线；② 中等；③ 依赖抽函数最重）。三条互不阻塞，可独立成 commit / 独立 review。
