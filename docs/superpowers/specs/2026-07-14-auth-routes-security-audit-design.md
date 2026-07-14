# auth + routes 安全隔离面深度逻辑审查（第二批）—— 设计

> 接续第一批 agent 旁挂能力审查（20 findings，台账 PR#207 已合 c3739d4）之后的第二批。用户裁定范围 = auth + routes 安全隔离面——上轮主链路 + 第一批之后**唯一安全敏感且未深审的区，风险最高**。方法论沿用前两轮，只审不修，产出 findings 台账后按 P0-P3 分批修。

## 背景与核心命题

上轮 53findings（主链路 8 环节）+ 第一批（agent 旁挂能力）都未深审 auth/ 与 routes/。本批圈定这两块。

**体量（亲验）**：`src/auth/` 仅 681 行（5 文件）；`src/routes/` 共 28017 行 / 46 文件（`management.rs` 3004 / `shared.rs` 2427 / `contacts.rs` 1980 / `evolution.rs` 1255 / `domain_profiles.rs` 1236 / `campaigns.rs` 1168 …）。

**认证链已亲验干净**（`auth/middleware.rs:36-90` require_session）：cookie session → JWT Bearer 双路径、TTL 双校验、白名单只 `/health`·`/auth/login`·`/auth/token` 三条，注入 `AuthenticatedAdmin { user_id, username, current_workspace }`（cookie 缺失回落 default_workspace_id）。

**所以核心命题不是认证，而是授权隔离的落实**：middleware 保证"已登录"，但**不保证"只能访问自己 workspace 的对象"**——授权靠每个 handler 自觉从 `AuthenticatedAdmin.current_workspace` 取 workspace 并锁进 DB 查询。IDOR/越权缺陷的根因层在共享授权 helper（`routes/shared.rs`）与各 handler 是否正确调用它。#153 做过 IDOR sweep（admin handler workspace 收口模式 + 隔离测试），但 routes 大量新增未覆盖。

## 范围与分簇（根因层优先 + 资源域分簇，5 簇）

- **簇S 根因层（授权共享层）**：`src/auth/`（全部 681 行）+ `src/routes/shared.rs`（2427，授权/落库 helper：validate_account:138 / find_contact_by_id:167 / upsert_contact_from_value:184(pub) / apply_contact_changes:632(pub) / apply_memory·playbook·domain_changes / ensure_operating_memory:255）。**先单独审并等结论**——workspace 是否强制锁、pub helper 跨模块暴露面、current_workspace 是否唯一 workspace 来源。此簇结论作为**审查基准**喂给簇 1-4。
- **簇1 客户数据域**：contacts.rs(1980) + conversations.rs + tasks.rs + reviews.rs + send_ledger.rs + operation_view.rs + contract_snapshot.rs。客户 PII/对话/任务——IDOR 危害最大。
- **簇2 配置/凭证域**：management.rs(3004) + llm_providers.rs(699) + accounts.rs + souls.rs + playbooks.rs + prompt_templates.rs + domain_profiles.rs(1236) + domain_schemas.rs + domains.rs。含 LLM 凭证/账号密钥/prompt——泄漏敏感。
- **簇3 媒体/运营动作域**：campaigns.rs(1168) + media_assets.rs(604) + referral_cards.rs + ask_human_inbox.rs(789) + principal_escalations.rs + simulations.rs + products.rs + chunk_locks.rs + assets.rs。能触发发送/引荐/圈人的动作面。
- **簇4 admin/指标/观测域**：admin_*.rs(7 文件：admin_ops_versions/admin_outbox/admin_relationship_suggestions/admin_state_policies/admin_suspected_deals/admin_taxonomies/admin_taxonomy_candidates) + observability.rs(859) + evolution.rs(1255) + evaluations.rs + outcomes_autonomy.rs + outcome_metrics.rs + lessons_learned.rs + guides.rs + guide_profile.rs + behavior_signal_metrics.rs + events.rs。admin 操作 + 指标读端点（#153 收口过 admin，本簇复核新增）。
- **簇5 knowledge 端点层**：`src/routes/knowledge/`（独立子目录，10 文件 / 11544 行：sources_meta.rs 1122 / wiki_edit.rs 1092 / repair.rs 862 / verify.rs 663 + catalog/chat/crud/digest_inbox/import/mod）。**亲验后独立成簇**（体量比整个第一批还大，塞簇 3 会超载）。这是知识库的 HTTP 端点层（导入/验证/修复/wiki 编辑/来源管理等写操作），IDOR/授权面真实——区别于 src/knowledge_wiki/ 子系统逻辑（后者留后续知识批）。
- 低敏略过：health.rs(15) / management_prompt_edit.rs(7)。

## 审查方法（沿用前两轮 + 安全强化）

- **簇 S 先派并等回**，其结论喂给簇 1-5 当基准；簇 1-5 拿到基准后并行审。6 subagent 全继承 Opus（省略 model 参数——`model:"opus"` 报 400，省略即继承）。
- **subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；凭猜测打回。
- **IDOR 审查检查清单**（喂给簇 1-5）：每个读/写数据的 handler——①workspace 是否来自 AuthenticatedAdmin.current_workspace（非请求体/query 可伪造来源）②DB 查询 filter 是否含 workspace_id ③按 id 取单条时是否像 find_contact_by_id 那样锁 workspace（防越 workspace 取他人对象）④list 端点是否漏 workspace 过滤 ⑤account_id 是否校验归属当前 workspace。
- **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造跨 workspace/account 越权访问）。
- **主控逐条亲验**：每 finding Read/Grep 复核 file:line + 越权链成立性，驳回夸大。
- **元家族聚焦**：middleware 保证认证不保证授权，授权靠每个 handler 自觉锁 workspace——找"自觉"漏掉处（新增 handler 忘了 #153 收口模式）。

## 严重度校准（安全语境，仍防夸大）

- **High**：推荐配置下**确定性可达的跨 workspace/跨 account 越权读写**（真实数据泄漏/篡改）或认证绕过。
- **Medium**：需多条件叠加、或多租户启用才触发、或仅信息泄漏无写入。
- **Low**：观测/边缘/输入校验缺失但无越权后果/就绪债。
- **⚠️ 关键校准原则**：**单租户默认部署下不可达的隔离缺陷 = 多租户就绪债，不夸大成 High**（memory project_multitenant_isolation_debt 已确立此口径：mcp 凭证/outbox/llm registry 硬绑 default_ws，多租户默认关单租户无害，启用才需加固）。每条严重度带主控裁定理由。

## 台账格式与产出

- 新建 `docs/superpowers/specs/2026-07-14-auth-routes-security-audit-findings.md`。
- 字段同前两轮 + 增 **越权链**（谁能越权访问谁的什么资源）：入口频道/所属簇/类型(IDOR|认证|授权|输入校验|敏感泄漏|就绪债)/严重度(带裁定理由)/现象风险/越权链/根因(亲验 file:line)/复现设想/验证状态(PLAUSIBLE|CONFIRMED)/修复建议/状态(Open)。
- **只审不修**：出完整台账 → 合并 docs PR（像上轮 PR#178/#207）。

## 后续修复路径

台账产出后按严重度定 P0-P3。**若发现 High（真实可达越权），优先级高于第一批遗留的 5 个 Medium**。每 finding 独立走 brainstorming→writing-plans→SDD→PR。

## 约束

- 纯代码/设计审查，绝不为"发现问题"改业务逻辑（反过拟合红线）。
- 不碰主仓在途工作（主仓被并行会话占 feat/principal-auth-exemption）。
- 审查分支 docs/auth-routes-security-audit 基于含 #206/#207 的最新 origin/main（c3739d4）。
- 本批产出纯 docs（台账）。

## 非目标

- 不审认证链本身（middleware 已亲验干净，除非簇 S 发现新问题）。
- 不审主链路/agent 旁挂能力（上轮 + 第一批已覆盖）。
- 不在本批做任何修复（只出台账）。
- 本批聚焦授权隔离/IDOR 主线；输入校验/DoS 等非隔离面若量大留第三批。
