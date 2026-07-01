# H7 m016 集合名/字段名一致性修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正 `m016_backfill_workspace_id_on_legacy_rows.rs` 里两张硬编码集合名表(7 处拼错真实 accessor 名、漏收录 15 个集合、误收录 2 个无单值 workspace_id 的集合),并加 lib 一致性单测防复发,让多租户上线前 legacy 行的 workspace_id 回填对正确的集合、写正确的字段名。

**Architecture:** m016 用 `SNAKE_CASE_COLLECTIONS` / `CAMEL_CASE_COLLECTIONS` 两张 `&[&str]` 硬编码表驱动 `update_many({workspace_id/workspaceId:{$exists:false}}, $set default_ws)`。修复=把两张表改成与真实 `Database` accessor 集合名 + 各 model 真实 BSON 字段名(snake vs camel 取决于 struct 头 `#[serde(rename_all)]`)一致,并加一份按 case 拆分的"审计定稿真实集合名基准"const + 4 个纯 lib 单测交叉锁死(挡拼错 + 挡 snake/camel 归错类 + 挡误收录无 ws 字段的集合)。无集中集合注册表,故基准手维(spec §3 已论证否决自动派生方案 B)。

**Tech Stack:** Rust 2021 / MongoDB(mongodb 2.8)/ migration 框架(`src/db/migrations/`)/ `#[cfg(test)]` lib 单测(`std::collections::HashSet`,无需 Docker)。

## Global Constraints

- 分支:`fix/h7-m016-collection-names`(已从 origin/main f1f4f1c 切;spec commit 26c88c2 + a81b405 已在其上)。绝不 push main,只在 worktree `E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/e4-f21-closure` 干活,不碰主仓根目录。
- cargo 命令前:`export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"` + `export CARGO_INCREMENTAL=0`(worktree 共享 target,不设会 clobber test binary)。磁盘紧先删 `target/debug/incremental`。
- 基线不回归:`cargo test --lib` ≥ 350 passed / 0 failed;4 PBT 文件(state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter)累计 ≥ 33 passed / 0 failed。本任务新增的一致性单测是 **lib 单测**(进 `cargo test --lib` 计数),必须在 commit 时全绿——**绝不提交红测**(违反基线门)。
- 本地只跑 `cargo test --lib`;绝不本地全量 `cargo test`(磁盘 os error 112)。本任务不新增集成测试,无需 Docker。
- 过拟合红线:绝不为过测试改业务逻辑。本任务改的是 migration 的集合名表(数据完整性 bug)+ 加一致性单测。单测锁的是"m016 表名 ∈ 审计定稿真实集合名基准 + 无 ws 字段集合绝不入表"这一**真实不变量**,基准由 models.rs / auth/mod.rs 逐 struct 亲验定稿(spec §2,每条附 file:line),不是把表抄一份对自己(那才是 tautology)。
- 禁词 lint:m016 文件改动不涉禁词(人工/接管/takeover/hand-off),无风险。
- commit:具名 `git add src/db/migrations/m016_backfill_workspace_id_on_legacy_rows.rs`,绝不 `-A`/`.`;commit 消息 `Co-Authored-By: Claude <noreply@anthropic.com>` 结尾。已授权 commit/push/PR/cron 监控 CI/squash 合并。
- **subagent 红线(用户 2026-07-01 点名强调):** 实现时遇到任何不理解的地方,先自己 Read/Grep 读代码、亲自验证,再执行——绝不基于猜测动手。产出必须带 file:line 证据。尤其:每个要写进/删出表的集合名,都要能对照 `src/db/mod.rs` 的 accessor 字面量 + `src/models.rs` 对应 struct 的 `workspace_id` 字段与 struct 头 `rename_all` 确认;用**机械穷举**(字段声明行 vs struct 边界行 vs rename_all 命中行做区间归属)复核,不接受"我读了都对"式抽样结论(spec §7 教训:首轮抽样亲读漏了 chunk_revisions)。

---

## 权威集合清单(实现必须逐条核对,来自 spec §2 亲验定稿)

**§2.A 7 处拼错(m016 SNAKE 名 → 真实 accessor 名,src/db/mod.rs 行号):**
`accounts`→`wechat_accounts`(:65)、`decision_reviews`→`agent_decision_reviews`(:176)、`management_sessions`→`management_agent_sessions`(:208)、`management_messages`→`management_agent_messages`(:212)、`command_runs`→`agent_command_runs`(:216)、`tool_calls`→`agent_tool_calls`(:220)、`outcome_metrics`→`agent_outcome_metrics`(:224)。

**§2.B 漏收录 15 个(有单值 workspace_id、m016 未收录):**
- snake 13(补进 SNAKE 表):`behavior_signals`、`behavior_signal_metrics`、`mcp_call_logs`、`referral_cards`、`agent_send_ledger`、`ingest_sources`、`domain_profiles`、`agent_send_outbox`、`relationship_type_suggestions`、`suspected_deal_signals`、`agent_principal_escalations`、`knowledge_chat_tasks`、`products`
- camel 2(补进 CAMEL 表,struct 头有 `#[serde(rename_all="camelCase")]`):`campaigns`(Campaign models.rs:552/556)、`campaign_sends`(CampaignSend models.rs:596/600)

**§2.C 该移除(无单值 workspace_id 字段):**
- `admin_users`(m016 CAMEL 表)→ 移除:`AdminUser`(auth/mod.rs:28-39)用 `workspaces:Vec<String>` + `default_workspace`,无单值 ws 字段。
- `chunk_revisions`(m016 SNAKE 表)→ 移除:`ChunkRevision`(models.rs:1613-1632)无 workspace_id 字段,靠 chunk_id 反查租户(索引 indexes.rs:1306-1331 无 ws 索引;读路径全按 chunk_id 查)。

**§2.C 保留:** `llm_provider_configs`(CAMEL 表)正确——`LlmProviderConfig`(models.rs:4732 `rename_all="camelCase"`)workspaceId。

**§2.D 确认排除(不入任何表):** `system_taxonomies`/`taxonomy_candidates`(用 `scope` 非 ws)、`knowledge_chat_session_seqs`(`Collection<Document>` 计数器)、`migrations`(记账表)。

---

## 文件结构

- **Modify:** `src/db/migrations/m016_backfill_workspace_id_on_legacy_rows.rs` — 改 `SNAKE_CASE_COLLECTIONS`(26-67)+ `CAMEL_CASE_COLLECTIONS`(70-73)两张表 + 新增文件末 `#[cfg(test)] mod tests`(审计基准 const + 4 个单测)。仅此一个文件。

单任务:修表 + 加一致性单测是对同一文件同一不变量的一次内聚改动(单测定义了表的正确性,二者不可分割评审)。TDD 在任务内完成:先写单测看它红(现表有拼错 + admin_users + chunk_revisions),再修两张表看它绿,一次 commit(全绿,不留红测污染基线)。

---

## Task 1: 修正 m016 两张集合名表 + 加 lib 一致性单测

**Files:**
- Modify: `src/db/migrations/m016_backfill_workspace_id_on_legacy_rows.rs`

**Interfaces:**
- Consumes: 模块级私有 const `SNAKE_CASE_COLLECTIONS: &[&str]`(m016:26)、`CAMEL_CASE_COLLECTIONS: &[&str]`(m016:70)。`run_step` 逻辑(75-123)**不改**,只改这两张表的内容。
- Produces: 无对外新接口。新增 `#[cfg(test)] mod tests` 内的 const(`KNOWN_SNAKE_TENANT_COLLECTIONS` / `KNOWN_CAMEL_TENANT_COLLECTIONS` / `MUST_NOT_BACKFILL`)与 4 个 `#[test]` 仅测试可见。

- [ ] **Step 1: 动手前先读码验证(机械穷举复核,不猜)**

先 Read `src/db/migrations/m016_backfill_workspace_id_on_legacy_rows.rs` 全文,确认 `SNAKE_CASE_COLLECTIONS`(26-67 当前 40 项)、`CAMEL_CASE_COLLECTIONS`(70-73 当前 2 项 = `llm_provider_configs` / `admin_users`)、`run_step` 用 `$exists:false` + `$set` 的逻辑与本计划一致。

再对上面「权威集合清单」逐条亲验(带 file:line 证据),用机械穷举而非抽样:
- 对 SNAKE 表最终应含的每个名字,Grep `src/db/mod.rs` 确认存在 `self.db.collection("<名字>")` 的 accessor;Grep `src/models.rs` 确认对应 struct 有 `pub workspace_id: String` 且 struct 头**无** `#[serde(rename_all)]`(snake)。
- 对 CAMEL 表最终应含的每个名字,确认对应 struct 头**有** `#[serde(rename_all="camelCase")]`(camel)。
- 对 `admin_users` / `chunk_revisions`,确认其 model 无单值 workspace_id 字段(AdminUser 用 `workspaces:Vec`;ChunkRevision 无 ws 字段)。
若任一条与本计划不符,停下核对,以读到的真实代码为准修正,并在 report 里记明分歧。

- [ ] **Step 2: 先写失败的一致性单测(TDD 红)**

用 Edit 在文件**末尾**(第 124 行 `}` 之后,即 `run_step` 函数闭合之后的文件结尾)追加下面整个测试模块。这些 const 的名字来自审计定稿(models.rs / auth/mod.rs 逐 struct 核对),**不是**从上方两张表抄来——它们是"允许被回填 X-case workspace_id 的真实集合宇宙",两张表是"实际选择回填的目标",目标必须 ⊆ 宇宙。

```rust

#[cfg(test)]
mod tests {
    use super::{CAMEL_CASE_COLLECTIONS, SNAKE_CASE_COLLECTIONS};
    use std::collections::HashSet;

    /// 审计定稿:真实携带**单值 snake_case `workspace_id`** 字段的 Mongo 集合名全集。
    /// 来源=对 `src/models.rs` 各 struct 的逐一核对(spec §2,每条附 file:line):
    /// struct 有 `pub workspace_id: String` 且 struct 头无 `#[serde(rename_all)]`。
    /// **不是**从下方 `SNAKE_CASE_COLLECTIONS` 抄来的——这是"允许被回填 snake workspace_id
    /// 的集合宇宙",下方表是"m016 实际回填目标",目标必须是本宇宙的子集。
    /// 新增真实 snake 租户集合时,先在这里登记(带 file:line),再决定是否加入回填表。
    const KNOWN_SNAKE_TENANT_COLLECTIONS: &[&str] = &[
        "wechat_accounts",
        "contacts",
        "conversation_messages",
        "agent_tasks",
        "agent_events",
        "content_assets",
        "agent_souls",
        "operation_playbooks",
        "operation_domain_configs",
        "operation_state_policies",
        "prompt_templates",
        "operating_memories",
        "operation_knowledge_documents",
        "operation_knowledge_chunks",
        "knowledge_usage_logs",
        "knowledge_chat_turns",
        "knowledge_daily_reports",
        "knowledge_operator_memory",
        "agent_decision_reviews",
        "agent_run_logs",
        "llm_call_logs",
        "memory_candidates",
        "user_operation_guide_previews",
        "management_agent_sessions",
        "management_agent_messages",
        "agent_command_runs",
        "agent_tool_calls",
        "agent_outcome_metrics",
        "evaluation_scenarios",
        "experiments",
        "proposals",
        "shadow_replays",
        "threshold_overrides",
        "threshold_overrides_audit",
        "post_release_reviews",
        "evolution_runtime_flags",
        "knowledge_gap_signals",
        "domain_schemas",
        "catalog_rebuild_jobs",
        "behavior_signals",
        "behavior_signal_metrics",
        "mcp_call_logs",
        "referral_cards",
        "agent_send_ledger",
        "ingest_sources",
        "domain_profiles",
        "agent_send_outbox",
        "relationship_type_suggestions",
        "suspected_deal_signals",
        "agent_principal_escalations",
        "knowledge_chat_tasks",
        "products",
    ];

    /// 审计定稿:真实携带**单值 camelCase `workspaceId`** 字段的集合。
    /// 判据=对应 struct 头带 `#[serde(rename_all="camelCase")]`(spec §2):
    /// LlmProviderConfig(models.rs:4732)/ Campaign(552)/ CampaignSend(596)。
    const KNOWN_CAMEL_TENANT_COLLECTIONS: &[&str] = &[
        "llm_provider_configs",
        "campaigns",
        "campaign_sends",
    ];

    /// 无单值 workspace_id 字段、绝不该进任一回填表(防回退,spec §2.C):
    /// `admin_users` 用 `workspaces:Vec<String>`(auth/mod.rs:28-39);
    /// `chunk_revisions` 无 ws 字段、靠 chunk_id 反查租户(models.rs:1613-1632)。
    const MUST_NOT_BACKFILL: &[&str] = &["admin_users", "chunk_revisions"];

    /// 挡拼错 + 挡 snake/camel 归错类:SNAKE 表每个名字都必须 ∈ snake 审计全集。
    /// (拼错真实名 → 不在全集;或该集合其实是 camelCase → 也不在 snake 全集而在 camel 全集。)
    #[test]
    fn snake_table_names_are_all_known_snake_tenant_collections() {
        let known: HashSet<&str> = KNOWN_SNAKE_TENANT_COLLECTIONS.iter().copied().collect();
        for name in SNAKE_CASE_COLLECTIONS {
            assert!(
                known.contains(name),
                "SNAKE_CASE_COLLECTIONS 含 `{name}`,但它不在 KNOWN_SNAKE_TENANT_COLLECTIONS 审计全集内\
                 (要么拼错了真实集合名,要么该集合其实是 camelCase workspaceId 应进 CAMEL 表)"
            );
        }
    }

    /// 挡拼错 + 挡 camel/snake 归错类:CAMEL 表每个名字都必须 ∈ camel 审计全集。
    #[test]
    fn camel_table_names_are_all_known_camel_tenant_collections() {
        let known: HashSet<&str> = KNOWN_CAMEL_TENANT_COLLECTIONS.iter().copied().collect();
        for name in CAMEL_CASE_COLLECTIONS {
            assert!(
                known.contains(name),
                "CAMEL_CASE_COLLECTIONS 含 `{name}`,但它不在 KNOWN_CAMEL_TENANT_COLLECTIONS 审计全集内\
                 (要么拼错,要么该集合其实是 snake workspace_id 应进 SNAKE 表)"
            );
        }
    }

    /// 无空串、无重复、两表不相交(同一集合不会被回填两次/写两种字段名)。
    #[test]
    fn tables_have_no_empty_no_duplicates_and_are_disjoint() {
        let mut seen: HashSet<&str> = HashSet::new();
        for name in SNAKE_CASE_COLLECTIONS
            .iter()
            .chain(CAMEL_CASE_COLLECTIONS.iter())
        {
            assert!(!name.is_empty(), "集合名表含空串");
            assert!(
                seen.insert(name),
                "集合名 `{name}` 在 m016 两张表里重复出现(跨表或表内)"
            );
        }
    }

    /// 防回退:无单值 workspace_id 的集合绝不该出现在任一回填表里。
    #[test]
    fn collections_without_single_workspace_id_are_never_backfilled() {
        let snake: HashSet<&str> = SNAKE_CASE_COLLECTIONS.iter().copied().collect();
        let camel: HashSet<&str> = CAMEL_CASE_COLLECTIONS.iter().copied().collect();
        for name in MUST_NOT_BACKFILL {
            assert!(
                !snake.contains(name),
                "`{name}` 无单值 workspace_id,不该在 SNAKE_CASE_COLLECTIONS"
            );
            assert!(
                !camel.contains(name),
                "`{name}` 无单值 workspace_id,不该在 CAMEL_CASE_COLLECTIONS"
            );
        }
    }
}
```

- [ ] **Step 3: 跑单测确认它红(TDD 红,证明是真护栏)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib m016 2>&1 | tail -30`
Expected: **FAIL**。现表未修,预期 3 个测试红:
- `snake_table_names_are_all_known_snake_tenant_collections`:现 SNAKE 表含 `accounts`/`decision_reviews`/`management_sessions`/`management_messages`/`command_runs`/`tool_calls`/`outcome_metrics`(拼错名不在 snake 全集)+ `chunk_revisions`(已从全集移除)→ assert 失败。
- `camel_table_names_are_all_known_camel_tenant_collections`:现 CAMEL 表含 `admin_users`(不在 camel 全集)→ 失败。
- `collections_without_single_workspace_id_are_never_backfilled`:现 SNAKE 表含 `chunk_revisions`、CAMEL 表含 `admin_users` → 失败。
- `tables_have_no_empty_no_duplicates_and_are_disjoint`:现表无空串/重复,预期此条**先绿**(修表后仍应绿)。

若单测编译不过(如 const 名笔误、HashSet import 缺失),先修编译再看红。看到上述红即证明护栏有效,进 Step 4。

- [ ] **Step 4: 修 SNAKE_CASE_COLLECTIONS(TDD 绿·其一)**

用 Edit 把 `SNAKE_CASE_COLLECTIONS`(当前 26-67)整块替换为下面最终内容。改动=7 个拼错名改对 + 移除 `chunk_revisions` + 补 13 个 snake 漏收录集合(顺序:先原有有效项按原顺序、拼错处就地改名,再在末尾追加 13 个新集合,便于 review diff)。

old_string(当前整块 const,含 `accounts`/`decision_reviews` 等拼错名与 `chunk_revisions`):
```rust
const SNAKE_CASE_COLLECTIONS: &[&str] = &[
    "accounts",
    "contacts",
    "conversation_messages",
    "agent_tasks",
    "agent_events",
    "content_assets",
    "agent_souls",
    "operation_playbooks",
    "operation_domain_configs",
    "operation_state_policies",
    "prompt_templates",
    "operating_memories",
    "operation_knowledge_documents",
    "operation_knowledge_chunks",
    "knowledge_usage_logs",
    "knowledge_chat_turns",
    "knowledge_daily_reports",
    "knowledge_operator_memory",
    "decision_reviews",
    "agent_run_logs",
    "llm_call_logs",
    "memory_candidates",
    "user_operation_guide_previews",
    "management_sessions",
    "management_messages",
    "command_runs",
    "tool_calls",
    "outcome_metrics",
    "evaluation_scenarios",
    "experiments",
    "proposals",
    "shadow_replays",
    "threshold_overrides",
    "threshold_overrides_audit",
    "post_release_reviews",
    "evolution_runtime_flags",
    "chunk_revisions",
    "knowledge_gap_signals",
    "domain_schemas",
    "catalog_rebuild_jobs",
];
```

new_string(7 拼错改对、删 chunk_revisions、末尾补 13 个;注释同步说明):
```rust
/// 业务侧 BSON 用 snake_case `workspace_id` 的集合(绝大多数)。
/// 名字须与 `src/db/mod.rs` 的 accessor 集合名字面量逐字一致;每个集合的 model
/// 都有 `pub workspace_id: String` 且 struct 头无 `#[serde(rename_all)]`(见
/// tests::KNOWN_SNAKE_TENANT_COLLECTIONS 审计基准)。
const SNAKE_CASE_COLLECTIONS: &[&str] = &[
    "wechat_accounts",
    "contacts",
    "conversation_messages",
    "agent_tasks",
    "agent_events",
    "content_assets",
    "agent_souls",
    "operation_playbooks",
    "operation_domain_configs",
    "operation_state_policies",
    "prompt_templates",
    "operating_memories",
    "operation_knowledge_documents",
    "operation_knowledge_chunks",
    "knowledge_usage_logs",
    "knowledge_chat_turns",
    "knowledge_daily_reports",
    "knowledge_operator_memory",
    "agent_decision_reviews",
    "agent_run_logs",
    "llm_call_logs",
    "memory_candidates",
    "user_operation_guide_previews",
    "management_agent_sessions",
    "management_agent_messages",
    "agent_command_runs",
    "agent_tool_calls",
    "agent_outcome_metrics",
    "evaluation_scenarios",
    "experiments",
    "proposals",
    "shadow_replays",
    "threshold_overrides",
    "threshold_overrides_audit",
    "post_release_reviews",
    "evolution_runtime_flags",
    "knowledge_gap_signals",
    "domain_schemas",
    "catalog_rebuild_jobs",
    "behavior_signals",
    "behavior_signal_metrics",
    "mcp_call_logs",
    "referral_cards",
    "agent_send_ledger",
    "ingest_sources",
    "domain_profiles",
    "agent_send_outbox",
    "relationship_type_suggestions",
    "suspected_deal_signals",
    "agent_principal_escalations",
    "knowledge_chat_tasks",
    "products",
];
```

注意:new_string 里 `chunk_revisions` 已删、7 个拼错名已改对、末尾多了 13 个。改完 SNAKE 表共 52 项(原 40 − 1 chunk_revisions + 13 = 52)。

- [ ] **Step 5: 修 CAMEL_CASE_COLLECTIONS(TDD 绿·其二)**

用 Edit 把 `CAMEL_CASE_COLLECTIONS`(当前 70-73)整块替换:移除 `admin_users`,补 `campaigns`/`campaign_sends`,保留 `llm_provider_configs`。

old_string:
```rust
/// 用 camelCase `workspaceId` 的集合(P0 鉴权 / LLM 服务商等)。
const CAMEL_CASE_COLLECTIONS: &[&str] = &[
    "llm_provider_configs",
    "admin_users",
];
```

new_string:
```rust
/// 用 camelCase `workspaceId` 的集合——对应 model struct 头带
/// `#[serde(rename_all="camelCase")]`,故 `workspace_id` 序列化成 `workspaceId`
/// (见 tests::KNOWN_CAMEL_TENANT_COLLECTIONS 审计基准)。
/// 注意:`admin_users` 不在此表——AdminUser 用 `workspaces:Vec<String>` 而非单值
/// workspaceId(auth/mod.rs:28-39),不符合单值回填契约。
const CAMEL_CASE_COLLECTIONS: &[&str] = &[
    "llm_provider_configs",
    "campaigns",
    "campaign_sends",
];
```

- [ ] **Step 6: 跑单测确认全绿(TDD 绿)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib m016 2>&1 | tail -20`
Expected: **PASS**,4 个测试全绿(`test result: ok. 4 passed; 0 failed`,或过滤到的相应数量)。若仍有红,读断言消息里的 `{name}`,回 Step 1 的机械穷举核对该集合名到底该不该在表里 / 在哪张表 / 审计基准是否漏登记——**绝不为过测试把不该收录的名字塞进基准 const**(那是自我印证的 tautology,违反过拟合红线)。

- [ ] **Step 7: 跑全量 lib 基线确认不回归**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib 2>&1 | tail -8`
Expected: `test result: ok. N passed; 0 failed`,N ≥ 350(新增 4 个 lib 单测,N 应比修改前 +4)。磁盘满(os error 112)先删 `target/debug/incremental` 再重跑。

- [ ] **Step 8: Commit**

```bash
git add src/db/migrations/m016_backfill_workspace_id_on_legacy_rows.rs
git commit -m "$(cat <<'EOF'
fix(db): 修 m016 workspace_id 回填的集合名/字段名一致性(H7)

m016 两张硬编码集合名表与真实 Database accessor / model BSON 字段名不一致,
导致 legacy 行 workspace_id 回填静默失效四类:
- 7 处集合名拼错(accounts/decision_reviews/management_sessions/
  management_messages/command_runs/tool_calls/outcome_metrics)→ update_many
  匹配 0 行,这些集合 legacy 行永不回填 → 多租户过滤上线后数据黑掉。
- 漏收录 15 个带单值 workspace_id 的集合(13 snake + campaigns/campaign_sends
  两个 camel)→ 同样永不回填。
- admin_users 误入 CAMEL 表:AdminUser 用 workspaces:Vec 无单值 workspaceId,
  回填是注入垃圾字段;移除。
- chunk_revisions 误入 SNAKE 表:ChunkRevision 无 workspace_id 字段、靠 chunk_id
  反查租户(索引/读路径均按 chunk_id),回填注入垃圾字段;移除。

修正两张表 + 加 4 个 lib 一致性单测(审计定稿真实集合名基准按 snake/camel 拆分
交叉锁死:挡拼错 + 挡 snake/camel 归错类 + 挡无 ws 字段集合误收录 + 无重复/空串)。
run_step 回填逻辑/幂等/生产 APP_ENV 守卫不变。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage(逐节核对 spec §2-§6):**
- §2.A 7 拼错改名 → Step 4 SNAKE 表 7 处改对 ✓
- §2.B 补 15(13 snake + 2 camel)→ Step 4 末尾补 13 + Step 5 补 campaigns/campaign_sends ✓
- §2.C 移除 admin_users + chunk_revisions → Step 5 CAMEL 删 admin_users + Step 4 SNAKE 删 chunk_revisions ✓
- §2.C 保留 llm_provider_configs → Step 5 保留 ✓
- §2.D 排除 system_taxonomies/taxonomy_candidates/session_seqs/migrations → 四者都不在任一表也不在任一审计基准 const,天然满足;Step 1 机械穷举复核会确认它们不被误加 ✓
- §3 方案 A(修表 + lib 单测)→ Task 1 整体 ✓;方案 B/C 已在 spec 否决,plan 不实现自动派生/不省单测 ✓
- §3 一致性单测局限(挡拼错不挡漏收录)→ plan 用 snake/camel 拆分基准额外挡住"归错类"(比 spec 原设计更强),但"漏收录"仍靠审计基准手维;这点在执行 handoff 里向用户说明 ✓
- §4.3 测试 1/2/3 → plan 用 4 个测试覆盖:snake 全集 + camel 全集(把 spec 的合并"测试1"拆成两条以额外挡归错类)+ 无重复无空串不相交(spec 测试2)+ MUST_NOT_BACKFILL(spec 测试3)✓
- §5/§6 run_step 幂等/生产守卫不变、不新起补救 migration → plan 全程不改 `run_step`(75-123)✓

**2. Placeholder scan:** 无 TBD/TODO;两张表的 old/new_string 给全量字面内容;4 个单测给完整代码;commit 消息完整。✓

**3. Type consistency:** 三个审计 const 都是 `&[&str]`,与被测的 `SNAKE_CASE_COLLECTIONS`/`CAMEL_CASE_COLLECTIONS`(`&[&str]`)同型;`HashSet<&str>` + `.iter().copied().collect()` / `.contains(name)`(name 是 `&&str`,`HashSet<&str>::contains` 接受 `&&str` 经 Borrow 匹配 ✓);测试用 `use super::{CAMEL_CASE_COLLECTIONS, SNAKE_CASE_COLLECTIONS}` 引模块级私有 const(同 crate 子模块可见 ✓)。`run_step` 签名未动。✓

**注意(留给 SDD controller):** Task 1 的单测在**表未修时**(Step 2 后)必红——这是 TDD 设计的红态,Step 4-6 修表后转绿。评审时若单独看 Step 2 的中间态会见红,属预期;完整 Task 1 执行完(Step 6)必须全绿,commit(Step 8)只在全绿后进行,绝不提交红测污染 lib 基线。

