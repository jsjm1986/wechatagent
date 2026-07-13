# 知识 Agent 反接管红线治本（批次1）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让「全 AI 自治·无人工接管」红线在知识层也成立——知识 Agent 不再产出转接话术（堵源头），且其自然语言总结 reason 不再回流进 Reply/Review 的 prompt（切回流）。

**Architecture:** 两层纵深防御。Layer 2（先做，纯函数可 TDD 锁行为）：把知识路由结果的 `reason` 字段从注入 Reply/Review 的 JSON 中剔除，Reply 与 Review 复用**同一个**净化函数（DRY）。Layer 1（后做，prompt 行为）：给知识 Agent 的内联 `SYSTEM_PROMPT` 追加反接管角色约束。Layer 3：影子模拟做泛化验证。

**Tech Stack:** Rust 2021 (Axum)，`cargo test --lib`，serde_json。无新依赖。

## Global Constraints

- **反过拟合红线（最高）**：改 prompt / 措辞只能沉淀可复现的抽象方法论，绝不对本次实测的 5 组样本点对点补丁。绝不加关键词黑名单、绝不对 reason 做关键词 replace。
- **无人工接管 lint**：`scripts/check-no-human-takeover.{sh,ps1}` 扫描 `src/agent/` 等目录 git diff 新增行的禁词（`human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`）。knowledge_agent.rs 受此扫描——新增 prompt 行须用 AI 自治合规表述（「AI 内部研判 / 正式口径 / 机构内部流程」），禁词只在「描述被检索知识里的流程」语境出现；提交前必跑该 lint。
- **基线门**：`cargo test --lib` 不得回归（当前 **1974** passed / 0 failed）；`scripts/check-baseline.{sh,ps1}` 双门绿。
- **子 agent 红线**：任何实现/修复子 agent 必须先 100% 读懂相关代码再改，产出带 file:line 证据，凭假设的产出打回。
- **不做**（YAGNI，留后续批次）：不改知识数据（转接 SOP chunk 口径）、不做红线注入机制架构统一、不动 `/api/knowledge/ask`（sources_meta.rs）、不改 reply/review 串行结构、不动阈值。

---

### Task 1: Layer 2 — 净化函数剔除 reason（Reply + Review 复用同一函数）

**Files:**
- Modify: `src/agent/decision.rs:1248-1259`（`format_knowledge_route_for_prompt` 加剔除 `reason`）
- Modify: `src/agent/decision.rs:1854-1863`（反转旧断言 `format_knowledge_route` 测试）
- Modify: `src/agent/review/mod.rs:368`（改用 `super::decision::format_knowledge_route_for_prompt`，替代裸 `serde_json::to_string`）
- Test: `src/agent/decision.rs`（既有 `#[cfg(test)] mod`，已含 `format_knowledge_route` 测试）

**Interfaces:**
- Consumes: `KnowledgeRouteResult`（types.rs:1340，字段 `reason: String`、`knowledge_coverage`、`missing_knowledge`、`evidence_excerpts`、`selected_chunk_ids` 等）。
- Produces: `pub(crate) fn format_knowledge_route_for_prompt(route: &KnowledgeRouteResult) -> String` —— 行为变更为**额外剔除 `reason`**（camelCase key 就叫 `reason`）。Review 侧改为调用它。

**背景（已亲验）**：
- `format_knowledge_route_for_prompt`（decision.rs:1248）当前 remove `toolTrace`/`evidenceExcerpts`/`selectedChunkRankings` 三个 key，保留 reason。Reply 侧 decision.rs:486 已调用它。
- Review 侧 review/mod.rs:368 是**裸** `serde_json::to_string(knowledge_route)`（连字段都不删），结果在 review/mod.rs:486 拼进 reviewer user prompt。
- review/mod.rs:49 已 `use super::decision::{...}` → 可直接复用净化函数。
- `KnowledgeRouteResult` 的 serde `rename_all = "camelCase"`（types.rs），故 `reason` 字段序列化后 key 仍是 `reason`（单词无大小写变化）。

- [ ] **Step 1: 反转旧断言，写出会失败的测试**

`src/agent/decision.rs` 既有测试（约 1854 行）里，把「保留 reason」断言反转为「剔除 reason」。当前第 1859 行是：
```rust
        assert!(out.contains("命中产品事实切片"), "保留 reason 内容");
```
改为：
```rust
        assert!(!out.contains("命中产品事实切片"), "剔除 reason 内容（防转接话术回流）");
        assert!(!out.contains("\"reason\""), "剔除 reason 字段 key");
```
其余断言（保留 neededCategories/selectedKnowledgeIds/knowledgeCoverage/missingKnowledge、剔除 toolTrace/evidenceExcerpts/selectedChunkRankings）保持不变。

> 注意：该测试构造的 `route` 里 `reason` 字段值含子串「命中产品事实切片」（这是既有测试数据；实现者读 1810-1863 附近确认构造处，若 reason 值不同则用实际值）。

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test --lib format_knowledge_route -- --nocapture`
Expected: FAIL —— 断言 `!out.contains("命中产品事实切片")` 失败，因为当前函数仍保留 reason。

- [ ] **Step 3: 实现——净化函数加剔除 reason**

`src/agent/decision.rs:1253-1257`，在现有三个 remove 后加一行：
```rust
    if let Some(map) = value.as_object_mut() {
        map.remove("toolTrace");
        map.remove("evidenceExcerpts");
        map.remove("selectedChunkRankings");
        map.remove("reason"); // 反接管治本：知识 Agent 的自然语言总结可能含转接话术，不回流进下游 prompt；知识充分度信号由 knowledgeCoverage/missingKnowledge/evidenceExcerpts 等结构化字段承载
    }
```
（同步更新函数上方 doc 注释，把「保留其余 10 个字段」改为「剔除 reason + 3 个调试字段，保留其余结构化字段」，避免注释与实现漂移。）

- [ ] **Step 4: Review 侧改用同一净化函数**

`src/agent/review/mod.rs:368`，把：
```rust
    let knowledge_route_text = serde_json::to_string(knowledge_route).unwrap_or_default();
```
改为：
```rust
    // 反接管治本：复用 reply 侧同一净化函数，剔除 reason（防知识 Agent 转接话术经 reviewer 上下文回流）+ 调试字段，两处口径单一真相源
    let knowledge_route_text = super::decision::format_knowledge_route_for_prompt(knowledge_route);
```
确认 `format_knowledge_route_for_prompt` 是 `pub(crate)`（decision.rs:1248 已是）→ review 模块可见。

- [ ] **Step 5: 运行测试，确认通过 + 无回归**

Run: `cargo test --lib format_knowledge_route -- --nocapture`
Expected: PASS。
Run: `cargo test --lib`
Expected: **1974 passed; 0 failed**（不回归；若既有别的测试断言 review 含 reason 内容，一并按同口径修正并在提交信息注明）。

- [ ] **Step 6: 提交**

```bash
git add src/agent/decision.rs src/agent/review/mod.rs
git commit -m "fix(agent): 知识路由reason不回流进Reply/Review prompt(Layer2/反接管治本)

知识Agent的自然语言总结reason可能含转接话术(实测'我帮您转人工客服'),
format_knowledge_route_for_prompt额外剔除reason;Review侧改用同一净化函数
(替代裸to_string),两处口径单一真相源。知识充分度信号由knowledgeCoverage/
missingKnowledge/evidenceExcerpts结构化字段承载,亲验剔reason不丢信号。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Layer 1 — 知识 Agent SYSTEM_PROMPT 补反接管角色约束

**Files:**
- Modify: `src/agent/knowledge_agent.rs:267-271`（`const SYSTEM_PROMPT` 追加反接管段）

**Interfaces:**
- Consumes: 无（改的是字符串常量）。
- Produces: 无新签名。行为变更：知识 Agent 每次调用的 system prompt 末尾恒含反接管角色约束。

**背景（已亲验）**：
- `const SYSTEM_PROMPT`（knowledge_agent.rs:267-271）当前只讲渐进式披露/带引用 answer/只输出 JSON/最多 4 轮，无一字反接管。它经 `generate_agent_json`（prompt_key `"knowledge.agent"`）直传，**不读 prompts.rs**，故加红线的唯一改点就是这个常量。
- 对比 Reply 红线在 prompts.rs:1147/1149/1172（DB 模板）。知识 Agent 是「wiki 研究员」角色，不是对客户说话的人——约束要校准其**角色认知**，非套用 Reply 的对客话术。

- [ ] **Step 1: 先跑 lint 基线（记录改前状态）**

Run（Windows/bash）：`bash scripts/check-no-human-takeover.sh` 或 `powershell -File scripts/check-no-human-takeover.ps1`
Expected: 记录当前 PASS/FAIL 基线（改前应 PASS，因为未新增禁词行）。

- [ ] **Step 2: 追加反接管角色约束到 SYSTEM_PROMPT**

`src/agent/knowledge_agent.rs:267-271`，在常量末尾（`最后一轮必须 answer。` 之后）追加。措辞须过 no-human-takeover lint——用「AI 内部研判 / 正式口径」等合规词，避免新增行出现裸禁词：
```rust
const SYSTEM_PROMPT: &str = "你是运营知识库的 wiki 研究员。\n\
你必须按 skills 的渐进式披露模式工作：先读文档目录（每份文档的 catalogSummary / routingMap 是给你导航的索引），判断哪份文档相关后 open_document 下钻到它的原子摘要，再选择性地 open_chunk 展开完整正文，最后给出带引用的 answer。\n\
你不能凭空回答；任何回答都必须来自被你 open 过的 chunk。\n\
你只输出严格 JSON。每轮只能输出 5 个 action 之一：list_catalog / open_document / open_chunk / follow_relations / answer。\n\
最多 4 轮工具调用。最后一轮必须 answer。\n\
你的 answer 是给系统内部回复 Agent 的**知识研判**，不是发给客户的话术，也不是可执行的对客行动脚本。本系统定位是 AI 全程自治：客户始终只与 AI 对话。若被检索的知识内容描述了机构内部的流程分工（例如某类事项需按正式政策核对、由内部相应岗位确认），你可以如实转述该知识所界定的**事实边界**（如「此项以正式政策/当期正式口径为准」），但绝不把这类内部流程改写成对客户的行动建议话术。凡超出当前已 open 知识能确定的事项，统一研判为「需按正式口径核对后确认」，由内部回复 Agent 决定如何向客户表达。";
```
> 措辞要点：约束「answer 的角色是内部研判、不是对客话术」这一**普适方法论**（任何行业知识库都成立），而非列禁词。实现时若 lint 仍报某词，微调该词表述（如把可能触发的字面换成语义等价的合规表述），不得改动约束语义。

- [ ] **Step 3: 跑 lint 确认新增行不触发禁词**

Run: `bash scripts/check-no-human-takeover.sh`（或 .ps1）
Expected: PASS。若 FAIL，读报错命中的行与词，把该词改为语义等价的合规表述（如「相应岗位」代替触发词），重跑至 PASS。**不得为过 lint 而删弱约束语义**。

- [ ] **Step 4: 编译确认无语法错误**

Run: `cargo check --lib`
Expected: 0 error（字符串常量改动，不涉类型）。

- [ ] **Step 5: 基线不回归**

Run: `cargo test --lib`
Expected: **1974 passed; 0 failed**（prompt 常量改动不影响单测；知识 Agent 行为在 Task 3 用影子模拟验证）。

- [ ] **Step 6: 提交**

```bash
git add src/agent/knowledge_agent.rs
git commit -m "fix(agent): 知识Agent SYSTEM_PROMPT补反接管角色约束(Layer1/反接管治本)

知识Agent内联SYSTEM_PROMPT(不读prompts.rs)是反接管红线盲区,忠实复现知识库
传统客服转接SOP。追加约束校准角色认知:answer是给内部回复Agent的知识研判、
非对客话术,内部流程分工只转述事实边界不改写成对客行动建议。语义约束(普适
方法论)非关键词黑名单,守反过拟合红线。过no-human-takeover lint。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Layer 3 — 影子模拟泛化验证（不改代码，回归验证）

**Files:**
- 无代码改动。产出：验证记录（可写入会话/临时文件，不入库）。

**Interfaces:**
- Consumes: 已部署到生产 117 的 Task 1+2 构建（或本地起服务）。影子端点 `POST /api/user-operations/simulations/dialogue`，body `{accountId, contactId, messages[]}`，走 `simulate_user_dialogue`（不真发客户）。
- Produces: 验证结论——`knowledgeRoute.reason` 不再含转接话术，且 `knowledge_coverage`/`missing_knowledge` 不退化。

**背景**：Layer 1 是 prompt 行为，无法用单测锁，靠影子模拟做**泛化验证**（非调绿）。本任务在 Task 1+2 合并部署后执行。

- [ ] **Step 1: 重跑实测那 5 组 + 新增泛化变体**

对 account 102 的 managed contact（如 Demi，contactId `6a4f5c379d28a161324c2dd1`，非 decider），逐组调影子端点：
- 原 5 组：价格优惠团购 / 直接要真人客服 / 机构地址 / 满意后付款 / 投诉要负责人。
- 新增 3-4 组泛化变体（不同触发点/措辞）：如「你们这能不能先做后付」「我想找个能拍板的人」「这个效果不满意找谁负责」。

复用 `scripts/e2e/wiki_verify_common.py::attach`（CDP 复用登录态）+ 页面内 fetch 的范式，结果写 UTF-8 JSON。慢端点串行、后台跑防超时。

- [ ] **Step 2: 核对 reason 不再含转接话术**

逐组检查返回的 `items[].knowledgeRoute.reason`：
Expected: 不再出现「转人工客服 / 转主管 / 让 ta 对接 / 帮您转接」这类对客行动话术；涉及超权事项表述为「按正式口径核对后确认」类内部研判口吻。

- [ ] **Step 3: 核对知识充分度信号不退化 + 客户话术仍守红线**

Expected:
- `knowledge_coverage` / `missing_knowledge` 与改前同源（不因剔 reason 而变差）。
- `decision.replyText`（对客话术）仍无转接措辞、第一人称承接（改前本就守住，确认不回归）。

- [ ] **Step 4: 记录验证结论**

把每组「输入 / reason 摘要 / replyText 摘要 / coverage」整理成对照结论。若发现任何一组 reason 仍漏转接话术：**不改措辞去调绿**，而是回到 Task 2 分析措辞的抽象层缺陷（是否角色约束不够普适），改抽象层后重跑全部变体验证泛化。

---

## 自审记录（Self-Review）

- **Spec 覆盖**：Layer 1（Task 2）✓、Layer 2（Task 1，含 Reply+Review 两改点用同一函数）✓、测试（Task 1 单测 + Task 3 泛化验证 + 基线门）✓、反过拟合（Global Constraints + Task 2/3 显式守则）✓、lint（Task 2 Step 1/3）✓、不做边界（Global Constraints）✓。
- **类型一致**：`format_knowledge_route_for_prompt(&KnowledgeRouteResult) -> String` 在 Task 1 定义/复用，Review 侧调用签名一致；`reason` camelCase key 确认为 `reason`。
- **无占位符**：所有代码步给出完整代码；lint/测试给出确切命令与期望。
- **顺序合理**：Task 1（纯函数锁行为，可独立测）→ Task 2（prompt，lint 联调）→ Task 3（部署后泛化验证）。Task 1、2 可独立提交、独立 review；Task 3 依赖前两者部署。
