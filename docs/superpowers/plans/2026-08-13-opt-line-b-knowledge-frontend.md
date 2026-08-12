# 优化线 B · 知识+前端 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复五个用户可感知缺陷（referrers 恒 400、products 过滤失效、领导裁决前端崩溃、锚点口径漏报、知识窗口错位），清理三处退役结构（user.reply.task 种子、execute_step 死路径、chat 死字段）。

**Architecture:** 全部改动限于线 B 文件所有权；参数错配类采用"后端 serde alias 兼容 + 前端改正"双保险；知识窗口修复保持 verified-only 红线零破坏。

**Tech Stack:** Rust/Axum、React 19 + TS、Vitest。

## Global Constraints（每任务隐含）

- 模型：仅 Fable 5 1M max（不派生 subagent）。
- 红线：动手前读懂受影响函数全文并重验计划锚点；锚点失效即停并报告；AI 永不 verify 的 19 处落点（25 号 §3 总表）一处不许弱化；禁词 lint。
- 文件边界：只许修改 `src/routes/knowledge/**`、`src/knowledge_wiki/**`、`src/agent/knowledge_*.rs`、`src/agent/chat_tool_loop.rs`、`src/prompts.rs`、`src/prompt_guard.rs`、`src/routes/products.rs`、`src/routes/principal_escalations.rs`、`src/routes/ask_human_inbox.rs`、`src/knowledge_task/**`、`frontend/**` 与对应 `tests/**`。越界需求登记不执行。
- 每任务收尾：`cargo check --tests`（-D warnings）+ 相关测试绿 + 前端改动跑 `cd frontend && npm test` 相关文件 + 独立 commit。
- 全线收尾：`cargo test --lib` ≥2530/0、四 PBT、前端 `npm run build` + `npm test` 全绿。

---

### Task B1: referrers 恒 400 修复（缺陷 #14）

**Files:**
- Modify: `src/routes/knowledge/wiki_edit.rs`（`ChunkReferrersQuery` :840-844 与注释 :846）、`frontend/src/features/knowledge/shared.tsx`（:933 调用）
- Test: wiki_edit 单测（Query 反序列化两种参数名）；前端相关组件测试更新

**行为契约:** 后端 `target_id` 字段加 `#[serde(alias = "target_id")]`（保持 camelCase 主名 `targetId`）；`:846` 注释改 `?targetId=...`；前端改发 `targetId`。双侧修完后该功能两种写法均可用（兼容历史书签/脚本）。

**Steps:**
- [ ] 重验两侧锚点 → 写 serde 反序列化单测（`targetId`/`target_id` 双通过）→ 实现 → 前端调用改正 + 该组件测试跑绿 → Commit：`fix(knowledge): accept both targetId and target_id on chunk referrers query`

### Task B2: products 过滤静默失效修复（缺陷 #15）

**Files:**
- Modify: `src/routes/products.rs`（`ListQuery` :40-46）、`frontend/src/features/campaign/ProductMultiSelect.tsx`（:15）
- Test: products 单测（双参数名过滤生效）

**行为契约:** `active_only` 加 `#[serde(alias = "active_only")]`；前端改 `activeOnly=true`。修复后 campaign 圈人下拉只见 active 产品。

**Steps:**
- [ ] 重验 → 失败测试（发 `active_only=true` 断言归档产品被滤掉）→ 实现 → 前端改正 → Commit：`fix(products): honor snake_case active_only alias so archived products stay filtered`

### Task B3: 领导裁决前端崩溃修复（缺陷 #17）

**Files:**
- Modify: `src/routes/principal_escalations.rs`、`src/routes/ask_human_inbox.rs`（两处裸 `bson::DateTime` 序列化——按 23 号终裁 12-3 定位 `authorizationExpiresAt` 输出点，重验）
- Modify: `frontend/src/features/ask-human/ResolvedEscalations.tsx`（formatExpiry 防御性兼容）
- Test: 路由响应 JSON 形状单测；前端 formatExpiry 单测

**行为契约:** 后端把 `authorizationExpiresAt`（及同响应内其他裸 DateTime，若有）序列化为**毫秒整数**（对齐 `domain_profiles.rs:521-528` 已修方案——重验该方案的实际形态后逐字对齐）；前端 formatExpiry 同时兼容毫秒数与历史 `{$date:...}` 对象（防旧数据）。

**Steps:**
- [ ] 重验两处输出点与 domain_profiles 已修方案 → 后端失败测试（响应 JSON 断言为 number）→ 实现 → 前端防御 + 单测 → Commit：`fix(escalation): serialize authorization expiry as millis to stop resolved-history crash`

### Task B4: 锚点口径统一（缺陷 #9）

**Files:**
- Modify: `src/routes/knowledge/crud.rs`（:547）、`verify.rs`（:398）、`digest_inbox.rs`（:480）、`catalog.rs`（:209）——四处裸 `!source_anchors.is_empty()` → `crate::models::chunk_has_citable_anchor(&chunk.source_anchors)`（行号动手前重验）
- Test: 各文件相关单测补"畸形锚（有 anchor 但缺 sourceQuote）归类为 source_orphan/不可引用"断言

**Steps:**
- [ ] 逐处重验上下文语义（确认统一后报表分类语义正确）→ 失败测试 → 替换 → Commit：`fix(knowledge): use citable-anchor predicate consistently in reports and review queues`

### Task B5: 知识窗口错位修复（缺陷 #4）

**Files:**
- Modify: `src/agent/knowledge_router.rs`（`cited_in_corpus` :752-762 及其下游 evidence/verified 判定链，重验并读全函数）
- Test: `tests/` 新建或扩展 knowledge_router 集成测试（`#[ignore]`）：201+ 条 verified 场景下 agent 引用第 201 名 chunk 不被降格

**行为契约:**
- 现状：agent `cited_chunk_ids` 与 router 预载的 200 条静态窗求交，窗外合法引用被丢弃 → fallback 降格。
- 目标：cited ids 不再与窗口求交——改为按 id 批量直查 DB 验证 `status=active && integrity_status=verified`（workspace 过滤），验证通过的保留为 selected_chunk_ids（上限 8 与现行为一致）、其 chunk 文档并入下游 evidence/verified 计算所用集合；验证不过的照旧丢弃。**verified-only 红线与 R5.4 判定的输入契约不变**（下游 `compute_verified_chunks` 拿到的集合语义不变，只是不再漏真引用）。
- 性能：一次 `$in` 批查 ≤8 个 id，可忽略。

**Steps:**
- [ ] 读 `route_gateway_knowledge`→`cited_in_corpus`→selected/evidence 下游全链 + `compute_verified_chunks` 的输入来源，画清集合流向后再动手。
- [ ] 失败测试 → 实现 → `cargo test --lib` 相关模块绿 → Commit：`fix(knowledge): verify agent-cited chunks by id instead of intersecting the static corpus window`

### Task B6: knowledge_task execute_step 死路径删除（25 号确认）

**Files:**
- Modify: `src/knowledge_task/mod.rs`（`execute_step` 内与两阶段提交重复的 add_chunk/retag/dismiss 直接路径——重验哪条是活路径：25/08 号结论为两阶段提交是生产路径，直接路径漂移且 dismiss 缺 account 过滤）
- Test: knowledge_task 现有测试全绿（删除不改活路径行为）

**Steps:**
- [ ] 全文读 `execute_step` 与 prepare/commit 两阶段，确认调用封闭与活路径 → 删除死路径 → 测试绿 → Commit：`chore(knowledge-task): remove drifted duplicate step paths superseded by two-phase commit`

### Task B7: user.reply.task 退役清理（偏差 #2 落地）

**Files:**
- Modify: `src/prompts.rs`（prompt_specs 移除 `user.reply.task` spec 与其守护测试；`reseed`/`align` 逻辑对已存在 DB 行的处理——重验 align 对"spec 消失的 key"行为：不得误删运营自建模板，只停止种入与对齐）
- Modify: `src/prompt_guard.rs`（`user.reply.task` 治理面条目与测试收缩）
- Test: prompts.rs / prompt_guard.rs 相关测试同步；`tests/` 中引用该 key 做 fixture 的（budget/run_audit/prompt_template_versions/m043）**不动**（历史 fixture 合法）

**行为契约:** 生产零消费（全量核证）。清理后：种子包不再包含该 key；新 workspace 不再种入；已有 DB 行保留不删（历史数据，align 跳过未知 key 即可——重验 align 现行为，若 align 会删除非 spec key 的行则停下报告）。

**Steps:**
- [ ] 重验 `ensure_prompt_pack_v2`/`align_prompt_specs`/`delete_redundant` 对消失 key 的语义（这是本任务唯一风险点）→ 移除 spec 与治理条目 → 全部 prompts/prompt_guard 测试绿 → Commit：`chore(prompts): retire user.reply.task from seed pack and guard surface (production has zero consumers)`

### Task B8: chat 死字段与响应注释清理

**Files:**
- Modify: `src/routes/knowledge/chat.rs`（更新映射表残留 `safe_claims/forbidden_claims` 等已删字段，约 :2872-2884 重验）、`crud.rs`（PUT 响应注释精确化——25 号已裁决响应正确，仅注释表述）

**Steps:**
- [ ] 重验映射表字段与 chunk 模型现状 → 删除死字段映射 → 测试绿 → Commit：`chore(knowledge): drop dead patch-field mappings and clarify PUT response comment`

### 收尾

- [ ] `cargo test --lib` ≥2530/0、四 PBT、`-D warnings` check、禁词 lint、`cd frontend && npm run build && npm test` 全绿。
- [ ] 交付报告：锚点重验记录、B5 集合流向说明、B7 align 语义核验结论、留 CI 的 ignored 测试、档案回写要点。
