# 用户运营 · 全量通讯录 + 批量托管 + 头像展示 设计

**日期**：2026-07-07
**范围**：用户（私聊）运营频道
**状态**：设计待审

---

## 0. 背景与问题

当前「把用户加入 Agent 运营」的交互存在三处硬伤：

1. **无法看到全量微信好友**。前端 `ContactsView`（`frontend/src/features/user-ops/legacy.tsx:424`）只展示本地 `contacts` 集合里已存在的联系人，且靠 `importQuery` 关键词搜索逐个导入。运营人员无法「把整个微信的好友列表拉出来，勾选谁进入托管」。后端 `contacts_search`（`src/routes/contacts.rs:156`）的 query 是必填项，本身就不支持「拉全量」。
2. **无头像**。`Contact` 结构体（`src/models.rs:138`）没有 avatar 字段，全链路都不携带头像；而 MCP server 的好友缓存接口是带头像字段的（`bigHeadImg`/`smallHeadImg`）。运营时只有一列文字，识别成本高。
3. **首次托管强制逐人录入运营备注**。`enable_agent`（`src/routes/contacts.rs:361`）硬要求 `human_profile_note` 非空（`:367-371`），并且**同步**调用 `build_initial_operation_profile`（`:396`）跑一次 LLM 生成初始画像（约 20-25 秒）。这在单个托管时合理，但批量勾选 50 个好友时：既无法逐人写备注，也无法接受 50 × 20 秒的串行阻塞。

用户明确的核心诉求：**把整个微信好友列表拉出来 → 勾选 → 批量进入托管运营**，同时**显示头像**，并且界面要从当前的「狭窄左栏」全面放大。

**关键设计张力（用户亲自点出）**：首次加入运营有一个「人类录入如何运营这个用户」的环节，批量托管时这个逐人录入无法进行。本设计的核心就是优雅地解决这个张力——既保留「人类给出运营意图」的价值，又不让它成为批量化的阻塞点。

---

## 1. 架构（已获批）

**通讯录作为用户运营频道内的第三个视图**，而非新的顶级频道。

用户运营频道当前是 smart / traditional 双模式（`frontend/src/features/user-ops/index.tsx`），smart 模式 = `ContactsView` + `CockpitPanel`。本设计在其中**新增「通讯录」视图**，与现有的联系人列表、驾驶舱并列。理由：批量托管本质是「运营准备」动作，属于用户运营的一环，不该割裂成独立频道；复用频道内已有的 account 选择、store、labels 基础设施。

**数据来源**：全量好友走 MCP 的好友列表接口（含头像字段）。拉回的好友列表与本地 `contacts` 集合**按 `wxid` 左连接**，标注每个人的 `agentStatus`（`managed` / `normal` / 未入库）。

**纯浏览不写库**：打开通讯录视图只做「MCP 拉全量 + 本地 contacts 左连接标注」，不落任何库。只有当运营人员勾选并点「加入 Agent 运营」时才写库。这样反复打开通讯录不会污染 contacts 集合。

**关键实现前提（2026-07-07 线上 `tools/list` 已亲验闭环）**：早前拉取的 MCP 指南页（`http://117.72.54.28:3001/mcp-guide.html`）曾载好友类工具名为 `contact_list`（主）与 `im_sync`（备选）——**此断言经线上 `tools/list` 证伪**：gewe-multi-tenant server（136 工具）**无 `contact_list`**（调用返 `-32000 Forbidden tool: contact_list`），`im_sync` 描述为 "Sync **enterprise** WeChat contacts"（企业微信，错域）。全量个人好友的唯一正确工具是 **`contacts_fetch_cache`**（描述 "Fetch the full remote contacts cache from GeWe"，入参 schema 为空 `{}`）。头像字段真实 key 仍**未能落定**——线上测试账号（alias `t-1`，`online:true`）的联系人缓存当前为空（`structuredContent:{}`），无 populated 样本可观察；故 `parse_roster_items` 保留 `bigHeadImg`/`smallHeadImg`/`headImgUrl`/`avatarUrl`/`headimgurl` 多 key fallback，并新增「按内容识别联系人数组」兜底（数组 key 也未核实）。⚠️ **仍开放**：待某账号缓存非空时，须再打一次 `contacts_fetch_cache` 核对 ①真实数组 key ②头像真实 key，若在 fallback 列表外则补入。另注：`call_tool_with_key`（`src/mcp.rs:202-205`）只回 `result.structuredContent`（已剥 JSON-RPC 外壳与 `content[0].text`），故解析器生效的是**顶层**数组候选。

---

## 2. 后端改动

### 2a. `avatar_url` 字段全链路

沿用刚落地的 `status` 字段全链路模式（commit `e7c9b9e`）：

- `Contact` 结构体（`src/models.rs:138`）新增 `#[serde(default)] pub avatar_url: Option<String>`。
- `WechatAccount` 结构体（`src/models.rs:58`）同样新增 `avatar_url: Option<String>`（账号自身头像，账号管理页也能显示）。
- `ApiContact`（对前端的出参 DTO）带上 `avatarUrl`。
- 头像 URL 来自 MCP 好友缓存，落库时写入；已有联系人在下次批量操作 / 同步时回填。

**不做**：不下载头像转存到本地/对象存储。直接透传 MCP 返回的头像 URL 给前端 `<img>`。若后续发现头像 URL 有防盗链/过期问题，再单独处理，不在本设计范围。

### 2b. 新端点 `GET /api/contacts/roster?accountId=<id>`

- **入参**：`accountId`（必填，指定拉哪个账号的好友）。
- **行为**：调 MCP 好友缓存接口拉该账号全量好友（含头像）→ 查本地 `contacts` 集合（`workspace_id + account_id` 过滤）→ 按 `wxid` 左连接 → 每条标注 `agentStatus`（`managed` / `normal` / `not_imported`）。
- **出参**：`{ items: [{ wxid, nickname, remark, avatarUrl, agentStatus }], total }`。
- **不写库**。
- **鉴权**：与其它 contacts 端点一致，经 `AuthenticatedAdmin` + workspace 隔离。account 必须属于当前 workspace。

### 2c. 新端点 `POST /api/contacts/batch-enable`

- **入参**：`{ accountId, candidates: [{ wxid, nickname?, remark?, avatarUrl? }], sharedNote, playbookId? }`。
  - `candidates`：勾选的好友列表。
  - `sharedNote`：**本批共享的运营备注**（对应「人类录入如何运营」，一次写、整批用）。非空校验（复用 `enable_agent` 的 note 非空红线语义）。
  - `playbookId`：可选，本批统一的运营方法。
- **行为**（对每个 candidate）：
  1. `upsert` 到 `contacts`（wxid 已存在则更新，不存在则插入基础字段 + `avatar_url`）。
  2. 置 `agent_status = "managed"`。
  3. 写 `human_profile_note = sharedNote`（整批同一份，与逐人录入的语义一致——都是「人类给的运营意图」）。
  4. 校验 candidate 的 `account_id`（= 入参 accountId）在 `wechat_accounts` 注册过（复用 `enable_agent:377-388` 的红线，否则 webhook 入站会被 `resolve_account_context` 拒收，AI 永不回复）。account 未注册直接整批 400，不做部分成功。
  5. **不同步跑 LLM**，而是给每个 candidate 入队一个 `initial_profile` `AgentTask`（见 §3），`content` 存 `sharedNote`。
  6. 老客户（`is_previously_operated`）沿用 `enable_agent:407-452` 的保留逻辑：保留已积累的 stage / operation_state / commitments，不回退 `new_contact`；全新客户由异步任务完成初始化。
- **出参**：`{ enabled: <n>, queued: <n> }`，**瞬时返回**（不等 LLM）。
- **幂等**：同一 wxid 已是 `managed` 则跳过重复入队（避免重复点击刷出多个 initial_profile 任务）。

### 2d. 单个托管保持不变

现有 `enable_agent`（`src/routes/contacts.rs:361`）**完全不动**：单个精细托管仍走同步生成画像（逐人录入 note + 立即拿到画像）。批量走异步。两条路径并存，各自服务不同场景。

---

## 3. 异步初始画像

### 3a. 新增 `AgentTask` kind `"initial_profile"`

- `AgentTask` 结构体（`src/models.rs:814`）字段已足够：`workspace_id` / `account_id` / `contact_wxid` / `kind` / `content` / `run_at` / `status`。批量入队时 `kind = "initial_profile"`，`content = sharedNote`，`run_at = now`。
- `ALLOWED_AGENT_TASK_STATUS`（`src/models.rs:853`）是闭集，`initial_profile` 复用现有 status 值（`pending`/`running`/`retry`/`failed` + 完成态），**无需**新增 status；只新增 `kind`，kind 不在那个闭集里（那个闭集约束的是 status 不是 kind）。

### 3b. worker 分发新增分支

`src/tasks.rs:230-236` 当前是：

```
if task.kind == "memory_consolidation" { ... }
else if task.kind == "outcome_aggregation" { ... }
else { handle_follow_up_task ... }        // 默认落到 follow_up
```

新增一条分支 `else if task.kind == "initial_profile"`，调用一个新的 `agent::handle_initial_profile_task(state, task)`：

1. 加载 contact（用 `account_id + contact_wxid`）。若 contact 已不是 `managed`（运营人员批量后又手动取消），跳过、置完成态。
2. 解析该 contact 的 playbook（复用 `resolve_playbook_for_contact`）。
3. 调 `agent::build_initial_operation_profile(state, workspace_id, &task.content, Some(&playbook))`（签名已核实 `src/agent/decision.rs:48`，`task.content` 就是 sharedNote）。
4. 把生成的 `agent_profile` / `profile_attributes` / stage / intent / operation_state 回填到 contact，走与 `enable_agent:410-452` **相同的写库逻辑**（含 `validate_generated_stage_intent` 的 MachineWrite 越界 drop、`is_previously_operated` 老客户保留分支、initial_operation_state 从状态机 initial 态取）。

   **为避免两处写库逻辑漂移**：将 `enable_agent` 中「用 `GeneratedOperationProfile` 组装 `set_doc`/`unset_doc` 并写库」的这段（`:403-461`）抽成一个共享函数（如 `apply_generated_profile_to_contact`），`enable_agent`（同步）与 `handle_initial_profile_task`（异步）共用。这是本设计唯一的重构，且直接服务当前目标（否则批量路径会复制一份必然漂移的画像落库逻辑）。
5. 失败重试走 worker 既有的 `attempt_count` / `max_attempts` / `retry_delay_seconds` 机制（`src/tasks.rs:252+`），无需另造。

### 3c. 画像未就绪期间的行为

批量托管后到异步画像回填前的时间窗内，若客户正好来消息：contact 已是 `managed`，webhook 会触发 gateway。此时 `agent_profile` 尚为空——**沿用系统对「无画像 managed 联系人」的既有降级行为**（gateway/decision 本就要处理新联系人首次来消息、画像字段可能缺失的情况）。本设计不为这个窗口新增特判；若真实测试发现降级不干净，作为独立问题处理，不扩大本设计范围。

---

## 4. 前端改动

### 4a. 通讯录视图（新）

在用户运营频道内新增「通讯录」视图（第三视图）。布局全面放大，摆脱当前狭窄左栏：

- **顶部**：account 选择器（选看哪个账号的好友）+ 刷新按钮 + 已选计数 + 搜索/筛选框（本地过滤已拉回的列表）。
- **主体**：头像网格 / 列表。每条 = 头像 `<img src={avatarUrl}>` + 昵称/备注 + `agentStatus` 徽标（已托管 / 普通 / 未导入）+ 多选 checkbox。已托管的默认置灰不可重复勾选。
- **底部操作条**（选中 ≥1 时出现）：本批「运营备注」多行输入框（对应 `sharedNote`）+ 可选 playbook 下拉 + 「加入 Agent 运营」按钮。
- 交互：选账号 → `GET /api/contacts/roster` 拉全量 → 勾选 → 填共享备注 → 提交 `POST /api/contacts/batch-enable` → 瞬时返回 toast「已加入 N 人，画像后台生成中」→ 刷新列表徽标。

### 4b. 现有 ContactsView 补头像

`ContactsView`（`frontend/src/features/user-ops/legacy.tsx:424`）当前每行是 dot + name + status，**在 name 前补一个头像小圆图**（`avatarUrl` 有则显示，无则占位首字母）。这是既有列表的最小增强，不重构其结构。

### 4c. store 与类型

- `userOpsStore.ts`：新增 `loadRoster(accountId)` 与 `batchEnable(payload)` action，走 `lib/api`。现有 `importContacts`（`:446`，两步 search→import）保留给「按关键词精确导入单个」的旧路径，不删。
- `types/index.ts`：`Contact` 类型加 `avatarUrl?`；新增 `RosterEntry` 类型（wxid/nickname/remark/avatarUrl/agentStatus）。
- 遵循前端设计系统：CSS Modules + 相对 import + tokens.css 变量 + zustand（**不用** `@/components/ui/*` UI 库、**不用** `@/` 别名——这是本项目的既有约定，见 AccountManagement 重写的教训）。

---

## 5. 测试

- **后端**：`batch-enable` 的 upsert + 入队逻辑（幂等、account 未注册整批 400、老客户保留字段）用直调 handler 的集成测试覆盖（state-only TestApp 范式）。`handle_initial_profile_task` 的画像回填与 `enable_agent` 共用抽出的 `apply_generated_profile_to_contact`，对该纯逻辑补单测。MCP roster 拉取用 wiremock 桩（注意 initialize 握手会多记一次请求，断言按 `tools/call` 计数，见 commit `a84ab48`）。
- **前端**：通讯录视图渲染（头像/徽标/多选）+ 批量提交 wire 键（camelCase）走 vitest。
- **基线**：不得回退 `cargo test --lib ≥ 350` 与 4 个 PBT 累计 ≥ 33（`scripts/check-baseline`）。新增测试只增量叠加。
- **无人工接管红线**：新增前端文案 / 状态标签不得含 `人工/接管/takeover/hand-off`（`scripts/check-no-human-takeover` lint）。批量托管、异步画像的所有措辞用 AI 自治口径。

---

## 6. 非目标（YAGNI）

- 不做头像本地转存/CDN（直接透传 MCP URL）。
- 不做群 / 朋友圈的批量托管（Phase 1 仅私聊用户运营）。
- 不改单个 `enable_agent` 的同步语义。
- 不为「画像未就绪窗口」新增特判（沿用既有降级）。
- 不引入分页/虚拟滚动优化，除非真实好友量级（数千）在测试中暴露卡顿——届时作为独立性能问题处理。

---

## 7. 待实现前核对清单

1. **MCP `tools/list` 亲验**：全量好友工具的确切名称 + 头像字段真实 key（`bigHeadImg`/`smallHeadImg` 或其它）。这是整个设计的数据源，名字/字段错则全链路空。
2. 确认 MCP 好友缓存接口是否需要 `account_alias`（Workspace Key 调账号类工具需传，见 MCP 接入经验）。
3. 确认好友列表接口是否分页、单次上限，决定 roster 端点是否需要循环拉取。
