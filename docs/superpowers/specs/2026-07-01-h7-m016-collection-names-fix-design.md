# H7 m016 集合名/字段名一致性修复设计

> 日期：2026-07-01
> 分支：`fix/h7-m016-collection-names`（off origin/main f1f4f1c，含 H8）
> 来源：终极审判审计 H7 项（m016 collection names）
> 前置核实：worktree HEAD = origin/main = f1f4f1c；权威清单由主控逐集合亲验定稿（两轮 Explore subagent 核查均不可靠，见 §7）。

## 1. 漏洞描述

`src/db/migrations/m016_backfill_workspace_id_on_legacy_rows.rs` 是多租户（P1-1 workspace 联邦）上线前的关键数据修复：single-tenant 时期的 legacy 业务行多数没写 `workspace_id` 字段，多租户过滤上线后会被无差别黑掉（不匹配任何租户 → 数据不可见）。m016 扫业务集合，把 `workspace_id`（或 camelCase `workspaceId`）缺失的行 `$set` 为 `DEFAULT_WORKSPACE_ID`。

它用**两张硬编码集合名表**：`SNAKE_CASE_COLLECTIONS`（m016:26-67，40 项，回填 snake `workspace_id`）+ `CAMEL_CASE_COLLECTIONS`（m016:70-73，回填 camel `workspaceId`）。

缺陷：这两张表与真实 `Database` typed accessor（`src/db/mod.rs`）及各 model 的真实 BSON 字段名**不一致**，导致三类静默失效：

1. **集合名拼错**：m016 的名字在真实集合里不存在 → `update_many` 匹配 0 行（Mongo 对不存在集合不报错），该真实集合的 legacy 行**永不被回填**。
2. **集合漏收录**：有 `workspace_id` 字段、却两张表都没列的集合 → legacy 行永不被回填。
3. **snake/camel 归错类**：集合真实字段是 camelCase 却放进 SNAKE 表（或反之）→ `$set` 写错字段名，legacy 行仍缺正确字段。

后果统一：多租户过滤上线后，未正确回填的 legacy 行 workspace_id 缺失 → 数据不可见/丢失。这是数据完整性缺陷。

### 触发条件

m016 有 `APP_ENV=production` 守卫（m016:76-82）：生产环境 noop，要求运维显式 backfill。所以缺陷在**非生产环境自动 backfill 时**静默发生，或**生产运维照 m016 逻辑手工 backfill 时**照样漏。任一路径下，受影响集合的 legacy 行都没被正确回填。

## 2. 权威集合清单（主控逐集合亲验定稿）

判定标准：集合的真实 BSON 字段名 = 看 model struct 的 `#[serde(rename_all)]`（有 camelCase → `workspace_id` 序列化成 `workspaceId`）+ 字段级 `rename`；租户字段是否为单值 `workspace_id`（`workspaces: Vec` 数组 / `scope` 不算）。

### A. 集合名拼错（7 处，m016 SNAKE 名 → 真实 accessor 名）

| m016 名（行） | 真实 accessor 名（mod.rs 行） |
| --- | --- |
| `accounts`（27） | `wechat_accounts`（65） |
| `decision_reviews`（45） | `agent_decision_reviews`（176） |
| `management_sessions`（50） | `management_agent_sessions`（208） |
| `management_messages`（51） | `management_agent_messages`（212） |
| `command_runs`（52） | `agent_command_runs`（216） |
| `tool_calls`（53） | `agent_tool_calls`（220） |
| `outcome_metrics`（54） | `agent_outcome_metrics`（224） |

### B. 漏掉的集合（15 个，有单值 workspace_id、m016 未收录）

- **snake（13，补进 SNAKE 表）**：`behavior_signals`(657)、`behavior_signal_metrics`(716)、`mcp_call_logs`(932)、`referral_cards`(1130)、`agent_send_ledger`(1159)、`ingest_sources`(1678)、`domain_profiles`(1794)、`agent_send_outbox`(2783)、`relationship_type_suggestions`(2929)、`suspected_deal_signals`(2963)、`agent_principal_escalations`(3469)、`knowledge_chat_tasks`(4683)、`products`(502)
- **camel（2，补进 CAMEL 表）**：`campaigns`（Campaign:552 `rename_all=camelCase` → workspaceId）、`campaign_sends`（CampaignSend:596 `rename_all=camelCase` → workspaceId）

（括号为 models.rs 里 `pub workspace_id` 字段行号；camel 两个的 rename_all 在 struct 头行 552/596。）

### C. 归错类 / 该移除

- `admin_users`（m016:72 CAMEL 表）→ **移除**：`AdminUser`（src/auth/mod.rs:28-39）租户模型是 `workspaces: Vec<String>` + `default_workspace: Option<String>`，**无单值 workspace_id/workspaceId 字段**，不符合"回填单值 workspace_id"的契约。它由 `auth/session.rs:35` 的 `db.raw().collection("admin_users")` 访问（不走 typed accessor），是真实集合但租户模型不同。
- `llm_provider_configs`（m016:71 CAMEL 表）→ **正确保留**：`LlmProviderConfig`（models.rs:4731-4736）有 `#[serde(rename_all="camelCase")]`（4732），字段 `workspace_id` 序列化成 `workspaceId`，放 CAMEL 表正确。

### D. 确认排除（用户已确认，不入回填范围）

- `system_taxonomies`（`TaxonomyEntry` models.rs:2828）、`taxonomy_candidates`（`TaxonomyCandidate` models.rs:2892）：用 `pub scope: String`（2832/2895，值为 `"global"` 或 account_id）隔离，**无 workspace_id 字段**，不是 workspace 租户维度。
- `knowledge_chat_session_seqs`（mod.rs:159）：`Collection<Document>`，原子自增计数器，无 model、无 workspace_id。
- `migrations`：迁移记账系统表。

### E. SNAKE 表 33 个有效项（除 7 拼错）已亲验全 snake（无 rename_all），归类正确

WechatAccount / Contact / ConversationMessage / AgentTask / AgentEvent / ContentAsset / AgentSoul / OperationPlaybook / OperationDomainConfig / OperationStatePolicy / PromptTemplate / OperatingMemory / OperationKnowledgeDocument / OperationKnowledgeChunk / KnowledgeUsageLog / KnowledgeChatTurn / KnowledgeDailyReport / KnowledgeOperatorMemory / AgentRunLog / LlmCallLog / MemoryCandidate / UserOperationGuidePreview / EvaluationScenario / Experiment / Proposal / ShadowReplay / ThresholdOverride / ThresholdOverrideAudit / PostReleaseReview / EvolutionRuntimeFlag / ChunkRevision / KnowledgeGapSignal / DomainSchema / CatalogRebuildJob。

## 3. 方案选型

### 方案 A（选定）：修正硬编码表 + 加一致性单测防复发

1. **修 SNAKE 表**：改对 7 个拼错名（→真实 accessor 名），补进 13 个漏掉的 snake 集合。
2. **修 CAMEL 表**：移除 `admin_users`，补进 `campaigns` / `campaign_sends`。
3. **加 lib 单测**：维护一份"已知真实集合名"基准，断言 m016 两张表里的每个名字都在基准内（挡拼错复发）+ 两表无重复、无空串。

**为什么选 A：** 贴近现有形态、改动可控、止血直接。一致性单测把"集合名拼错"这类最隐蔽的复发挡在 lib 层（不需 Docker，进 `cargo test --lib`）。

**否决方案 B（改成从 typed accessor 自动派生集合名单一来源）：** 项目无集中集合注册表，accessor 是 62 个散落的手写 `self.db.collection("字面量")` fn，Rust 无反射，自动枚举要么靠宏重构整个 db 层、要么维护另一份手写映射——改动面远超 H7 该有的范围，且引入新抽象层。否决。

**否决方案 C（只修表不加单测）：** 止血但同类 bug（新增集合忘了加、又拼错）必然复发。用户已明确要防复发。否决。

### 一致性单测的固有局限（诚实声明）

单测能挡"拼错"（名字不在已知真实集合基准内即 fail），但**挡不住"漏收录"**（表里没有的集合，单测无从知道它本该在）。因为无集中注册表，"哪些集合有 workspace_id"这份知识本身要手维。本方案接受这个局限：单测的"已知真实集合名基准"手维一份，至少让拼错无处遁形；漏收录靠本次 spec 的权威清单一次补全 + 未来 code review。若未来要根治漏收录，需方案 B 级别的注册表重构，超出 H7。

## 4. 核心改动

落点：`src/db/migrations/m016_backfill_workspace_id_on_legacy_rows.rs`。

### 4.1 SNAKE_CASE_COLLECTIONS

- 改 7 个拼错：`accounts`→`wechat_accounts`、`decision_reviews`→`agent_decision_reviews`、`management_sessions`→`management_agent_sessions`、`management_messages`→`management_agent_messages`、`command_runs`→`agent_command_runs`、`tool_calls`→`agent_tool_calls`、`outcome_metrics`→`agent_outcome_metrics`
- 补 13 个 snake 漏掉集合（B 类 snake 组）。

### 4.2 CAMEL_CASE_COLLECTIONS

- 移除 `admin_users`。
- 补 `campaigns`、`campaign_sends`。
- 保留 `llm_provider_configs`。

### 4.3 一致性单测（同文件 `#[cfg(test)] mod tests`，lib 单测）

- 维护 `const KNOWN_TENANT_COLLECTIONS: &[&str]`（本 spec §2 定稿的全部真实集合名，snake + camel 合并）。
- 测试 1：`SNAKE_CASE_COLLECTIONS` ∪ `CAMEL_CASE_COLLECTIONS` 的每个名字都 ∈ `KNOWN_TENANT_COLLECTIONS`（挡拼错）。
- 测试 2：两表内部 + 跨表无重复项、无空串。
- 测试 3：`admin_users` 不在任一表内（锁 C 类移除，防回退）。

## 5. 数据行为（改动后）

- 非生产环境启动跑 migrations：m016 对**正确的**集合名、**正确的**字段名回填缺失 workspace_id/workspaceId。之前拼错/漏掉的集合现在被覆盖。
- 幂等不变：仍只改 `$exists: false` 的文档（m016:90/108），二次执行 matched=0。
- 生产守卫不变（m016:76-82）：`APP_ENV=production` 仍 noop，运维照修正后的表手工 backfill。
- m016 已入账的环境（migration 记录已存在）：**migration 不会重跑**（mod.rs run_with 幂等按 id 记账）。这意味着**已经跑过旧 m016 的环境，legacy 行不会被自动补救**——见 §6 风险。

## 6. 风险与边界

- **已跑过旧 m016 的环境**：migration 按 id 记账，改了 m016 内容它也不重跑，旧环境漏回填的 legacy 行不会自动补。但：(a) 生产有 APP_ENV 守卫，m016 在生产本就 noop（靠手工 backfill，运维用修正后的表）；(b) 非生产是测试/开发库，重建无损。故不需要新起一个 m016b migration 强制重跑——**本次修复保证"从此以后"m016 正确**，历史环境由运维手工 backfill 兜底。若用户要求补救已跑环境，另开 migration（超出 H7）。
- **不做（YAGNI）**：不改成自动派生（方案 B）；不动 m016 的 update_many/幂等/生产守卫逻辑；不碰 D 类排除集合；不新起补救 migration。
- **过拟合红线**：一致性单测锁的是"m016 表名 ∈ 真实集合名"这一真实不变量，不为调绿改任何业务逻辑。

## 7. 附：为何权威清单由主控亲验（方法论记录）

两轮 Explore subagent 核查这份清单均不可靠且互相矛盾：第一轮报 7 处拼错、第二轮主表否认其中 6 处（把真实名误当 m016 内容）；第二轮把 `system_taxonomies` 说成有 workspace_id（实为 `scope`，把注释行张冠李戴）；漏看 struct 头 `rename_all` 导致 llm_provider_configs 归类误判。故 §2 清单由主控逐集合 Read struct + 对照 m016 表原文 + db/mod.rs accessor 亲验定稿，每条附 file:line。实现阶段（SDD）subagent 同样必须先读码验证每个集合名再改，产出带 file:line 证据。
