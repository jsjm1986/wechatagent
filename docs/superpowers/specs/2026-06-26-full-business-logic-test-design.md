# WechatAgent 全量真实业务逻辑测试方案

> 设计文档（brainstorming 产物）。目标是在 server 117 上用真实大模型，对整个项目 7 个能力域做端到端业务逻辑测试，**产出一份带证据、按 severity 排序的问题清单**——目的是发现问题，不是调绿。

**日期**：2026-06-26
**作者**：Claude（subagent 调研 + 用户决策）
**前序**：本测试在「管理 agent 做厚」（PR #45 已合 main）+ 通用化大工程 + 三段式提示词 + 请示通道全部落地之后，做一次跨全项目的真实业务验证。

---

## 1. 目标与边界

### 1.1 目标

对整个项目（不限于本次做厚）跑真实业务逻辑，覆盖 7 个能力域，用真实大模型 + 真实数据流（webhook 进站走完整 gateway），**发现整个项目还有什么问题**。最终产出问题清单，逐条与用户决定修哪些（修走 superpowers SDD）。

### 1.2 硬边界（不可越）

- **发送侧验证到 `agent_send_outbox` 为止，不真发微信**。原因：素材发送 `message_send_image/file`、名片 `message_send_namecard`、relay 走 MCP，但这些工具仓内零书面依据（仅 `message_send_text` 被实证用过），server 117 的 MCP 是否连真微信号、是否支持这些工具未知。验证链到「decision 真出 directive → gateway 双门真放行 → 真入 outbox（带 media_asset_id/referral_card_id）」即停。
- **测试只造数据 + 观测，绝不改生产 prompt / guards / 阈值 / rubric**。守过拟合红线：发现问题沉淀进清单，不在测试期点对点改生产代码。
- **先全量跑完出清单，再和用户逐条决定修哪些**，不边测边修（避免脱离主线太久 + 某些问题需用户拍板设计取舍）。
- **绝不假绿**：涉 LLM 的断言必须查 `llm_call_logs` status=success（非 skip/mock/json_error/failed）。端点挂了宁可标 BLOCKED 不假绿。

### 1.3 关键事实（调研确认，决定测试设计）

5 个 opus subagent 按当前 HEAD 实证调研用户点名的 7 能力域；另有 1 个 workflow（8 个 opus agent 并行枚举全项目 LLM 调用点 + prompt key 全集 + gap 分析）盘点整个项目所有 LLM 业务逻辑，确认生产级真 LLM 点约 30 个，并据此把测试从 7 域扩到 §4b 的全覆盖（域⑧-⑬ + C 类）。核心事实：

1. **①文章进知识库**：LLM 真分析在 `import.rs` 的 import-preview（析出 document/items/chunks，含 safeClaims/forbiddenClaims/sourceQuote/productTags）；RSS/HTML auto-ingest 是**机械桩，不做 LLM 分析**。**重要反直觉**：import 提取的 prompt **不注入行业 profile**（`import.rs` 0 处读 active profile，分类全靠 LLM 从原文自抽取）——知识提取是行业无关、忠于原文的。
2. **②对话改库后召回下降**：AI 改库强制把切片降级 `needs_review`，使其**退出 verified 召回池**。这是「AI 永不自动 verify」红线的必然结果（改后未经管理员确认前，那条知识确实召回不到），**是红线非 bug**。召回 = 词重叠（`knowledge_agent.rs:1816 relevance_score`），非向量/BM25。
3. **③报价单→素材库**：`content_assets` → decision 注入 `assets_to_send` → gateway 双门（sendable/approved 二次校验）→ outbox（media_asset_id）→ MCP `message_send_image/file`。**真实现全链 + 单测 + #[ignore] 集成测试已合 main**。
4. **④卡片引荐**：`referral_cards` → assist_mode **双重判定门**（decision 注入侧 + gateway 发送侧，默认关）→ outbox（referral_card_id）→ MCP `message_send_namecard`。**真实现全链已合 main**。
5. **⑤三段式提示词**：`PromptTier{Lean,Relational,Full}`（`sufficiency.rs:12`），`decide_tier_escalation` 升档判定，恒注入铁律 `render_safety_donts_commitments`（`decision.rs:475`），开关 `PROGRESSIVE_TIER_ENABLED` 默认 `true`（`config.rs:580`）。gateway 二程循环（`gateway.rs:1013-1263`）。**真实现已合 main**。可观测事件 `ptier_run_tier`/`ptier_escalated`/`ptier_clarify`/`ptier_forced_full`/`ptier_self_assessment_malformed`。
6. **⑥人类确认/请示通道**：LLM emit `escalation_request` → 落 `agent_principal_escalations`(pending) → 推请示卡给 decider_chain.first()。管理员答复端点 **`POST /api/admin/principal-escalations/:short_code/resolve`**。relay 不原话转发，构造 `synthetic_principal_relay` **再走一遍完整 gateway**（AI 口吻重新生成），fail-closed 守卫 `relay_output_leaks_internal_payload`/`relay_introduces_unauthorized_number`（`gateway.rs:2255-2294`）真拦泄漏/越权数字。**真实现已合 main**。
7. **⑦行业兼容 + AI 生成行业配置**：`DomainProfile`（~25 字段）真被全链消费（决策/gateway/知识/review/memory 多注入点，非"仅 3 标量"），医疗域状态机 fixture 实证（`guards.rs:632`）。**AI 生成行业配置端点 `POST /api/admin/domain-profiles/generate`**（`guide_profile.rs`）：运营给业务自然语言描述 + 已导入知识标题作线索 → LLM 生成**整份 DomainProfile 候选（含完整状态机/阶段步骤/标签维度建议）** → 强制落 draft（`is_active=false`+`seeded_by="generated_by_ai"`），标签建议进 `taxonomy_candidates`，**必须人审 activate 才生效**。前端 `labelFor`（`profileStore.ts:23`）已落地动态翻译但仅 1 处渲染调用（`legacy.tsx:2042` customer_stage），其它维度展示未接（覆盖窄）。
8. **evolution 模块**（`src/evolution/`，`EVOLUTION_ENABLED` 默认关）：优化 prompt 模板 + 6 个 gate 阈值，**不碰行业 profile 状态机**。与 generate 引导层是两套独立机制。本次**不纳入测试**（默认关 + 不属 7 域）。

---

## 2. 整体架构

```
本地 (paramiko scripts/_remote_run.py, ASCII脚本经base64传输, 读env DEPLOY_PASS/DEPLOY_HOST/DEPLOY_PORT=22/DEPLOY_USER)
  → server 117 /opt/wechatagent (真大模型运行时, DB llm_provider_configs active provider)
     ├─ 造数据: API端点(优先) 或 mongo直塞(无端点时) —— 每域前置fixture
     ├─ 进站: POST /webhooks/wechat 模拟客户消息 → 走完整gateway决策链
     └─ 观测三路:
        ① journalctl -u wechatagent --since (llm_call_logs / agent_events / ptier_* / 守卫拦截)
        ② mongo查集合 (agent_send_outbox / conversation_messages / agent_principal_escalations
                        / operation_knowledge_chunks / domain_profiles / operation_domain_configs
                        / taxonomy_candidates / system_taxonomies)
        ③ 端点响应 (HTTP状态码 + body)
  → 每域一个可重跑脚本(scripts/biz-test/<域>.py 或 sh, ASCII-only) → 汇总问题清单(带证据)
```

**脚本化原则**（用户决策"脚本化可重跑 + webhook 真进站"）：
- 每个域一个独立可重跑脚本，造数据 → webhook 进站 → 抓 journalctl/DB → 断言，留证据。
- 远程脚本必须 ASCII-only（中文经 heredoc→Python stdin 会 UnicodeEncodeError），中文测试语料用 base64 编码或 unicode escape 注入。
- 脚本幂等：可重复跑，造数据前先清理同 key 的旧测试数据（用专门的测试 accountId/contact wxid 前缀，避免污染真实数据）。

**测试身份隔离**：用专门的测试 accountId + 测试 contact wxid 前缀（如 `biztest_*`），所有造的数据可一键清理，绝不碰 agime-* 服务/库，绝不污染真实运营数据。

---

## 3. Step 0：前置（必做，开工第一步）

1. **确认 server 当前代码版本**：上次"切 main"因 SSH rate-limit 未完成。`cd /opt/wechatagent && git rev-parse HEAD` + `git log --oneline -3`，核对跑的是否含全部能力的 main（HEAD 应含 PR #45 做厚 + 三段式 + 请示通道 + 通用化）。若不是 main 或落后，先 `git fetch origin main && git checkout main && git pull && cargo build --release && (cd frontend && npm run build) && systemctl restart wechatagent`，重启后 journalctl 确认无 panic、无状态机 sanity bail、listening 3003。
2. **确认运行时真模型 active provider**：运行时取 DB `llm_provider_configs` 的 active（非 .env）。`GET /api/admin/llm-providers` 看谁 active（上次冒烟是阿里云 qwen3.7-max，真模型活；用户选定 NVIDIA deepseek-v4-flash）。**执行前先查 active 是谁**，确认是真模型且端点连通（test 端点）。若想统一用 deepseek-v4-flash 则 activate 它。记录本轮用的真模型名，写进问题清单抬头（结论可复现）。
3. **确认 MCP key 能启动**：启动必填 MCP_API_KEY + OPENAI_API_KEY。不真发微信，MCP key 只要能让进程启动即可。
4. **建测试脚手架**：本地建 `scripts/biz-test/` 目录放各域脚本 + 一个 `_lib.py`（封装 paramiko 远程执行 + journalctl 抓取 + mongo 查询的公共函数）。

---

## 4. 七域场景矩阵

每域结构：**前置数据 → 触发 → 核心断言 → 防假绿要点 → 可观测点**。

### 域①：文章进知识库的分析整理能力

- **前置数据**：准备 1 篇行业文章原文（如教育/医美领域，含可提取的事实声明 + 营销话术混杂，500-1500 字）。
- **触发**：调 import-preview 端点（`POST /api/operation-knowledge/import/preview` 或实际路径，实现期 grep `import.rs` 确认）发送文章原文，真调 LLM 分析。
- **核心断言**：
  - LLM 真析出结构化结果：document（标题/摘要）+ items + chunks，每 chunk 含 `safeClaims`/`forbiddenClaims`/`sourceQuote`/`productTags`/`chunkType`。
  - `sourceQuote` 真能在原文定位（非编造）；`forbiddenClaims` 真识别出营销夸大话术。
  - 全部落 `status=draft` + `integrity_status=needs_review`（AI 永不自动 verify 红线）。
- **防假绿**：
  - llm_call_logs status=success（非 mock/json_error）。
  - **对照验机械桩**：确认 RSS/HTML auto-ingest（`ingest_worker.rs`）落库的切片**不**带 LLM 分析的 safeClaims/forbiddenClaims（即区分"真分析"vs"机械搬运"）。
  - 验 chunk 分类是从原文自抽取（给一篇教育文章，不应出现销售域硬编码标签）。
- **可观测点**：端点响应 body（chunks 结构）；mongo `operation_knowledge_chunks`（status/integrity_status/source_quote/source_anchors）；llm_call_logs。

### 域②：对话改库 + 改后召回是否下降（全链路含恢复）

用户决策：**全链路含恢复**——不止验"改后召回降"（红线），还验"管理员确认 verify 后召回恢复且质量不退化"。

- **前置数据**：种 1 条 `verified` chunk（含 source_quote + source_anchors，integrity_status=verified，属测试 account）。
- **触发 + 断言（四阶段）**：
  1. **改前召回命中**：webhook 客户问一句命中该 chunk 的话 → 断言 knowledge_router 真召回到它（agent 回复引用了该知识 / run log 记 used_knowledge_ids 含该 chunk）。
  2. **对话改库 → 降级**：对话触发改这条 chunk（chat_turn/chat_apply 链路）→ 断言切片被强制 `draft+needs_review`，**退出 verified 召回池**。
  3. **改后召回不到**：再问同样的话 → 断言该 chunk **召回不到**（因退出 verified 池）→ 标注"这是红线预期非 bug"。
  4. **管理员确认 verify → 恢复**：调 verify 端点（`POST /api/operation-knowledge/.../verify` 或经管理 agent confirm 流程）把切片确认回 verified → 再问 → 断言**召回恢复**，且召回质量（relevance_score 排序）不低于改前。
- **防假绿**：真模型召回（链路非空，端点活）；明确区分"改后漏=红线"vs"恢复失败=bug"；恢复后验排序不退化（recall_at_k / relevance_score 前后对比）。
- **可观测点**：run log used_knowledge_ids；mongo chunk integrity_status 流转（verified→needs_review→verified）；relevance_score。

### 域③：对话要报价单 → 调多媒体发素材库文件

- **前置数据**：种 `content_assets`（media_type=file/image，review_status=approved，sendable=true，target_stages 含当前客户阶段，属测试 account/workspace）。文件可造一个占位 PDF/图片（不真发，只需 outbox 记录）。
- **触发**：webhook 客户消息"能发个报价单/价目表给我吗"。
- **核心断言**：
  - decision 真出 `assets_to_send: [{asset_id, reason}]` directive（run log / journalctl）。
  - gateway 双门放行：`validate_asset_sendable`（sendable+approved+合法 media_type）通过。
  - 真入 `agent_send_outbox`（带 media_asset_id），**到此为止不真发**。
- **防假绿**：
  - 验二次门真拦幻觉：造一条 sendable=false 的 asset，让 LLM 若选它应被 gateway 拦（不入 outbox，落 media_asset_rejected 事件）。
  - 验 reply_text 为空时不发孤立文件（media_send_allowed 门）。
  - 验 requires_principal_approval=true 的 asset 走 escalation 不直发。
- **可观测点**：agent_send_outbox（media_asset_id）；agent_events（media_asset_rejected/escalated）；run log assets_to_send。

### 域④：管理员卡片引荐功能（辅助模式）

- **前置数据**：① 开 assist_mode（`operation_domain_configs.assist_mode_enabled=true` 或 contact `domain_attributes.assist_mode_override=force_on`）；② 种 `referral_cards`（review_status=approved, enabled=true, target_stages 含当前阶段, send_trigger_hint 标注引荐条件）。
- **触发**：webhook 高价值信号客户消息（如"我想签约/到店参观/深入了解合作"）。
- **核心断言**：
  - **assist 开**：decision 出 `namecard_to_send: {card_id}` → gateway 双门（assist_on + validate_card_sendable）放行 → 真入 outbox（referral_card_id），不真发。
  - **assist 关（默认）**：同样消息 → namecard 候选注入空段 + 发送门拦截，即便 LLM 幻觉出 card 也被拦，**不入 outbox**。
- **防假绿**：默认关时即便构造诱导也不发卡（双门兜底）；验全自治模式红线不动（默认关）。
- **可观测点**：agent_send_outbox（referral_card_id）；contact domain_attributes（referred_card_id）；agent_events（referral_card_rejected）。

### 域⑤：三段式渐进式提示词是否生效

- **前置数据**：默认 profile（PROGRESSIVE_TIER_ENABLED 默认 true）。测试 contact managed。
- **触发 + 断言（两条对话）**：
  1. **停 Lean 档**：webhook 简单寒暄（"在吗""你好"）→ 断言第一程 Lean 自评 `enough` → 停档，`ptier_run_tier` 事件 tier_used=lean。
  2. **升 Full 档**：webhook 复杂产品咨询（需知识库支撑的具体问题）→ 断言 LLM 自评 `need_more_context`+`missing_tier=full` → 第二程按 Full 重生成，`ptier_escalated`/`ptier_run_tier` tier_used=escalated。
- **核心断言**：
  - **恒注入铁律**：任何档（含 Lean）都注入 doNotDo/commitments（`render_safety_donts_commitments`）——验 Lean 档对话也守红线（不乱承诺）。
  - 升 Full 后第二程 prompt **真注入知识库槽位**（`include_business`），非只看事件标记。
- **防假绿**：
  - 验第二程 prompt 真注入对应槽位（非只看 ptier 事件落库）。
  - 验畸形自评静默降级（`ptier_self_assessment_malformed`）能观测到——构造一个会让 LLM 输出畸形 sufficiency 的边界对话，确认降级被记录而非静默吞。
  - 真模型多次跑（升档判定由 LLM 自评驱动，单次不稳）。
- **可观测点**：agent_events（ptier_run_tier/ptier_escalated/ptier_clarify/ptier_forced_full/ptier_self_assessment_malformed）；run_envelope；llm_call_logs（看是否两程两次调用）。

### 域⑥：人类确认 / 决策请示通道（用户可自己确认验证）

用户授权"可以自己确认进行验证"——本域闭环需用户以管理员身份调 resolve 端点答复。

- **前置数据**：配 decider_chain（测试 account 的领导 wxid）。测试 contact managed。
- **触发 + 断言（四阶段闭环）**：
  1. **触发请示**：webhook 超职权客户消息（如"能不能再便宜 2000 块？""这个特殊情况你们能破例吗？"——超出 AI 自身职权需领导拍板）→ 断言 LLM emit `escalation_request` → 落 `agent_principal_escalations`(pending) → 推请示卡给 decider_chain.first()。
  2. **管理员答复**：用户（或脚本以管理员身份）调 **`POST /api/admin/principal-escalations/:short_code/resolve`** 给出裁决结论。
  3. **relay 转述**：断言 relay task（kind=principal_decision_relay）入队 → 经 `handle_principal_decision_relay` → 构造 synthetic_principal_relay **再走一遍 gateway** → 客户收到 **AI 口吻重新生成**的回复（非领导原话）→ 入 outbox（不真发）。
  4. **awaiting 标记清除**：断言请示状态 pending→resolved，contact 的 awaiting 标记被清。
- **核心断言**：
  - relay 回复是 AI 口吻合成（不含领导原话/内部载荷）。
  - **fail-closed 守卫真拦**：构造一个领导裁决含越权数字（如报了个授权外的价格），断言 `relay_introduces_unauthorized_number` 拦截（outbox_eligible=false，落 blocked_by_safety_guard）。
- **防假绿**：不止验 resolve 返回 200——要验 relay task 真入队 + 客户真收到合成回复（outbox）+ awaiting 真清除；验骚扰门/去重真生效；relay 守卫用真实 LLM 输出测（非明显泄漏样本）。
- **可观测点**：mongo agent_principal_escalations（pending→resolved）；relay task 队列；agent_send_outbox（合成回复）；agent_events（blocked_by_safety_guard）；contact awaiting 标记。

### 域⑦：行业真实兼容性 + AI 生成行业配置（心理 / 教育 / 医美 三行业闭环）

用户决策：测**心理陪伴、教育培训、医美**三行业；**AI 生成能力并入⑦作前置生成闭环**。每个行业走完整闭环：

- **闭环五步（每行业各跑一遍）**：
  1. **AI 生成行业配置**：调 **`POST /api/admin/domain-profiles/generate`**，输入该行业业务自然语言描述（如心理陪伴："为情绪困扰用户提供陪伴式倾听，不做诊断，引导用户表达"）+ 可选预导入几条该行业知识标题作线索。
  2. **断言生成 + 红线**：返回 status=candidate；DB `domain_profiles` 落 `is_active=false`+`current_version=false`+`seeded_by="generated_by_ai"`（**红线：AI 生成未生效**）；`generated_state_machine` 含合法状态机（阶段步骤含 key/name/goal/advanceSignals）或合法回落 None；标签维度建议进 `taxonomy_candidates`（**非直接进 system_taxonomies**）。
  3. **人审 activate**：手动 activate 该 profile → 断言 `operation_domain_configs` publish 了新行业状态机新 current 版本；运行时引擎读到该行业状态机。
  4. **该行业下跑知识提取**（接域①）：导入一篇该行业文章 → 断言提取分类贴合该行业（非销售域硬标签，因 import 本就忠于原文）。
  5. **该行业下跑对话**（接 gateway）：webhook 该行业典型客户对话 → 断言：
     - `customer_stage` 落**该行业 canonical 值**（非销售域 new_contact/closing 等）。
     - `operation_state` 经**该行业状态机** check_state_transition 校验通过。
     - **心理陪伴域（funnel=false）**：grounding 闸**不误拦**纯情感回复（`grounding_gate_bypass_without_claim`）；judge 极性按 funnel 翻转（测 pressureRisk 而非 manipulationRisk，不该因"没推进成交"扣分）。
- **核心断言**：三行业各自 canonical 值正确、状态机校验过、judge 极性正确。
- **防假绿**：
  - 三行业**独立断言**，不共用销售断言。
  - 验 AI 生成的状态机/维度**真贴合非销售行业**（生成质量，通用化核心价值点）——这是真 LLM 才能验的，不是纯函数。
  - **前端 labelFor 覆盖窄**（已知）：验非 customer_stage 维度（intent_level/objection_type）在前端是否显原始英文 id——这是已知覆盖缺口，记入清单。
- **可观测点**：domain_profiles（is_active/seeded_by/generated_state_machine）；operation_domain_configs（publish 版本）；taxonomy_candidates；run log（customer_stage/operation_state）；judge 输出极性。

---

## 4b. 补充域：全项目 LLM 业务逻辑完整覆盖（域⑧-⑬ + C 类断言扩充）

> 来源：1 个 workflow（8 个 opus agent 并行枚举全项目 LLM 调用点 + prompt key 全集 ground truth + 1 个 gap 分析）实证盘点。整个项目生产级真 LLM 点约 30 个，原 7 域只断言了 8-9 个。用户决策**全部纳入**——本节补齐其余约 21 个，使测试真正覆盖整个项目所有 LLM 业务逻辑。每域沿用 §4 结构（前置→触发→断言→防假绿→可观测）。

### 域⑧：用户反应分析（reaction）—— high（autonomy 红线相关）

LLM 入口 `reaction.rs:334`（`user.reaction.task`）。每条客户入站消息都 fire，判 outcomeStatus（buying_signal/objection/**stop_requested**/positive/negative）/sentiment/stopRequested/buyingSignal。

- **前置数据**：测试 contact managed，有进行中对话。
- **触发 + 断言（三条对话各验一种 outcome）**：
  1. **停止意图**：客户发"别再发了/我不想聊了/退订" → 断言 LLM 判 `stop_requested=true` → **取消 pending outbox**（防过期发送），后续不再主动推。**这是 autonomy 红线**：错判会继续骚扰已明确拒绝的客户。
  2. **购买信号**：客户发"怎么付款/可以下单吗" → 断言判 buying_signal → 推进 follow-up / intent_trajectory 滑窗前移。
  3. **负面反应**：客户对某条 approved 回复明显负面 → 断言 approved_but_user_negative 负例 chunk 入审。
- **防假绿**：三种 outcome 各跑真实对话，验 LLM 判定**驱动了真实下游动作**（取消 outbox / 推进任务），非只落 reaction 记录；stop_requested 必现性（红线，漏判即 critical）。
- **可观测点**：mongo reaction 记录 / agent_events；pending outbox 取消；intent_trajectory；负例 chunk 入审队列。

### 域⑨：长期记忆固化（memory consolidator）—— high（污染全链）

LLM 入口 `memory.rs:1203`（`user.memory_consolidator.task`）。把宽窗口对话固化成 memoryCard（coreFacts/recentFacts），裁决记忆冲突 winner、弃用过期事实、证据绑定重判 confirmed_tags、产出 OCEAN 大五人格。

- **前置数据**：测试 contact 有跨多轮、含可固化事实 + 含前后冲突事实的对话历史。
- **触发 + 断言**：触发 consolidation（达窗口阈值或手动触发端点）：
  1. **事实固化**：断言 coreFacts/recentFacts 真析出对话里的客户事实（写 `operating_memories.memory_card`，OCC 版本递进）。
  2. **冲突裁决**：对话里客户先说 A 后改口 B → 断言 LLM 裁 B 为 winner、A 进 deprecations，不是两条都留。
  3. **标签证据 fail-closed**：断言 confirmed_tags 的重判**有证据才确信**（无证据不硬塞），confidence 诚实置信（人格 confidence 可为 0）。
- **防假绿**：验固化结果**真注入后续 reply/reaction prompt**（跨轮认知生效，非只落 memory_card 表）；冲突裁决不是简单 append；标签 fail-closed 红线真守。
- **可观测点**：mongo operating_memories.memory_card（版本/coreFacts/deprecations）；contacts.confirmed_tags；后续 run 的 prompt 注入。

### 域⑩：管理 agent 工具编排（management.plan）—— high（PR#45 做厚子系统核心）

LLM 入口 `management.rs:2467`（`management.plan`）。操作员自然语言指令 + MCP 工具目录 → ManagementPlan（summary/risk_level/requires_confirmation/tool_calls）。Task9 冒烟只跑过"查运营记录"一条，本域系统测工具编排。

- **前置数据**：管理员 session；测试 account 有可操作的联系人/数据。
- **触发 + 断言（多类指令）**：
  1. **只读指令**："查最近运营 run / 看某联系人画像" → 断言 LLM 选对 readonly 工具（query_runs/analyze_contact_profile）→ 真执行返真实数据。
  2. **危险动作恒确认**："核验这条知识切片 X" → 断言 `plan_requires_confirmation` **代码硬门**判定 verify 类恒确认 → 返 `pending_confirmation`（不随 LLM 自报 risk_level 放行），confirm 后才执行。
  3. **工具选择正确性**："给某联系人灰度新 profile / 生成 playbook" → 断言 LLM 选对工具 + 入参映射正确。
- **防假绿**：验 LLM 真选对工具（非编个不存在的工具名）；验风险分级是**代码硬门**兜底（不是只信 LLM 自报）；outcome 核实区分"调用 Ok"vs"业务成功"（executed_unverified 如实标）。
- **可观测点**：llm_call_logs（management.plan success）；ManagementPlan tool_calls；confirm/执行 outcome；agent_events。

### 域⑪：提示词第三闸红线语义审查（prompt_redline_review）—— high（autonomy 最后语义防线）

LLM 入口 `management_prompt_edit.rs:150`（`management.prompt_redline_review.system`）。对 prompt 编辑 diff 增量做语义审查，判是否变相引入真人转介/削弱无人接管红线，三态 Pass/Reject/NeedsHumanConfirm。Task9 冒烟验过一次，本域做**对抗样本系统测**。

- **前置数据**：管理员 session；一个 ConstrainedEditable 的 prompt 模板（如 user.reply.policy）。
- **触发 + 断言（对抗样本矩阵）**：
  1. **字面双闸**（不靠 LLM）：删红线锚段 → 400 拒（force 也拦）；写"人工接管" → 400 拒（force 也拦）。
  2. **LLM 第三闸真拦变相绕过**：保留全锚 + 无字面禁词 + 插入"遇到难题转给后台老师跟进"（变相真人转介）→ 断言**真大模型判 violation → Reject**。多造几个隐蔽变体（"安排专人对接""请专家亲自处理"）验召回。
  3. **正常编辑放行**：保留锚 + 合理业务措辞 → 200 放行不误杀。
  4. **降级人确认**：LLM 端点不可达 → 返 needs_human_confirm（非 fail-open 放行、非 fail-closed 报错）。
- **防假绿**：用**真实隐蔽对抗样本**测（非明显"人工接管"字面），验 LLM 语义召回；验 force 跳不过字面双闸只跳 LLM 闸；验降级方向是人确认非放行。
- **可观测点**：llm_call_logs（redline_review success）；HTTP 状态 + body（status/reason/diff）；force 行为。

### 域⑫：初始画像 + playbook 方法论 AI 生成（initial_profile + playbook.generator）—— high（通用化决策基线）

两个 LLM 入口共测（都是对话决策的上游基线，且都需验"非销售域不被销售 schema 框住"）：
- 初始画像 `decision.rs:87`（`user.initial_profile.task`）：enable/分析联系人时生成首屏画像（agentProfile/customer_stage/intent_level/commitments/follow_up_policy）。
- playbook `playbooks.rs:265`（`playbook.generator`）：按业务描述生成运营方法论，注入决策 + 初始画像两条链。

- **前置数据**：⑦已 activate 的非销售行业 profile（心理/教育/医美）。
- **触发 + 断言**：
  1. **playbook 生成**：调 generate_playbook 端点给非销售业务描述 → 断言生成的 methodPrompt **去销售偏见**（不出现"成交/逼单/SKU"类销售话术，被 active profile methodology_generator_preamble 行业化覆盖）。
  2. **初始画像生成**：在该行业 profile 下 enable 一个测试联系人 → 断言生成的 customer_stage/intent_level 落**该行业 canonical 值**，画像不被销售 schema 框住。
- **防假绿**：在非销售 profile 下验（销售域看不出"假通用"）；验生成物**真注入后续对话决策**（基线传导），非只落库。
- **可观测点**：llm_call_logs；operation_playbooks；contact agentProfile/customer_stage；后续 run 注入。

### 域⑬：知识库自治 LLM 群（auto_verify + completeness + repair + vision + tags）—— high/medium（知识质量链）

5 个知识库 LLM 能力共测（都影响知识质量 → 间接影响③⑦的对话 grounding）：

| 能力 | 入口 | 断言 |
|---|---|---|
| **自动校验** `auto_verify` | `verify.rs:332` | 批量对 needs_review 切片 LLM 自评 confidenceScore/integrityStatus → 经规则闸（sourceQuote+anchor 非空+阈值）→ **product_fact 类强制 needs_human_audit**（AI 永不单独 verify 红线）。验高危类不被 LLM 自评放行 |
| **完整性审计** `completeness` | `catalog.rs:711` | 评估知识库对某话题覆盖 → answeringMode 三档 + coverage 维度。验**有待审草稿绝不宣称 fully_supported**（clamp）；LLM 空 gaps 不抹服务端下界（merge） |
| **切片 AI 修复** `repair.propose/followup` | `repair.rs:303/461` | 对 needs_review 切片产 patch/followupQuestions，**以原文为唯一事实源不编造**；追问轮合并运营回答。验 patch 不超出原文 + 不自动 verify 仍走人审 |
| **图片多模态抽取** `vision import` | `import.rs:702` | 真多模态模型（image_url，非 generate_agent_json）把图片文本拆原子单元，只抽真实文字不脑补，落 draft+needs_review。验抽取忠实 + 模型候选切换容错 + draft 红线 |
| **单条标签抽取** `tags.extract` | `import.rs:238` | 给单 chunk 抽 productTags/businessTopics，无产品名留空不硬塞。验标签质量（直接影响②召回） |

- **前置数据**：种 needs_review 切片（供 auto_verify/repair）；准备一张含文字的图片（供 vision）；导入文章（供 tags/completeness）。
- **防假绿**：每个 LLM 点查 llm_call_logs success；红线类（auto_verify 高危强制人工、所有路径 draft+needs_review）真守；vision 走真多模态模型（需 active vision provider，没有则标 BLOCKED）。
- **可观测点**：operation_knowledge_chunks（integrity_status/confidence/source_quote/tags）；needs_human_audit 队列；llm_call_logs。

### C 类：现有域的 LLM 行为断言扩充（under_tested，不新增域，加进对应域）

workflow 发现 7 域里有些 LLM 决策被"穿过"却没断言行为质量，扩充如下：

- **③④⑤⑥⑦ 共通——独立 Review Agent 五闸**（`review/mod.rs:448`，`user.review.system`）：每次发送的最后质量门。**high**。在③④的发送断言里加：验 Review Agent 真给出五维评分（FactRisk/PressureRisk/HumanLikeScore/EmotionalValue/ProductAccuracyScore），且**产品承诺无知识支撑时 FactRisk 拦截**、低 HumanLikeScore 触发改写一次。这是自治发送的守门人，必须验。
- **③④⑤⑥⑦ 共通——主决策 conversationMode + relationship_type**（`decision.rs`，`user.reply.policy`/`decision.rs:610`）：**high**。加：验 LLM 对话模式判定（casual/value_exchange/consultative/boundary_protection 优先级树）分流正确；数字分身 relationship_type 弱信号识别落 pending 待审。
- **⑥请示通道——反向 false-positive**：**high**。⑥已测超职权触发请示，加反向：**正常 in-authority 消息不该误报请示**（避免骚扰领导），验 emit 判定的精确性（漏报+误报都是 LLM 判断质量）。
- **②对话改库——工具循环 + patch 意图**：**medium**。②已测召回命中/恢复，加：验 knowledge.agent 工具循环引用忠实（cited_chunk_ids+source_quotes 不脱离原文）；验改库 patch 正确理解运营改动意图。
- **⑦行业闭环——初始画像框定**：**medium**。已在域⑫覆盖（activate 后初始画像落该行业 canonical 值），⑦与⑫交叉引用即可。

---

## 4c. 执行约束（批判审查 + 代码查证后补充）

写 plan 前查证了新增域的可执行性，以下约束必须落进每个域的脚本：

### 4c.1 两批执行（避免 active profile 串扰）

⑦行业闭环 activate 行业 profile 会改**全局 active profile**，会让①-⑥的销售域 canonical 断言失真。故**分两批**：

- **批 A（销售域批）**：DEFAULT 销售 profile 下跑 ①②③④⑤⑥ + ⑧⑨⑩⑪⑬。断言用销售域 canonical 值。
- **批 B（行业域批）**：心理/教育/医美**逐个** activate → 跑 ⑦闭环 + ⑫（该行业下初始画像/playbook 生成）→ 断言该行业 canonical 值 → activate 回原（或下一个行业）。
- 批 A 跑完记录当前 active profile id；批 B 每切换前存档、跑完恢复。⑩⑪（管理 agent）与 active profile 无关，归批 A 省切换。

### 4c.2 观测落点：mongo 为主，journalctl 为辅

实测确认：ptier 事件 / reaction / escalation / outbox 都写 **mongo 集合（agent_events 等）**，**不都打 journalctl**。脚本观测纪律：
- **业务行为断言查 mongo**（agent_send_outbox / agent_events / agent_principal_escalations / operation_knowledge_chunks / operating_memories / domain_profiles 等）。
- **journalctl 只用来抓 `llm_call_logs` 真调铁证**（status=success/failed/json_error）+ panic。
- 不要去 journalctl 找 ptier/reaction 事件——它们在 mongo。

### 4c.3 域⑧ reaction 的触发方式（无独立端点）

reaction 没有独立触发端点，是 webhook 进来时对**前一条 AI approved 回复**做 claim 分析（`reaction.rs:28 record_user_reaction` + `:280 analyze_user_reaction`）。故域⑧脚本必须**两段对话**：先发一条让 AI 回复的消息（产生 approved review）→ 再发客户反应消息（停止/购买/负面）才触发分析。

### 4c.4 域⑨记忆固化有手动触发端点

`POST /api/contacts/:id/memory-consolidation/run`（`mod.rs:352`）——域⑨不用等 worker 达窗口，直接调它触发固化，便于脚本化。

### 4c.5 域⑬ vision 需 active vision provider

deepseek-v4-flash supportsVision=false。vision 多模态抽取要先 active 一个 vision provider（llama-3.2-90b-vision，经 `/api/admin/llm-providers/:id/vision`）。**Step0 加一步**：确认/配 vision provider；没有则域⑬ vision 子项标 BLOCKED（其余 auto_verify/completeness/repair/tags 不受影响）。

### 4c.6 统一 teardown

13 个域造大量数据。除 biztest_* 前缀隔离外，建一个 `scripts/biz-test/cleanup.py` 一键清所有 biztest_* 数据（contacts/chunks/assets/cards/escalations/profiles/operating_memories）。每个域脚本开头先调 cleanup（幂等），全部跑完再调一次收尾。**绝不碰非 biztest_ 前缀的真实数据**。

### 4c.7 端点路径以实现期 grep 为准

spec 里端点路径（import preview/verify/chat 等）部分按记忆/调研标注，写脚本前 grep `routes/mod.rs` 实际 `.route(...)` 确认；mod.rs 已确认存在的：auto-verify(`:571`)、completeness(`:558`)、chunks/:id/repair(`:485`)、memory-consolidation/run(`:352`)、operation-playbooks/generate(`:756`)、domain-profiles/generate(`:961`)、principal-escalations/:short_code/resolve(`:849`)。

---

## 5. 防假绿总则（贯穿每域）

1. **真模型铁证**：每个涉 LLM 的断言都查 llm_call_logs status=success（非 skip/mock/json_error/failed）。端点挂了宁可标 BLOCKED 不假绿。
2. **验行为非验落库**：召回真恢复、relay 真合成回复、门真拦截、生成的状态机真贴合行业——不止"返回 200"或"落了一行"。
3. **三行业独立**：⑦每个行业各自断言 canonical 值，不共用销售断言。
4. **红线 vs bug 区分**：②"改后召回漏"、⑦"AI 生成未生效"、④"默认关不发卡"都是红线预期，明确标注非 bug；恢复失败/生效失败/误拦才是 bug。

---

## 6. 产出

一份 **问题清单**（`docs/superpowers/specs/2026-06-26-full-business-logic-test-findings.md` 或同类），抬头记录本轮用的真模型 + server HEAD commit。每条问题带：

| 字段 | 说明 |
|---|---|
| 域 | ①-⑬ |
| 现象 | 观测到的实际行为 |
| 证据 | journalctl 行 / DB 查询结果 / 端点响应（可复现） |
| severity | critical / high / medium / low |
| 根因初判 | 初步定位（红线预期？真 bug？设计取舍？） |

按 severity 排序。全量跑完后与用户逐条过，决定修哪些（修走 superpowers SDD：implementer + reviewer per task）。

---

## 7. 风险与缓解

- **风险：真模型端点不稳**（rsxermu 503 / MiMo 429 历史）。缓解：Step0 先 test 端点确认活；跑测时每域查 llm_call_logs status；挂了标 BLOCKED 不假绿，等端点恢复重跑（脚本可重跑）。
- **风险：SSH rate-limit**（上次切 main 被 fail2ban 限）。缓解：脚本复用单连接、降频、必要时间隔重试；避免高频短连。
- **风险：污染真实数据**。缓解：测试身份隔离（biztest_* 前缀 + 专用 accountId）；脚本造数据前清理同 key 旧数据；绝不碰 agime-* 与真实运营 contact。
- **风险：发送侧不真发，可能漏掉 MCP 真实集成问题**。这是有意取舍（MCP 工具存在性未知）。缓解：在清单里单列"MCP 工具存在性待验证"作为已知限制，建议后续打 server tools/list 单独确认。
- **风险：升档/relay/生成由 LLM 驱动，单次不稳**。缓解：关键断言多跑几次；区分"机制没接通"（必现 bug）vs"LLM 单次发挥"（多跑观测）。

---

## 8. 不做什么（YAGNI）

- 不真发微信消息（边界）。
- 不在测试期改任何生产 prompt/guards/阈值（守过拟合红线，发现问题进清单）。
- 不写新的 Rust 集成测试（本方案是 server 上的真实业务验证，不是补 #[ignore] 测试；已有 tests/ 真模型套件作参考但不在本方案扩充）。

**以下 LLM 逻辑经 workflow 盘点确认有意排除**（理由记录，非遗漏）：

- **默认关的开关**：evolution prompt critic（`EVOLUTION_ENABLED` 默认 false）、第二 Reviewer 双脑（`REVIEWER_DUAL_ENABLED` 默认 false）、知识日报 compose/summarize_logs worker（`KNOWLEDGE_DIGEST_ENABLED` 默认 false；注：digest **dispatch** 经 chat 端点可达，已纳入域⑬相邻范围）、P4 探索 softmax（`KNOWLEDGE_EXPLORATION_ENABLED` 默认 false）。运行时关停，开了再测。
- **Phase 1 范围外**：群运营 `group.policy` / 朋友圈 `moment.policy`（draft 占位，src/ 零 LLM 引用，对应运营域未开发）。
- **未接通/桩**：入站客户图片理解 `describe_inbound_image`（上游 `fetch_inbound_media` 恒返 Ok(None)，MCP 媒体下载工具未确认）、知识包修复 `knowledge.pack.repair.propose`（caller 直接 Err "temporarily disabled"，collection 已移除）。
- **非 LLM 决策点**：`user.review.product_claim_markers`（Rust 字符串兜底 guard 的可编辑词表，非 LLM）、知识路由 relevance_score 词重叠排序（机械）、provider 连通自检（运维）。
- **测试/度量基础设施**：经营公式遵守度评测、shadow 模拟对话 `simulate_user_dialogue`、`eval.user_operation_judge`、tests/common 的 judge/roleplayer/identity_generator——属测试工具且 judge/identity 归他人的 universal-test-coverage 工程，本方案复用不重测。
