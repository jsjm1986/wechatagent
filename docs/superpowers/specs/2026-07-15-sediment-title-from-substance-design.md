# 领导授权沉淀知识 title/body 误用 reviewer 质检点评 — 修复设计

- 日期：2026-07-15
- 状态：设计待实现
- 关联：`docs/superpowers/specs/2026-07-14-principal-authorization-exemption-design.md`（B 类沉淀由该功能引入）

## 背景

领导授权豁免 B 类（`exemption_type=knowledge`）沉淀功能上线后，通过真实全流程测试（请示 E6PM5，客户吴界）暴露一个真实缺陷：沉淀进知识库的 verified chunk，其 `title` 是一段 reviewer 内部质检点评黑话（"回复短句分段、口语化、避开了'我帮你确认稍等'这条明确 doNotDo……emotionalValue 只到 6……"），而非一条产品知识应有的标题。

## 根因（单一、已亲验，非猜测）

`sediment_principal_authorized_knowledge`（`src/agent/escalation/ledger.rs:283`）与 `emit_knowledge_gap_proposal`（`ledger.rs:184`）都用 `entry.reason` 当知识 chunk 的 `title`，并在 `body` 里塞 `卡点：{reason}`。

而 `entry.reason` 的语义是"卡点原因 / 给领导看的上下文"（`models.rs` `AgentPrincipalEscalation.reason` 字段注释「卡点原因」）。它有两种来源，均不适合当"面向全体复用的知识标题"：

- `escalate_held_decision`（`src/agent/escalation/mod.rs:102-106`，高风险硬闸走此路径，E6PM5 即是）：
  `reason = review.hold_reason`（空则 `review.review_summary`）→ **Review Agent 的质检点评黑话**。
- `trigger_principal_escalation`（`src/agent/gateway.rs:722`，out_of_scope 等）：
  `reason = req.reason`（decision agent 自填的卡点描述）→ 相对可读，但仍是"卡点"非"知识标题"。

真正的知识内容是 `decision.substance`（`models.rs` `PrincipalDecision.substance` 字段注释「决策实质……AI 口吻转述的事实源」）。

## 影响面（三重，全部亲验）

1. **召回打分被扭曲（最实质）**：`score_chunk_for_query`（`src/agent/knowledge_tools.rs:363`）`score += relevance_score(query, &chunk.title) * 3.0` —— title 是**权重最高**（×3，高于 summary ×2、body ×1）的召回信号。质检黑话 title 对客户产品问句（如"眼袋怎么弄"）几乎零词命中 → 这条 verified 产品知识**召回排名下沉、可能召不回**。
2. **进 Reply Agent 决策 prompt**：`knowledge_router.rs:260` 把 `title={}` 直接拼进候选知识清单交给决策 LLM → AI 看到黑话当知识标题，干扰决策。body 中的 `卡点：{reason}` 同样进 prompt。
3. **展示层不可读**：前端 / router 展示的知识标题是内部质检黑话。

## 存量污染范围（已亲验生产库）

- B 类 verified/active 污染 chunk：**1 条**（`_id=6a566a9d6f89ea84b3b24d9d`，E6PM5 真实测试产生，`status=active integrity=verified`，实质生效中）。
- `emit_knowledge_gap_proposal` 产生的 `真人决策沉淀（待审核）：` draft：**1 条**（`_id=6a54f281ce8e1ff82a77cd4a`，`status=draft integrity=needs_review`，人工会审，影响小）。

## 修复设计

### ① 新增 title 生成：LLM 提炼为主 + 确定性兜底

- **确定性兜底纯函数** `derive_sediment_title_fallback(substance: &str) -> String`（放 `ledger.rs`，可单测）：
  - 取 substance 首句：截到第一个句末标点（`。！？\n` 任一）之前。
  - 限长：超过 40 个字符（按 `chars()` 计，不按字节，避免截断多字节）则截断并加省略号。
  - 空 substance → 返回一个固定安全标题（如 `领导授权沉淀`），配合 sediment 现有"空 substance 直接跳过"逻辑，此兜底实际只在有 substance 时被用到。
- **LLM 提炼**：新增 prompt_key `escalation.sediment.title`，经 `prompts::ensure_prompt_pack_v2` 种入（与 `escalation.principal.interpret` 同范式），通过唯一 LLM JSON 入口 `generate_agent_json`（`src/agent/mod.rs:215`）从 substance 提炼一句知识标题（要求：一句话、不含质检黑话、面向知识复用）。
- **失败即兜底**：LLM 出错 / 返回空 / 解析失败 → 回退 `derive_sediment_title_fallback`。沉淀永远成功、title 永远可读。
  - 预算安全（已亲验）：`generate_agent_json` 内部 `record_call` **不抛错**（只有 tool_call 才抛 `BudgetExceeded`）；title 提炼是纯 JSON 生成、无 tool call → 绝不因预算被硬拦。且 sediment 在 gateway 返回之后跑，run budget scope 多半已退出（`current_run_budget()=None` 亦正常）。

### ② 修 `sediment_principal_authorized_knowledge`（B 类 verified）

- `title` = 提炼结果（不再用 `entry.reason`）。
- `body` 去掉 `卡点：{reason}` 行，保留：`源自客户「{contact}」请示 #{code}。` + `领导裁决：{substance}` + `约束：{...}`。
- 其余（domain / chunk_type / status=active / integrity 两步法 verify / 自锚定 / source=PrincipalAuthorized）**一律不动**。

### ③ 修 `emit_knowledge_gap_proposal`（可泛化 draft 提案）

- `title` 同样从 substance 提炼（复用 ① 的 LLM+兜底路径）；空 substance 时用固定安全标题。
- `body` 去掉 `卡点：{reason}` 行。
- draft + needs_review 契约不动（AI 永不自动验证红线）。

### ④ 存量修正（一次性脚本）

- `scripts/` 下一次性 mongosh 脚本（**非 migration**——不适合每次启动跑、且要调 substance 提炼逻辑），对生产库：
  - 找出 title 以 `领导授权沉淀：` / `真人决策沉淀（待审核）：` 开头的 chunk。
  - 就地 `$set` 修正 title（从该 chunk 自身的 `source_quote` 或 body 里的"领导裁决："段提取 substance，走确定性兜底提炼——脚本内不调 LLM，用与 `derive_sediment_title_fallback` 等价的首句+限长逻辑）+ body 去卡点行。
  - 备份 → 修正 → 回读三段式（与既有 `scripts/cleanup_non_human_managed.js` 范式一致）。

## 测试

- 单元测试 `derive_sediment_title_fallback`：
  - 首句提取（有句号 / 无句号取整段）。
  - 限长截断（>40 chars 截断加省略号；多字节字符按 chars 不按 bytes）。
  - 空 substance → 固定安全标题。
  - 多行 substance（换行作为句末）。
- 不降基线：`cargo test --lib` ≥ 350、PBT 四件 ≥ 33，`scripts/check-baseline` 双门绿。
- lint：`check-no-human-takeover` / `check-no-model-hint` 新增行 / prompt 文案不踩禁词。

## 不做（YAGNI）

- 不改 `entry.reason` 本身的语义 / 来源（reason 给领导看质检点评是合理的，问题只在"拿 reason 当知识标题"）。
- 不改召回打分权重、不动 knowledge_router / knowledge_tools。
- 不改 relay / R5.4 产品门 / 豁免写入逻辑（本次真实测试已验证这些正确）。
- 不重发 E6PM5 被 MCP 429 卡住的那条消息（独立问题，与本修复无关）。
