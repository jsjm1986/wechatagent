# Ask-Human 统一频道 — 设计 spec

> 状态：设计已逐段获批（2026-06-21 brainstorming）。本 spec 只详设 **Phase 1（后端地基）**，可直接转 writing-plans 开工。Phase 2/3 仅在末尾留接口骨架，不展开。

## 一句话目标

把系统里所有「需要人类介入/审核/决策」的触点（ask-human）收口成一个统一的、可配置的前端频道；Phase 1 先打后端地基：让决策请示通道能在 admin 处置、让请示策略可配置、为收件箱提供只读聚合端点。

## 背景与问题

产品定位是「全 AI 自主、无人工接管」——客户永远只跟 AI 对话。但系统内部有约 11 类「人审/人决策」触点，分两种性质：

- **推送型（push-to-微信）**：只有**决策请示通道**（`src/agent/escalation/`，幕后领导模式）。AI 撞决策墙时把请示卡推到领导微信，领导在微信回复完成裁决。「骚扰等级/发送频率/超时」语义只对这类成立。
- **拉取型（pull-in-admin）**：知识待评审、标签候选、关系类型建议、profile 危险字段发布、进化候选、状态机/策略发布、lessons_learned 晋升等。待办躺在队列里、管理员登录 admin 处理，无「推送频率」概念。

**两个核心痛点**：
1. **散落**：拉取型触点散在「知识」「系统」两个频道，无统一入口；推送型（请示通道）**连 REST API 都没有**（`src/routes/mod.rs` 无 escalation route），admin 里只有一个只读计数卡（`knowledge/steward.tsx` 的 `principalEscalations` 来自 phase-rollup），无法处置。
2. **写死**：`principal_decider` / `high_risk_escalation_mode`（`src/models.rs:798-804`，挂在 `OperationDomainConfig`）**没有任何写入端点**——只有 `admin_ops_versions.rs:85-86` 在发布新版本时「复制结转」。今天要配置领导是谁，只能直接改 MongoDB。升级规则、骚扰频率、超时全部写死在代码里。

## 全局架构判断（贯穿三期）

**判断一：收件箱用「只读聚合器」，不建物化待办总表。**
全系统约 11 类触点散在十几个 collection。建统一待办物化表需改每个产生方的写路径（侵入大、违反 additive-only）。改为做一个只读聚合端点：后端扇出查各现有 collection，归一成统一 `InboxItem` 返前端。零侵入、不动任何写路径、红线零风险。

**判断二：统一频道是交互主场（canonical home），交互逻辑抽成中立共享组件；老页面并存不动。**（Phase 2 落地，此处仅记录）
用户最终愿景：统一频道成为「一页解决全部 ask-human」的交互主场，rich 项也在频道内打开、不往外跳。实现上把现有复杂处置交互（`steward.tsx` 的逐条核验+WebSocket 软锁+锚点、`system-strategy` 的 profile 发布二次确认）抽到**中立的共享位置**（不归属任何单一频道），统一频道作为主入口消费它们——一份代码、零重复。归属方向：不是「收件箱内嵌老页组件」，而是「组件中立化、统一频道是主场」。
**老页面本期并存、导航不弱化**：新频道证明价值前老页面是安全网，零删除风险。「逐步优化掉其他页面」是将来的独立决策，**本系列不做、不写进计划**（这是 strangler 模式的最终态，但退役节奏单独定）。
Phase 1 的聚合端点用 `action_kind`（`inline` 简单内联 / `rich` 需完整交互组件，**两者都在统一频道内打开**）标记轻重，为这个策略铺路。

**判断三：拆 3 个独立可交付子项目，各自 spec→plan→实现循环。** 交付顺序（用户拍板）：P1 后端 → P2 收件箱前端 → P3 配置页。

---

# Phase 1 详细设计（本轮交付）

三块：A 配置模型+写入端点 / B escalation 三端点+超时扫描 / C 只读聚合器二端点。**不含任何前端**。

## A. ask_human 配置模型

### A.1 数据模型

在 `OperationDomainConfig`（`src/models.rs:764`）新增字段：

```rust
/// 请示通道策略配置。None/缺省 = 沿用旧 principal_decider/high_risk_escalation_mode
/// 字段的行为（DEFAULT 字节等价，红线②）。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub ask_human_policy: Option<AskHumanPolicy>,
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AskHumanPolicy {
    /// 有序决策人链：主决策人 → 备选1 → 备选2 …。空 = 未启用请示通道。
    #[serde(default)]
    pub decider_chain: Vec<DeciderRef>,
    /// 升级触发范围（逐类别开关，取代写死的 all/decision_only）。
    #[serde(default = "default_true")]
    pub escalate_safety_guard: bool,        // 安全门拦截 → 默认 true
    #[serde(default = "default_true")]
    pub escalate_unverified_product: bool,  // 未验证产品声明 → 默认 true
    #[serde(default)]
    pub escalate_ai_policy_hold: bool,      // AI 策略暂缓 → 默认 false（= 原 decision_only）
    #[serde(default = "default_true")]
    pub escalate_stuck: bool,               // 多轮卡死 → 默认 true
    /// 骚扰等级 / 频率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_window_hours: Option<f64>,   // 同客户同类别多久内不重复推；None = 沿用现有 pending 去重
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_push_cap: Option<u32>,        // 每决策人每日推送上限；None = 无上限
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<AskHumanQuietHours>, // 静默时段不推
    /// 超时转备选：主决策人多久不回转下一位。None = 无限等待（保持原红线默认）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_hours: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeciderRef {
    pub wxid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AskHumanQuietHours {
    pub start_hour: u8,             // 0-23
    pub end_hour: u8,              // 0-23
    pub tz_offset_hours: i8,        // 复用运营时区基建
}
```

### A.2 旧字段映射（红线④：唯一真相源）

`high_risk_escalation_mode` 的 all/decision_only 被 `escalate_*` 四布尔**取代并细化**。旧字段 `principal_decider`/`high_risk_escalation_mode` **保留**（迁移期兜底 + 向后兼容），但运行时**只读** `ask_human_policy`（存在时）。解析优先级：

```
ask_human_policy 存在 → 用它（decider_chain[0] = 主决策人）
ask_human_policy = None → 回落旧字段（decider_chain = [principal_decider]，
                          escalate_* 按 high_risk_escalation_mode 映射）
```

映射规则（纯函数 `resolve_ask_human_policy(config) -> ResolvedAskHumanPolicy`）：
- `mode = "all"` → 四布尔全 true
- `mode = "decision_only"` / None → safety+product+stuck = true, ai_policy = false
- 这保证现有行为**字节等价**：旧 `should_escalate_held`（`logic.rs:254`）当前的真值表与「decision_only 映射」完全一致（safety/product 无条件、ai_policy 仅 all、stuck 经 build_decision_signals_text）。

### A.3 写入端点（当前完全缺失）

`PUT /api/admin/operation-domains/:domain/ask-human-policy`
- body = `AskHumanPolicy` JSON
- 直接 `$set ask_human_policy` 到 `(workspace_id, domain, current_version=true)` 行——**不 bump 版本**（贴生产：admin 编辑配置是 $set 到既有行，版本发布才 bump；与请示通道 MVP 文档「admin 编辑用 $set」一致）。
- 校验：decider_chain 每个 wxid 非空；escalate_* 为 bool；quiet_hours 小时范围 0-23。
- workspace 隔离：用调用方 workspace_id 约束 filter。

挂载点：`src/routes/mod.rs:624` 附近 `operation-domains` 路由组，复用现有 admin 鉴权链。handler 放 `src/routes/domains.rs`（与 `update_operation_domain_state_machine:155` 同文件同模式）。

## B. escalation REST API

决策请示通道当前无 REST 路由。补三端点，全部 workspace 隔离、挂 admin 鉴权链。新建 `src/routes/principal_escalations.rs`，在 `mod.rs` 注册。

### B.1 列表 `GET /api/admin/principal-escalations?status=pending|resolved`
- 查 `agent_principal_escalations`，workspace 约束，按 `created_at` 排序（复用 `list_pending_for_principal` 的查询形态，但按 workspace 不按 principal_wxid）。
- 返回字段：`short_code` / 客户（`contact_wxid` + 联查 nickname）/ `category` / `reason` / `question_for_principal` / 当前 `principal_wxid` / `age_hours`（now - created_at）/（resolved 项加 `decision` + `authorization_expires_at`）。
- 供收件箱与 SLA 看板使用。

### B.2 admin 直接裁决 `POST /api/admin/principal-escalations/:short_code/resolve`
- body：`{ verdict, substance, constraints[], authorization_window_hours? }`（admin **结构化**给裁决，非自由文本）。
- **复用现有下游、跳过 LLM interpret**：admin 已给结构化输入，无需 `interpret_principal_reply` 解析自由文本。直接：
  1. `sanitize_verdict`（`logic.rs:299`）校验 verdict 闭集（`ALLOWED_PRINCIPAL_VERDICT`，models.rs:2852）
  2. 算 `authorization_expires_at`（同 `handle_principal_reply:276` 逻辑：window_hours > 0 → 算过期，否则 None）
  3. `resolve_escalation`（`ledger.rs:153`，已存在）写裁决 + 过期时间
  4. `enqueue_relay_task`（`ledger.rs:249`，已存在）→ relay 任务用 AI 口吻转述客户
- relay 出站泄漏守卫（`relay_output_leaks_internal_payload`）、AI 口吻转述全程不变。
- **审计**：`AgentPrincipalEscalation` 新增 `resolved_via: Option<String>`（`"admin"` / `"wechat"`，`#[serde(default)]` 向后兼容）。admin 路径写 `"admin"`，微信路径（`handle_principal_reply`）写 `"wechat"`。
- 幂等：`resolve_escalation` 的 `find_one_and_update` 带 `status=pending` 条件，已 resolved 则返回 None → 端点返回幂等成功。

### B.3 改派 `POST /api/admin/principal-escalations/:short_code/reassign`
- body：`{ to_wxid }`。
- 校验 `to_wxid` 必须在该 workspace config 的 `decider_chain` 内（否则 400）。
- `$set principal_wxid = to_wxid` 到该 pending 行 + 重推请示卡（`render_principal_card` + `logged_call_for_account`）。

### B.4 超时转备选扫描（让 timeout_hours 生效的机制）

在现有 task worker 循环（`src/tasks.rs:13` `run_task_worker` loop）加一个周期扫描（与 `ensure_today_outcome_aggregation_tasks:164` 同位置调用）：

```
对每个 workspace 的 config（ask_human_policy.timeout_hours = Some(h)）：
  查该 workspace 所有 pending escalation，age > h 且当前 principal_wxid = chain[i]：
    若 chain[i+1] 存在 → 改派到 chain[i+1] + 重推卡 + 写审计 event
    若已是链尾 → 不动（继续等，可选重推提醒卡）
```

- 纯判定抽成纯函数 `next_decider_on_timeout(chain, current_wxid, age_hours, timeout_hours) -> Option<&DeciderRef>`（便于单测）。
- **红线**：AI 绝不自己拍板——超时只是把请示转给链上**下一位真人**。`timeout_hours = None` → 无限等待（保持原 spec §7.3 红线默认）。

## C. 只读聚合器（收件箱地基）

### C.1 明细 `GET /api/admin/ask-human/inbox?source=<filter>`

后端并行查各来源（全部 workspace 约束），归一成统一 `InboxItem`：

| source 枚举 | 查询来源 collection | 关键 status | action_kind |
|---|---|---|---|
| `principal_escalation` | agent_principal_escalations | status=pending | inline |
| `knowledge_review` | operation_knowledge_chunks | integrity_status=needs_review | rich |
| `taxonomy_candidate` | taxonomy_candidates | review_status=pending | inline |
| `relationship_suggestion` | admin_relationship_suggestions | pending | inline |
| `gap_signal` | knowledge gap signals | 未处置 | inline |
| `profile_risky` | domain_profiles 草稿 | 待发布(带 risky_fields) | rich |
| `evolution_proposal` | evolution proposals | eligible_for_release | rich |
| `lessons_learned` | lessons_learned | review_status=pending_review | rich |

统一 `InboxItem`（聚合器返回，不落库）：
```rust
struct InboxItem {
    source: String,          // 上表 source 枚举（DB 无写入，仅响应；枚举值在响应构造侧固定）
    id: String,              // 源行 id（用于处置时定位）
    title: String,
    summary: String,
    severity: String,        // high|medium|low|info（各 source 归一映射）
    created_at: DateTime,
    age_hours: f64,
    action_kind: String,     // "inline"（简单内联处置）| "rich"（需完整交互组件）；两者都在统一频道内打开
    rich_component: Option<String>,  // rich 项在统一频道内挂载哪个共享交互组件
    rich_params: Option<Document>,   // 该组件需要的定位参数（如 chunk_id / profile_id）
}
```

### C.2 计数 `GET /api/admin/ask-human/summary`
- 只返回各 source 的 pending 计数（`count_documents`，不拉明细）。供频道徽标 / 总览卡。

### C.3 设计要点
- **action_kind 由后端打标**：简单项标 `inline`（前端内联处置），复杂项标 `rich`（Phase 2 在统一频道内挂载中立共享交互组件，**不往外跳老页**）。后端标记 + rich_component/rich_params 为「统一频道是主场」铺路。
- **聚合失败降级**：每个 source 独立 `try`，失败的标 `error` 状态返回，其余正常显示（类比前端 operationsStore 兜底）。一个 source 查询异常绝不让整个收件箱崩。

---

## 红线守护清单（逐条对照 7 红线 + check 门）

1. **无人工接管（红线⑤）**：admin resolve 时 admin 是「幕后决策人」（真人决策），客户仍只收 AI 口吻转述。新增端点/字段命名一律 `principal`/`escalation`/`decider`/`ask-human`，**绝不出现** `takeover`/`人工接管`/`转人工`/`人工介入` —— 过 `check-no-human-takeover.sh`。relay 出站泄漏守卫不动。
2. **serde 向后兼容（红线②）**：`ask_human_policy`、`resolved_via`、所有新字段 `#[serde(default)]`，None/缺省 = 现有行为字节等价。
3. **AI 永不自动 verify（红线③）**：聚合器对知识审核项**只读列出**，处置仍走现有 verify 端点（双闸不变）；配置层**不提供**「知识免审」开关——拉取型只能配排序/优先级，不能关人审本身。
4. **不造双真相源（红线④）**：`high_risk_escalation_mode` 旧字段保留，新 `escalate_*` 是唯一权威；运行时只读 `resolve_ask_human_policy` 解析结果，旧字段仅 None 时兜底。
5. **闭集校验**：admin resolve 的 verdict 走 `sanitize_verdict` 闭集；source/action_kind/resolved_via 枚举在写入或响应构造侧校验。
6. **反过拟合（红线⑥）**：所有阈值（卡死轮数/骚扰窗/超时）做成可配置 + 纯函数判定，不针对单例硬编码。
7. **boundary_protection 不被放宽（红线⑦）**：配置层不引入任何降低安全门/grounding 的开关。

## 错误处理

- 聚合器每 source 独立降级（失败标 error 不整体崩）。
- admin resolve 命中已 resolved → 幂等成功（`resolve_escalation` 的 pending 条件兜住）。
- 改派目标不在 decider_chain → 400 BadRequest。
- 配置写入 wxid 为空 / 小时越界 → 400。
- 错误类型沿用 `AppError::BadRequest`（→400）/ `External`（→502）；DB `?` 自动转 `AppError::Db`。

## 测试（守反过拟合 + additive-only，维持基线不回归）

**纯函数（进 lib baseline 硬门 ≥350/0）**：
- `resolve_ask_human_policy`：all → 四 true；decision_only/None → 三 true + ai_policy false（字节等价护栏）。
- `next_decider_on_timeout`：链中取下一位 / 链尾返 None / timeout=None 不触发。
- 骚扰频率 / 静默时段判定纯函数。
- escalate_* 四布尔 → 升级决策映射（替代 `should_escalate_held` 的纯函数版，旧测试保留不改）。

**集成测试（`#[ignore]`，CI 跑）**：
- admin resolve 复用 relay 下游、幂等。
- 聚合器 source 降级（一个 source 查询失败其余正常）。
- 配置写入 $set 不 bump 版本、回读一致。
- **harness 注意**：测试 DB 经 `TestApp::start()` → `ensure_prompt_pack_v2` 已 seed 三域 config（`seeded_by="system"`），seed helper 必须用 `replace_one(upsert)` 而非 `insert_one`（见教训 [[project_config_seed_in_prompts_not_migrations]]）。

**基线**：lib ≥ 350/0 + 4 PBT 累计 ≥ 33/0 不回归。

## 迁移

`mNNN_backfill_ask_human_policy`：把现有 `(principal_decider, high_risk_escalation_mode)` 回填成 `ask_human_policy`：
- `decider_chain = principal_decider.map(|w| [DeciderRef{wxid:w}]).unwrap_or_default()`
- `escalate_*` 按 mode 映射（同 A.2 规则）
- 幂等、可重跑（已有 ask_human_policy 的行跳过）。
- **不删旧字段**（向后兼容 + 兜底）。

## Phase 1 交付边界

后端三块：A 配置模型+写入端点 / B escalation 三端点+超时扫描 / C 聚合器二端点 + 红线/测试/迁移。**不含任何前端**。交付后：请示通道可在 admin 用 API 处置，请示策略可写、超时转备选生效，收件箱聚合数据就绪。

---

# Phase 2 / 3 接口骨架（不在本轮 plan，仅预留）

## Phase 2：统一收件箱（交互主场）
- 新频道：`types/index.ts` Channel union 加 `"askHuman"`；`app/channels.ts` 加 ChannelDef（group 选「系统」或「运营」，lucide 图标如 Inbox/ShieldQuestion）；`Shell.tsx` 自动渲染。
- feature 目录 `features/ask-human/index.tsx`，包 ConfirmProvider/ToastProvider，sub-tab 切各 source。
- 消费 C 的 `/inbox` + `/summary`；inline 项内联处置（调 B 的 resolve/reassign + 各拉取型现有 approve/reject 端点）；**rich 项在统一频道内挂载中立共享交互组件**（把 steward 的 ReviewView、system-strategy 的 profile 发布抽到中立共享位置），按 `rich_component`/`rich_params` 渲染——**不往外跳老页**，统一频道是 canonical 主场。
- **老页面并存不动**：把交互组件中立化后，老页面（steward/system-strategy）改为复用同一共享组件的薄壳，**导航保持原样、不弱化**。退役老页是将来独立决策，本系列不做。
- 手动刷新（无轮询/无 WebSocket），与 steward/system-strategy 一致。
- 范例文件：抄 `features/knowledge/index.tsx`（外壳+provider+sub-tab）、`features/knowledge/steward.tsx` 的 ReviewView/LintView（审核列表+处置+重拉）作为待中立化的组件来源。

## Phase 3：ask_human 配置页
- 消费 A 的 `PUT .../ask-human-policy`。
- **微信好友选决策人**：复用 `GET /api/contacts`（`contacts.rs:98` list_contacts，返回 wxid/nickname/remark），做一个联系人选择器选 decider_chain（而非手填 wxid）。
- 配置项：升级触发范围（四 escalate_* 开关）、骚扰等级/频率（dedupe_window/daily_cap/quiet_hours）、超时（timeout_hours）、决策人链（有序，主+备选）。
- **操作说明**：每个配置项配引导文案，说明它是干什么的、怎么用、对客户体验的影响。
