# 管理面路由与鉴权深读记录（核证日期 2026-08-13）

> 范围：`src/auth/**` 全部 6 文件 + routes 管理面 25 文件（admin_* 系列、observability 系列、llm/domain 配置系列、escalation 系列、chunk_locks、worker_controls、evolution、contract_snapshot、auth/health，及 11 号任务未覆盖的 simulations / management_prompt_edit / guide_profile 三个兜底文件）。
> 方法：逐行读完全文（分段 Read，无跳读）；所有断言均在当日以 Read/Grep 亲验并附 file:line。`routes/mod.rs`（11 号亦覆盖）为核证挂载与中间件层叠全文读过一遍；`shared.rs`、`config.rs`、`main.rs`、`models.rs`、`supervisor.rs` 仅核证被本组引用的片段。
> 工作树状态：读取的是当日工作树（含未提交改动，见 git status——`domain_profiles.rs` 等有未提交修改），非某一 commit 快照。

---

## 1. 模块地图

```
HTTP 入口（src/main.rs:356-363）
  Router::new()
    .nest("/api", api_router(state))          ← 本组全部路由都在 /api 下
    .route("/webhooks/wechat", post(...))     ← 不经 admin auth（走 HMAC，03 号任务范围）
    .fallback_service(frontend/dist SPA)

api_router（src/routes/mod.rs:336-1067）
  └── .layer(from_fn_with_state(require_session))   ← mod.rs:1062-1065，包住 api_router 内全部路由
        白名单（剥 /api 前缀后）：/health、/auth/login、/auth/token（auth/middleware.rs:32-34）

src/auth/                                “谁在请求”整条链
  mod.rs         AdminUser / AdminSession / AuthenticatedAdmin / is_workspace_authorized
  middleware.rs  require_session：cookie session → JWT Bearer 双路径 + 每请求 ACL 重查
  session.rs     admin_users/admin_sessions CRUD；token 只存 SHA-256 摘要；bootstrap
  password.rs    Argon2id 哈希 + 进程级 dummy hash 抗时序
  jwt.rs         RS256 签发/验签（JWT_ENABLED 门控）
  rate_limit.rs  登录/发 token 共享三维限流 + 隐私化失败审计

routes 管理面（本组 25 文件）
  身份与探活      auth.rs（login/logout/me/workspace/token）、health.rs
  ops 三表版本    admin_ops_versions.rs（operation_domain_configs / operation_state_policies /
                  system_taxonomies 的 publish/rollout/rollback 单-current 协议）
                  admin_state_policies.rs（state policy 只读列表/详情）
  审核队列        admin_taxonomies.rs（字典 CRUD）、admin_taxonomy_candidates.rs（候选审批）、
                  admin_relationship_suggestions.rs（关系类型建议）、admin_suspected_deals.rs（疑似成交）
  决策请示        principal_escalations.rs（list/resolve/reassign）、ask_human_inbox.rs（9 源聚合收件箱）
  发送链路        admin_outbox.rs（outbox 列表 + 取消）
  观测            observability.rs（performance/phase-rollup/worker-health）、
                  behavior_signal_metrics.rs、outcomes_autonomy.rs
  运行时配置      llm_providers.rs（LLM 服务商 CRUD/测试/激活/vision）、
                  domain_schemas.rs（行业 schema 版本血缘）、domain_profiles.rs（行业总装配单
                  草稿→发布→激活）、guide_profile.rs（AI 生成候选 profile，挂 /admin/domain-profiles/generate）
  自学习          lessons_learned.rs（列表 + 晋升 peer_case）、evolution.rs（实验/提案/发布/回滚/灰度旗）
  协作与仿真      chunk_locks.rs（presence + WebSocket 事件总线）、simulations.rs（影子对话/评估）
  运维            worker_controls.rs（后台 worker 熔断查看/恢复，独立 system-operator ACL）
  基建            contract_snapshot.rs（#[cfg(test)] 前后端契约快照机制）、
                  management_prompt_edit.rs（7 行 re-export prompt_guard）
```

租户模型：`AuthenticatedAdmin.current_workspace` 是唯一租户边界，本组每个 handler 的每条 Mongo filter 都强制带 `workspace_id`（个别用 `workspaceId` camelCase 字段，如 llm_provider_configs，llm_providers.rs:420）。无 RBAC 角色——除 worker_controls 的 system-operator 名单外，同 workspace 内所有 admin 权限等同（admin_ops_versions.rs:1236-1240 注释明言）。

---

## 2. 逐文件深读

### 2.1 src/auth/mod.rs（108 行）

- `AdminUser`（L29-40）：`user_id/username/password_hash(Argon2id PHC)/created_at/last_login_at/workspaces(Vec)/default_workspace`。`workspaces` 空列表 = 无任何权限（见下）。
- `AdminSession`（L47-56）：`session_id`（Mongo 中存 `sha256-v1:<digest>`，见 session.rs）、`admin_user_id/username/created_at/expires_at(TTL 索引字段)/current_workspace`。
- `AuthenticatedAdmin`（L60-67）：middleware 注入 request extension 的已认证上下文，handler 经 `Extension<AuthenticatedAdmin>` 取；`current_workspace` 是多租户隔离的权威来源，handler 不应读 `config.default_workspace_id`。
- `SESSION_COOKIE_NAME = "wa_session"`（L70）。
- `is_workspace_authorized(resolved, user_workspaces, _default)`（L75-81）：**纯 `contains` 判定，空 ACL 拒绝一切**；第三参 `_default_workspace_id` 被忽略（历史签名残留）。注释（L72-74）：历史"空列表=默认 workspace"语义已由 m037 一次性固化为 `[DEFAULT_WORKSPACE_ID]`，此后清空列表即时撤权。测试 L87-107 锁死三种情形。

### 2.2 src/auth/middleware.rs（147 行）

`require_session`（L36-121），挂载点 mod.rs:1062-1065（layer 在所有 route 之后声明 → 包住全部）：

1. 白名单 `is_public_path`（L32-34）：`/health`、`/auth/login`、`/auth/token`（路径已剥 `/api` 前缀，L10 注释）。测试 L137-146 断言 `/auth/logout`、`/auth/me`、`/health/foo` 都**不在**白名单。
2. **路径 1 cookie**（L48-84）：读 `wa_session` → `lookup_session`（摘要查找 + 过期校验）→ `get_admin_user`（**每请求重查 admin_users**，L51）→ `current_workspace = session.current_workspace 或 config.default_workspace_id`（L59-61）→ `is_workspace_authorized`（L62-68，失败 401）→ 注入 `AuthenticatedAdmin`。ACL 即时生效的机制正在此：workspaces 列表改动下一个请求立刻生效，无需等 session 过期。`SessionExpired/SessionNotFound` 落穿到 Bearer 路径（L76-78）；其余 DB 错误 → 500。
3. **路径 2 JWT Bearer**（L87-118）：仅 `config.jwt_enabled && state.jwt_keys.is_some()`；`extract_bearer`（L123-130）取 `Authorization: Bearer ` 前缀、trim、拒空串。`verify_jwt` 通过后同样**重查 admin_users + ACL 校验 claims.current_workspace**（L92-106）——JWT 不携带 ACL，靠 DB 即时判定，token 吊销 = 从 workspaces 移除该 ws。
4. 两路径都不命中 → 裸 401（无 body，L120）。

### 2.3 src/auth/session.rs（268 行）

- 集合常量：`admin_users` / `admin_sessions`（L18-19）；摘要前缀 `sha256-v1:`（L20）。
- `session_token_digest`（L22-25）：SHA-256(hex)。`find_session_by_token`（L32-47）：**先查摘要行，miss 再查明文旧行**（返回 `(session, legacy_plaintext)`），保证升级过渡期摘要行优先，不会歧义选中明文行。
- `AuthError` 枚举（L49-63）：InvalidCredentials/SessionExpired/SessionNotFound/NoAuthorizedWorkspace/Password/Mongo。
- `bootstrap_admin_if_needed`（L78-106）：仅 env 双变量齐全且 `admin_users` 空表时创建首个 admin（幂等）；workspaces 初始 `[default_workspace]`。main.rs:63-67 启动调用。
- `authenticate`（L109-136）：用户名不存在 → `verify_against_dummy` 支付等价 Argon2 耗时（L115-119，抗枚举时序）；密码错 → InvalidCredentials；成功更新 `last_login_at`，**必须写 RFC3339 字符串**而非 bson DateTime（L128-131 注释：否则下次反序列化 AdminUser 报 "invalid type: map"）。
- `create_session`（L141-163）：token=UUIDv4 返回给 cookie，**库中只存摘要**（L159-161）；TTL `max(1)` 小时；初始 workspace 由 `initial_authorized_workspace`（L167-176）决定：ACL 空 → None（登录直接失败 NoAuthorizedWorkspace）；`default_workspace` 必须仍在 ACL 内才用，否则回落 ACL 第一项——**过期的 default_workspace 永不授予权限**。
- `lookup_session`（L180-203）：不滚动续期（L178-179 注释）；命中明文旧行则事务外透明迁移为摘要（L188-199）；返回前把 `session_id` 换回调用方 token（L200-201，绝不外泄库中摘要）。
- `delete_session`（L206-211）：`delete_many` 摘要+明文双形态 filter，幂等。
- `update_session_workspace`（L215-232）：先按摘要 update，matched=0 再按明文兜底；权限校验由调用方负责（L213-214 注释）。
- `get_admin_user`（L235-240）：按 user_id 查。

### 2.4 src/auth/password.rs（94 行）

- Argon2 默认参数（OWASP 2024：m=19MiB/t=2/p=1，L3）。`hash_password`（L30-37）每次随机盐；`verify_password`（L40-45）。
- **`DUMMY_HASH`**（L17-19）：进程启动惰性预计算的合法 PHC 假哈希；`verify_against_dummy`（L49-51）恒 false。注释 L13-16 说明红线：假哈希必须是合法 PHC，否则 parse 失败走快路径反而重新制造时序差；测试 `dummy_hash_is_valid_phc`（L83-87）锁此不变量。

### 2.5 src/auth/jwt.rs（178 行）

- `JwtClaims`（L37-43）：`sub(user_id)/username/current_workspace/exp/iat`——与 cookie 注入的 AuthenticatedAdmin 等价，**不复制 ACL**；workspace 切换 = 重新签发（L11-12）。
- `JwtKeys::from_config`（L64-82）：`jwt_enabled=true` 时私/公钥 PEM 任一缺失或解析失败返 Err → main.rs:159-160 经 `?` 直接启动失败（"以为开了实际没开"防御）。`ttl_minutes.max(1)`。
- `issue_jwt`（L86-103）：RS256，exp=now+ttl。`verify_jwt`（L106-116）：`leeway=0`；过期 → `Unauthorized("token_expired")`，其余 → `Unauthorized("token_invalid")`（错误码表 L17-22）。
- 测试密钥来自 `tests/fixtures/jwt_test_*.pem`（L127-128），不参与生产。

### 2.6 src/auth/rate_limit.rs（450 行）

`/auth/login` 与 `/auth/token` 共享一个 `AuthRateLimiter`（AppState 持有，mod.rs:329-331；main.rs:188-192 用 4 个 config 值构造）。

- 结构（L58-66）：`window`（秒，min 1）、`client_capacity`、`target_capacity`、`global_capacity`（上限再夹 `MAX_TRACKED_AUTH_ATTEMPTS=100_000`，L19/80）、进程级随机盐 `process_salt`（L75，重启即换）、`Mutex<LimiterInner>`。
- **三维窗口**（`begin_at` L95-198）：
  - 指纹（L200-207）：`sha256(salt ‖ namespace ‖ 0x00 ‖ value)`；client=直连 IP，target=**trim + 小写化**的用户名（L101-105，`Alice`/`alice`/`ALICE` 同一 target，测试 L353-359）。
  - 每次尝试前先清理窗口外记录（L107-112），再计数：`client_count`（该 client 指纹的 Pending+Failed）、`target_count`（该 target 的 Pending+Failed）、`global`（**仅 Pending**，L129-134——全局维度只保护 Argon2 并发，历史失败不占全局槽，防"随机撞名把全体管理员锁死"，测试 L389-404）。
  - 任一满 → `AuthRateLimitExceeded{retry_after_seconds（按维度内最早尝试推算+1，L136-155）, dimension∈{global, client_and_target, client, target}, subject, should_audit}`；`should_audit` 按 `rejection_audit_key`（L233-243）在窗口内**每等价拒绝键只审计一次**（L165-173，防审计风暴；审计表容量上限 100_000）。
  - 通过 → 登记 `Pending` 并返回 `AuthAttemptPermit`（L182-197）。
- **Permit 生命周期**（L245-282）：`mark_invalid` → 状态转 Failed（留在窗口计数）；`mark_success` → 删自身 + 删同 subject（client+target 对）的全部 Failed（L215-226，成功登录清对失败记录但不清其它 target 的，测试 L371-379）；**Drop 未终结（基础设施错误/请求取消/panic）→ cancel 直接释放槽**（L274-282，不惩罚，测试 L382-386）。
- **审计**（L285-337）：`auth_security_events` 集合；文档只含盐化指纹（`fingerprint_scheme: "sha256-process-salt-v1"`）、entrypoint、outcome、limit_dimension、retry_after、created_at、expires_at（**90 天保留**，L21/314-319）；测试 L431-449 断言不含明文 IP/用户名/密码字段。

### 2.7 src/routes/auth.rs（328 行）

| handler | 方法/路径（mod.rs 行） | 权限 | 校验与业务 | 写集合 |
|---|---|---|---|---|
| `login` L51-93 | POST /auth/login（mod.rs:339） | 公开（白名单）| trim 用户名，双字段必填（L57-63）；`authenticate_public_endpoint`（限流+验密）→ `create_session`（TTL=`session_ttl_hours` max 1）→ `ensure_workspace_taxonomies`（L80）→ Set-Cookie | admin_sessions、admin_users(last_login_at)、auth_security_events、system_taxonomies(seed) |
| `logout` L95-106 | POST /auth/logout（mod.rs:340） | 需登录 | 删 session 行（失败仅 warn）+ 清 cookie；幂等 | admin_sessions |
| `me` L108-124 | GET /auth/me（mod.rs:341） | 需登录 | **每次重查 DB** 取最新 workspaces 列表（L112-117） | 无 |
| `switch_workspace` L132-161 | POST /auth/workspace（mod.rs:342） | 需登录 | target 非空；重查 user → `is_workspace_authorized`（不在 ACL → 400 `workspace_not_in_user_acl`，L146-148）→ `ensure_workspace_taxonomies` → 原地更新 session.current_workspace | admin_sessions、system_taxonomies(seed) |
| `issue_token` L209-240 | POST /auth/token（mod.rs:343） | 公开（白名单）| `jwt_enabled=false` → 401 `jwt_disabled`（L214-216）；同 login 的限流+验密；`initial_authorized_workspace` 无授权 ws → 401；返回 `{token, tokenType:"Bearer", expiresInMinutes, currentWorkspace}` | auth_security_events |

- Cookie 属性（`build_session_cookie` L163-171）：HttpOnly、**SameSite=Strict**、Secure=`config.session_cookie_secure`（默认 false，config.rs:779）、Path=/、Max-Age=ttl 小时。
- `authenticate_public_endpoint`（L254-301）：**Argon2 之前先 `begin` 占限流槽**；被限 → 首个等价拒绝写 `rate_limited` 审计 + 返回 `AppError::AuthRateLimited{retry_after}`；密码错 → `mark_invalid` + `invalid_credentials` 审计 + 401；成功 → `mark_success`；基础设施错误 → permit Drop 释放（L248-253 注释）。
- `direct_client_identity`（L242-246）：只取 `ConnectInfo` 直连 IP，缺失时 `"unknown-direct-peer"`；**不解析 X-Forwarded-For**（见 §5 疑点 7）。
- `map_auth_error`（L173-184）：InvalidCredentials/SessionExpired/SessionNotFound/NoAuthorizedWorkspace → 401 各自错误码；Password → External；Mongo → Db。

### 2.8 src/routes/health.rs（15 行）

GET /health（mod.rs:338，白名单）。返回 `{ok:true, appBaseUrl, evolutionEnabled}`（L8-14）；`evolutionEnabled` 供前端 EvolutionCenterTab 渲染占位。无 DB 访问——**不是数据库健康探针**。

### 2.9 src/routes/mod.rs（1249 行；11 号亦覆盖，此处只记本组核证所需）

- `AppState`（L284-334）：db/mcp/llm/llm_registry/llm_concurrency/config/prompt_pack_version/chat_progress_bus/second_reviewer_llm/**chunk_locks**(DashMap)/**chunk_event_bus**(broadcast)/**jwt_keys**(Option)/**auth_rate_limiter**/completeness_cache。
- 本组端点挂载行号：auth L338-343；worker-controls L844-848；taxonomies L850-857 + 版本三动作 L947-958；taxonomy-candidates L858-866；relationship-suggestions L868-879；suspected-deals L884-892；state-policies 只读 L898-905 + 版本三动作 L935-946；operation-domains 版本三动作 L906-917；principal-escalations L920-931；ask-human L933-934；outbox L960-961；lessons-learned L963-969；observability L972-976；llm-providers L978-988；domain-schemas L990-1001；domain-profiles L1003-1029 + generate L1031-1034；evolution L1036-1059；chunk lock L571-575（`/ws/chunks` 也在 layer 内 → WebSocket 握手同样过 require_session）；simulations L405-412；behavior-signal-metrics L742-745；outcomes L746-747；health/auth 见上。
- `contract_snapshot` 模块是 `#[cfg(test)]`（L48-49）——生产二进制不含。
- 死路由 tripwire 测试（L1080-1248）：扫描 `pub async fn` 是否都被 mount；include 名单（L1083-1129）与豁免名单（L1132-1211）均手工维护（见 §5 偏差 2）。

### 2.10 src/routes/admin_ops_versions.rs（1390 行）—— ops 三表版本协议核心

三表共享 `(version, current_version, previous_version, seeded_by)` 四元字段（L1-17）。行为约定（L9-14）：publish=事务内 `max(version)+1` 新行 + 降级旧 current；rollout=历史版本切 current；rollback=按 `previous_version` 找回上一版切 current；**历史版本永不删除**。

**共享事务内核**：
- `commit_ops_transaction`（L87-98）：commit 循环重试 `UnknownTransactionCommitResult`（**无限重试直到确定**，与 taxonomy/profile 的有限重试不同）；其余错误 → 409 `ops_version_switch_conflict`。
- `unique_current_id[_with_session]`（L110-172）：scope 内 `current_version=true` 行必须**恰好一条**：0 条 → 409 `missing_current_ops_version`；≥2 条 → 409 `multiple_current_ops_versions`。
- `insert_new_current`（L174-254）：事务内 ① 重验 current 未变（CAS `expected_current_id`，变了 409 `ops_current_changed`）② `max(version)+1`（`checked_add` 防溢出）③ 降级旧 current（`modified_count==1` 硬校验）④ 插入新行（`current_version=true`）⑤ 可选 `bump_generation_with_session`（taxonomy 命名空间，同事务）。
- `switch_current`（L256-331）：事务内 ① target 必须在 scope 内 ② current==target → no-op false ③ 降级 current ④ promote target（filter 带 target 读取时的 `current_version` 值做同快照 CAS，L294-297）⑤ changed 时事务内 bump generation（L319-328）→ commit。

**operation_domain_configs**（scope=`(workspace_id, domain)`）：
- `publish_operation_domain_version`（L442-477）：POST /admin/operation-domains/:id/publish。按 `_id+workspace` 读 source（跨租户 404）→ `insert_new_current_domain_config`（L344-390，克隆 source 全字段、status 恒 "active"、seeded_by="manual"、user_ops 域先过 `validate_and_normalize_user_runtime_parameters` L44-52）。
- `publish_state_machine_version`（L720-809，pub(crate)，被 domain_profiles.activate 复用）：**no-op 幂等短路**——新本体与 current 行逐字节相等时不发版，但仍幂等 reconcile policy（G11 补漏，L749-773）；否则克隆 current 行仅换 state_machine 发新版，随后按 `forbidsProactive` 联动重派生 policy（L796-804），policy 溯源标签 `statemachine_publish:<seeded_by>`。
- `reconcile_state_policies_for_machine`（L537-715，pub(crate)）：逐 state 从 `derive_state_policy_lists(forbidsProactive)`（m013 单一真相）导出 `(allowed, forbidden)`；**只认 current_version=true 的 policy 行**（L565-570 注释：裸 find_one 会改到历史行污染回滚链）；`is_refreshable_policy_seeded_by`（L507-516）区分机器派生行（None / `statemachine_publish:*` / `statemachine_edit:*` / `legacy_migration` → 可刷新）与手工行（其它值 → 恒保留）；缺行用 `next_version_for_scope`（L1208-1230）分配版本避开 `(ws,domain,state_key,version)` 唯一索引；**best-effort**：逐 state 失败进 `StatePolicyReconcileReport.failures`（L54-78），不回滚主操作。不变量（L503-506）：所有手工 policy 写入点必须写不可刷新的 seeded_by（今天写 "manual"），否则会被误判机器行 clobber。
- `rollout`（L811-855）/`rollback`（L857-922）：切 current（rollback 经 `previous_version` 找目标，None → 400）后**事务外** best-effort reconcile policy 到目标机器（G12 防漂移）。
- **operation_state_policies**（scope=`(ws,domain,state_key)`，L926-1058）：publish 恒写 `seeded_by=Some("manual")`（L961，守护上述不变量）；rollout/rollback 同构。
- **system_taxonomies**（scope=`(ws, scope, kind, value.id)`，L1062-1202）：三动作均传 `generation_workspace=Some(ws)`（事务内 bump），提交后 `invalidate_global_taxonomy_cache`（进程内 shard 失效，其它副本靠 generation 重建，L16-17）+ `audit_taxonomy_change`（L1244-1292）：写 `agent_events` kind=`taxonomy_version_changed`，含 admin user_id/username/scope/kind/valueId/version；**fail-soft `let _`**（审计失败不反噬业务，L1242-1243）。

### 2.11 src/routes/admin_state_policies.rs（233 行）

- `list_operation_state_policies`（L45-78）：GET /admin/operation-state-policies?domain=&stateKey=&includeAllVersions=。workspace 强制；空白 filter 参数 trim 后忽略；默认 `current_version: {$ne:false}`（兼容 m015 前缺字段的老行，L57-59）；响应附 `actionValues: OPERATION_STATE_ACTION_VALUES`（L76，闭集由 agent 层供给前端下拉）。
- `get_operation_state_policy`（L80-96）：`_id+workspace` 查，404 不泄漏存在性。
- 投影 `operation_state_policy_json`（L98-116）：13 键含四元灰度字段；契约快照测试 L213-232。

### 2.12 src/routes/admin_outbox.rs（827 行）

- 常量：`MAX_CANCEL_REASON_LEN=200`、列表默认 50 / 上限 200（L37-41）。
- `list_outbox`（L62-121）：GET /admin/outbox。filter=workspace + 可选 status CSV/account_id/horizon；**status 每个 token 必须命中 `OutboxStatus::from_str` 闭集，否则 400**（`parse_status_filter` L286-309；闭集全集见测试 L544-557：`pending,in_flight,sent,failed_terminal,canceled,delivery_unknown`；旧 `failed` 字面量按 R13.10 非法，L567-569）；horizon 必须 RFC3339 否则 400（L82-88）；返回 `{items,total}`；每页最多两次批量回表取 media/referral card 元数据（L331-401），**目标 account 与 outbox 行 account 不一致时元数据置 null 不外泄**（L413-417/426-430，测试 L652-673）。
- typed payload（L403-445）：`(media_asset_id, referral_card_id)` → text/media/referralCard；**两者同时存在 → kind="invalid", reason="multiple_payload_targets"**（L438-444）。
- `cancel_outbox`（L123-141）→ `cancel_outbox_inner`（L149-281，pub(in routes)，管理 Agent 工具复用）：
  - 硬校验：`expected_account_id` trim 非空（400）；`cancel_reason` trim 非空且 ≤200 字符（chars 计数，400）。
  - **pending**：`find_one_and_update`(filter=`_id+workspace+account+status=pending`) 原子置 `canceled` + `$unset worker_id/locked_until/claim_token`（L186-210）。
  - **in_flight**：只登记 `cancel_requested=true, cancel_requested_at`（L219-237），由 dispatcher 在远端边界前取消或按真实回执收敛。
  - 两者都不命中（终态/跨租户/账号错配/不存在）→ 409 `outbox_not_cancelable`（L238-240）——**跨租户/错账号零写**。
  - 审计事件 `outbox_canceled` / `outbox_cancel_requested`（status="warning"，details 含 cancel_disposition/source="admin_route"，fail-soft `let _`，L251-275）。
  - 可取消集 {pending, in_flight} 与 `outbox_status_is_user_cancelable` 用 debug_assert 对齐（L177-182）。

### 2.13 src/routes/admin_taxonomies.rs（643 行）

- 事务参数：commit 最多 3 次重试 + `max_commit_time=5s`（L39-46）；`commit_taxonomy_transaction`（L48-90）第 3 次仍 Unknown → **读权威行（committed_filter）判定成败**，行在 → Ok，不在 → 409 `taxonomy_commit_result_unknown`；重复键 → 409 `taxonomy_identity_claim_conflict`；其它 → 409 `taxonomy_transaction_conflict`。
- `list_taxonomies`（L145-182）：scope 参数须过 `authorize_taxonomy_scope`（L477-486：`"global"` 直通，否则 `validate_account` 校验 scope 是本 workspace 账号，shared.rs:189-209）；默认只列 `value.status="active"` 且 `current_version≠false`。
- `create_taxonomy`（L184-282）：scope/kind/value.id/label 非空（400）；`label` 别名兼容 `displayName`（L118-121）；插入 version=1、current_version=true、seeded_by="manual"；**事务=insert + bump generation**；`(scope,kind,value.id)` 唯一索引冲突 → 409 `duplicate_taxonomy`（带 message，L241-253）。
- `patch_taxonomy`（L284-408）：只改 `current_version=true` 行；可改 label/aliases/description/deprecated(双向)/isTerminal/isReactivationTarget；aliases 改动重算 `identityClaims`（`taxonomy_identity_claims(id, aliases)`，首元素是 id 自身故 `skip(1)` 得净化别名，L312-320）；空 patch → 400；事务 + bump + 进程缓存失效。
- `delete_taxonomy`（L410-475）：**软删除** `value.status="deprecated"`（历史 run 留档可读，L9-10）；同款事务。
- `is_duplicate_key_error`（L514-527）：11000/11001，含 BulkWrite 变体。

### 2.14 src/routes/admin_taxonomy_candidates.rs（695 行）

- `list_taxonomy_candidates`（L77-116）：status 默认 "pending"，传 "all" 不过滤；scope 过 authorize；limit 硬 500，按 last_seen_at 倒序。
- `approve_taxonomy_candidate`（L118-140，REST）→ `approve_candidate_transaction`（L235-491）。事务全链：
  1. `canonicalValue.id/label` trim 非空（400）；
  2. 事务内读 candidate（workspace 过滤）→ 可选 scope 校验（**仅管理 Agent 工具侧入口 `approve_taxonomy_candidate_inner` L206-228 传 account_id**：`taxonomy_scope_allows`（L194-196）= scope 是 global 或等于发起账号；REST 侧显式保持无 scope 校验，L204-205 注释）；
  3. status≠pending → 409 `taxonomy_candidate_not_pending:<status>`；
  4. **pending→approving 两段 claim**（L280-296，CAS 失败 409 `taxonomy_candidate_claim_conflict`）；
  5. 查 canonical 的 current 行（limit 2 防多 current，L312-333）+ 最大 version 行：
     - 已有 canonical → `normalized_aliases`（L503-524：existing∪requested∪raw_value，trim、去重、剔除 canonical id）无变化则**不发新版**（merged=true 直接成功）；有变化 → 退役旧 current（带 version CAS）+ 插 `version=max+1` 新 current（继承运行字段，seeded_by="manual"）；
     - 无 canonical → 新建 TaxonomyEntry（description 优先请求值、缺省用 candidate.evidence；version=max+1 或 1；`next_taxonomy_version` checked_add 防溢出 L493-501）；
  6. 字典变更时事务内 bump generation（L418-426）；
  7. approving→approved 终态（reviewed_at/reviewed_by=actor，CAS 失败 409 finalize_conflict）；
  8. commit 无限重试 Unknown；重复键 → 409 `taxonomy_identity_claim_conflict`（事务体与 commit 两处映射 L459-484）；成功后失效进程缓存。
- `reject_taxonomy_candidate`（L142-188）：reason trim 非空（400）；`status:"pending"` CAS 置 rejected + `rejection_reason`（动态字段，模型未声明，L167-169 注释）；miss → 404 "not found or not pending"。
- actor 恒为 `ReviewActor`（shared.rs:24-55）：REST 从 `admin.username`（空则 401），工具侧 `system:management_agent`——**请求 JSON 无法伪造 reviewedBy**（relationship 的测试 SR-058 同一精神）。

### 2.15 src/routes/admin_relationship_suggestions.rs（427 行）

- 语义（L1-24）：LLM 建议 relationship_type 不直接生效，人审 approve 才回写 contact——保守闭环。
- `list`（L64-98）：workspace 强制 + status 默认 pending / "all" 全量；limit 500。
- `approve`（L100-112 → inner L118-264）：
  1. workspace 隔离读 suggestion（跨租户 404 不泄漏）；status≠pending → 400；
  2. **AdminWrite 维度校验**：`validate_dimension_value(db, ws, "relationship_type", suggested_value, account, AdminWrite)`（L142-164）——Reject → 400（越界值不落库）；DropSilently（空值）→ 400；Accept 取 canonical；
  3. `find_contact_by_id`（workspace 强制）+ `contact.account_id == suggestion.account_id`（不等 → 409 identity_changed，L168-173）；
  4. **事务**：① suggestion 以**完整快照 CAS**（`_id+ws+account_id+contact_id+suggested_value+last_seen_at+status=pending`，L184-205——绑定"审的就是验过的那个对象"，并发 gateway 刷新即失败）置 approved；② contact 用 **pipeline `$mergeObjects`** 合并 `domain_attributes.relationship_type=canonical`（`$ifNull` 兼容缺失/null 容器；非文档值使事务失败而非静默覆盖，L212-239）+ 刷新 `domain_attributes_updated_at/updated_at`；matched≠1 → 409 contact_changed；commit 无限重试 Unknown。
- `reject`（L266-309）：reason 必填非空；pending CAS 置 rejected + rejection_reason；**注意**：终态回读 L302-304 只按 `_id` 无 workspace 过滤（前一步已在本 workspace matched=1，无越权风险，风格不一致而已，见 §5-11）。

### 2.16 src/routes/admin_suspected_deals.rs（390 行）

- 红线（L7-9）：**AI 永不直写 outcome_events**；approve 强制 `verification=Some("staff_confirmed")`。
- `list`（L72-106）：同款 pending 默认 + workspace 强制 + limit 500。
- `approve`（L108-220）：**validate-first + 事务**两段：
  - 事务前：读 signal（ws 过滤，404）→ `ReviewActor::from_admin` → `find_contact_by_id` → `prepare_outcome_event`（shared.rs 核心，L143-160）完成金额（最小币种单位）、币种（ISO-4217）、product 归属校验并**冻结成交快照**——任何校验失败时 signal 仍是 pending 可修正重试（L15-16）；
  - 事务内：① `status:"pending"` CAS 置 approved（modified≠1 → 409 `suspected_deal_not_pending`）② `persist_prepared_outcome_event_with_session`（同事务追加 contact.outcome_events + agent_events 审计）；Db 错误 → 409 `suspected_deal_approval_conflict`；commit 无限重试 Unknown。重复 approve 在 CAS 处冲突不重复登记（L22-23）。
  - 请求体 `amount/currency/productId` 全可选（serde default，测试 L339-353）；多余的 `reviewedBy` 字段被忽略（非 deny_unknown_fields，但模型无该字段）。
- `reject`（L222-263）：与 relationship 同构（含同样的"终态回读无 ws 过滤"风格差异，L258-260）。

### 2.17 src/routes/principal_escalations.rs（309 行）

- `list`（L26-60）：GET /admin/principal-escalations?status=。**status 必须 ∈ `ALLOWED_PRINCIPAL_ESCALATION_STATUS`**（`pending|resolved|delivery_failed`，models.rs:4491-4499），否则 400（L32-36）；默认 pending。投影含 shortCode/contactWxid/category/reason/questionForPrincipal/principalWxid/status/ageHours/createdAt/decision/authorizationExpiresAt/resolvedVia（L43-56）。**注意**：`createdAt`/`authorizationExpiresAt` 是裸 `bson::DateTime` 直接进 `json!` → wire 上是扩展 JSON `{"$date":…}` 对象（见 §5 疑点 5）。
- `resolve`（L163-191）：POST :short_code/resolve。
  - IDOR/幂等门：**只在本 workspace 的 pending 列表内找 short_code**；找不到（已 resolved 或越权）→ `{ok:true, alreadyResolved:true}` **幂等成功以避免泄漏存在性**（L169-175）。
  - **DTO 硬校验** `ResolveBody`（L68-81）带 **`deny_unknown_fields`**；`validate_admin_decision`（L83-159）：verdict ∈ 5 值闭集（approved/rejected/conditional/deferred/delegated_back，models.rs:4563-4576）；exemptionType ∈ {none, customer_only, knowledge}（models.rs:4578-4580）；approved|conditional 必须有非空 substance；exemption≠none 与 authorizationWindowHours 都**只允许**配 approved|conditional；window 必须有限且 ∈ (0, 8760]（`MAX_AUTHORIZATION_WINDOW_HOURS=24*365`，L66）；constraints 逐条 trim 去空。
  - deferred → 保持 pending 不 resolve 不 relay（L177-180）；expires 只约束本次转述可用期，customer_only/knowledge 豁免长期常驻（L181-185）；`resolve_escalation(..., "admin")` 返 None → alreadyResolved（并发已处置）。
- `reassign`（L259-309）：POST :short_code/reassign。pending 内找（miss → 404，与 resolve 的幂等策略不对称——改派是纯管理动作无泄漏顾虑）；`to_wxid` ≠ 请示客户本人（400）≠ 当前决策人（400）；**必须命中冻结协议 `protocol.policy.decider_chain`**（无协议的旧请示 → 409"不能自动改派"，链外 → 400，L279-288）且目标 decider 绑定非空 account_id（400）；`reassign_escalation` 带 `protocol.delivery_generation` 做投递代际 CAS（当前投递未终结/并发处置 → 409，L295-305）；成功后**即时物化**领导卡投递，进程中断由 worker 按同 generation 幂等补偿（L306-307）。

### 2.18 src/routes/ask_human_inbox.rs（1202 行）

- 只读聚合器：**9 个待审源扇出，各自独立降级**（单源 Err 记入 `errors` 数组不整体 5xx，L686-696 宏）。
- `InboxItem`（L14-60）：统一形态 source/id/account_id/title/summary/severity（排序档 low|medium|high）/created_at/age_hours/action_kind("inline"|"rich")/rich_component/rich_params + 各源富字段（escalation 的 category/question/contact/principal；suggestion 的 evidence/confidence/occurrences；gap 的 kind/signal_severity；knowledge 的 integrity_status）。空串归一 None（L69-75）。
- 9 源与 filter（`ask_human_inbox` L675-730 逐一挂接）：
  1. principal_escalation：pending，severity=high，inline（L106-132）；
  2. knowledge_review：`integrity_status ∈ {needs_review, needs_human_audit}`（`knowledge_review_statuses()` L137-139，**双状态防 needs_human_audit 黑洞**，KB-08，测试 L1171-1180），rich=knowledgeReview，limit 100；
  3. taxonomy_candidate：**只列 global scope** 的 pending（账号私有候选不进共享收件箱，L251-252/274-280），rich=taxonomyCandidateReview，title 不暴露裸维度键（测试 L1081-1086）；
  4. relationship_suggestion：pending，inline；
  5. suspected_deal：pending，rich=suspectedDealReview，severity=high；
  6. gap_signal：pending，inline，severity 恒 "medium"（语义严重度走 signal_severity 单独字段，测试 L1037-1038）；
  7. profile_risky：`reviewable_profile_filter`（L461-470）= `is_active=false ∧ (draft ∨ published-current)`——待发布草稿与待激活已发布行都进审（历史 published 行除外），rich=profilePublish，severity=high；
  8. evolution_proposal：`status="eligible_for_release"`，rich=evolutionRelease；
  9. lessons_learned：`review_status="pending_review"`（裸 Document 集合），rich=lessonsPromote；**id 必须用 `lesson_id` 字段而非 `_id` hex**（否则深链/晋升 404，L601-604 注释）。
- 账号过滤 `account_scoped_filter`（L652-672）：query.accountId 有值时，escalation/suggestion/deal/proposal 精确匹配；knowledge_review 额外放行 `account_id` null/缺失（workspace 全局治理项仍可见，include_global=true）；taxonomy/gap/profile/lessons 为 workspace 全局源不加账号过滤。
- `ask_human_summary`（L733-825）：`tokio::join!` 并发 9 个 `count_documents`（与列表**同一 filter 函数**防口径漂移，L136）；`build_summary_response`（L827-878）：单源失败 → 该 count 为 **null（绝不伪造 0）**、errors 记源、status=complete|partial|error、`total` 仅 complete 时给值；顶层同时平铺 legacy 键做旧客户端兼容（L875-876）。测试 L1183-1201 锁 partial 语义。

### 2.19 src/routes/observability.rs（1799 行）

三个只读聚合端点，全部 workspace=admin.current_workspace（query 不可指定 ws，L115-116）：

- **`performance_summary`**（L117-222，GET /admin/observability/performance）：`hours ∈ [1,168]` 否则 400；`path` 必须 ∈ 6 值闭集 {direct, escalated, rewrite, revision, no_reply, manual}（L51-58/138-145）；`agent_run_logs`（要求 `gateway_result.performance.totalMs` 存在）与 `llm_call_logs` 各采样 ≤20_000 行（`PERFORMANCE_MAX_ROWS`，L50）并报告 `truncated`；产出 overall/byPath（`PerformanceBucket` L68-113：totalMs/llmCalls/tokens/stages 分布）、operations（`OperationMetrics` L300-426：knowledge toolTrace 观察集、zero_local_relevance skip 率、unknown_usage、degraded_reasons **每 run 去重后计数**，`degradation` 与 legacy 别名 `budget` 双发 L411-415）、llmAdmission（foreground/background/legacyUnclassified 三桶 latency/queueWait/providerLatency，L232-281）。分位数 = nearest-rank（L432-453），空集 → 全 null。
- **`phase_rollup`**（L497-612，GET /admin/observability/phase-rollup）：固定 24h 窗（`WINDOW_MS` L492）。八块：
  - lifecycle（L614-666）：`$group by lifecycle`；**闭集 7 值恒输出（无样本=0）**，闭集外的值原样透出并标 `outOfClosedSet:true`（L659-664，"不吞历史脏数据"）；
  - holdBreakdown（L674-725）：`final_review_status ∈ {held_by_ai_policy, blocked_by_safety_guard, ai_waiting_for_more_context}` 三类 24h 计数（与 outcomes_autonomy 的 7 日 ratio 同源不同窗，L668-673）；
  - revisionReasons top10（L727-761）；reviewerMisjudge 分类计数（L763-804）；
  - reviewerStats / dealAttribution（L806-840）：读 feedback_worker 的滚动缓存 doc（`reviewer_stats`/`deal_attribution_stats`，缺失返回 `{}`）；
  - negativeExamplePending（L842-858）：chunks `chunk_type=negative_example ∧ integrity_status=needs_review` 即时计数；
  - principalEscalations（L875-974）：status 全量分布（闭集 3 值 + outOfClosedSet 透出）、pending 年龄四桶 `lt_1h/1h_6h/6h_24h/gt_24h`（`age_bucket_index` L980-991，>24h 桶 = 领导长期未回告警）、oldestPendingAgeMs、relayDeliveryFailed = `agent_tasks{kind=principal_decision_relay, status=failed}` 计数（relay 最后一公里断裂硬信号）；
  - 每个指标都带 `metricScopes` 元数据（kind/consistency/窗口，L1005-1023）声明口径。
- **`worker_health`**（L1041-1095，GET /admin/observability/worker-health）：
  - chatTasks（L1257-1330）：`knowledge_chat_tasks` 全量 status 分布（闭集=`ALLOWED_TASK_STATUS` 5 值 `pending/running/completed/failed/cancelled`，models.rs:5934-5935）+ failed 的 error_kind top10；
  - gapSignals（L1332-1440）：status 分布（闭集 5 值 `pending/auto_resolved/llm_resolved/applied/dismissed` L1362-1368）+ pending kind top10 + **`historicalResolvedShare`**——代码注释明确这是"保留历史中已解决状态占比"，**不是某轮 sweep 命中率**（集合无 run/cohort 标识无法反推，无样本 → null 不伪造 0%，L1409-1410）；
  - lessonsLearned（L1450-1537）：14d 窗（与 feedback_worker 同窗，L1446-1448）pattern×review_status 矩阵 + 闭集 3 pattern（success/reviewer_misjudge_negative/blocked_by_safety_guard）恒输出；
  - postDecisionProjection（L1097-1255）：`agent_decision_reviews.post_decision_status` 闭集 7 值 {prepared,pending,retry,processing,completed,failed_terminal,discarded} 分布 + 最老 active 年龄 + error_kind top10 + 采样 2000 行的 attempts/snapshotBytes/completionLatency/staleProfileSkips。

### 2.20 src/routes/behavior_signal_metrics.rs（103 行）

GET /behavior-signal-metrics?fromDate=&toDate=&limit=。workspace 强制；date 字符串区间过滤；limit 默认 60 夹 [1,365]（L46）；按 date 倒序。投影 8 键（L65-76）+ 契约快照（L85-102）。纯只读。

### 2.21 src/routes/outcomes_autonomy.rs（544 行）

- `get_autonomy_outcomes`（L215-442，GET /outcomes/autonomy?horizon=24h|7d|30d&accountId=）：
  - horizon 解析（L80-90）：非法值**静默回退 24h**；accountId 缺省 = `config.default_account_id`（L221-223，无 validate_account，见 §5-12）。
  - "升级后总数"过滤（L232-249）：`final_review_status ∈` 9 值闭集 {approved, revision_applied_approved, revision_failed, held_by_ai_policy, blocked_by_safety_guard, ai_waiting_for_more_context, blocked_by_required_field, blocked_by_budget, blocked_unverified_product_claim}；历史脏值（如 held_for_human）天然不命中即剔除（L230-231）；`legacy_mode_unchecked` **独立计数不进分母**（L40/252-257）。
  - 指标（L392-441）：revisionTriggerRate、revisionPassRate（分母=revision_applied）、aiHoldBreakdown 三类、taxonomyCandidateRate（`review.risks` 前缀正则 `^taxonomy_candidate:`，L313-325）、unverifiedClaimBlockRate、selfCritiqueAddressedRate（分母=revision_applied）、autonomyModeDistribution(auto/assisted/blocked)。**所有比率分母 0 → null**（`ratio` L118-124，R10 SHALL）。rawCounts 全量分子分母平铺。
  - outboxLink（L361-386/429-439）：outbox 同 horizon 的 total/sent/canceled/failed_terminal/delivery_unknown + 各率。
  - planner 段（L135-213）：`agent_events` 单管道 `$group by kind`，11 种 planner 事件 kind 白名单（L47-59），silent/commitment/stagnation 三段 tick/emit/capped/backoff + details.scanned/emitted 汇总。
- `list_autonomy_revisions`（L444-531）：`revision_applied=true` 最近 N 条（limit 默认 50 夹 [1,200]）；逐条回表 contacts 取显示名（remark→nickname→wxid 兜底，L474-490）；pre/post 摘要 Unicode 安全截断 50 字（`excerpt` L534-544）；透出 revisionDirection/holdCategory/finalReviewStatus/selfCritique。

### 2.22 src/routes/llm_providers.rs（1605 行）

- 并发模型（L43-48）：进程级 `LLM_PROVIDER_MUTATION_LOCK`（tokio Mutex）串行化本进程 provider 变更；**跨副本正确性由 Mongo 事务 + revision 谓词提供**（activate 注释 L853-855），多副本部署需分布式租约（文档承认的限制）。
- workspace 解析：所有 handler 走 `resolve_authorized_workspace`（shared.rs:1905-1946）——请求可带 workspaceId 覆盖，但须过调用者 ACL（`workspace_not_in_user_acl` 400）；**空 user_id = 管理 Agent 合成 admin 的可信内部委托，跳过 ACL**（shared.rs:1915-1922，全仓唯一空 user_id 构造点是 management_admin）。
- `list_providers`（L411-439）：api_key 一律 mask（`mask_secret` 前 3+后 4，中间 `****`，L272-281）；附 `active` = LlmRegistry 当前元信息。
- `create_provider`（L500-560）：format 过 `LlmFormat::parse`；providerId/baseUrl/apiKey/model 非空；**apiKey 不得是 mask 占位串**（含 `****` → 400，L519-523）；base_url 去尾斜杠；恒 `is_active=false` 落库；唯一索引冲突 → 400；openai 形态缺 `/v1` 软警告（`base_url_v1_warning` L1330-1345，不阻断）。
- `update_provider`（L562-794）：**激活中的 provider 三重门**（L581-612）：① `expectedUpdatedAt` 必填且等于现行 updated_at（否则 409 revision_changed）② `activeUpdateConfirmed=true` 显式确认 ③ **一次性测试审批令牌**——须先调 test 接口对"即将保存的确切草案"测试成功获得 token，`consume_active_update_approval`（L245-270）校验 token 绑定的 (workspace, provider, admin.user_id, expected_updated_at, **draft_fingerprint**=草案 12 字段 SHA-256 L185-217) 完全一致且未过期（TTL 10min L50）且**单次使用（不匹配也烧毁）**（测试 L1376-1458）。masked apiKey 回传 → 沿用旧值（L475-479）。active 行更新走**全字段快照 CAS filter**（L663-686：name/format/baseUrl/apiKey/model/timeouts/vision 全部等于 pre-read 值）+ 事务 + generation bump + `commit_text_provider_transaction`（L64-117：3 次重试，最终 Unknown 读权威行比对 **runtime_fingerprint** 判定）；提交后 `LlmRegistry.swap` + `mark_database_generation`（L776-782）。`supportsVision=false` 且被并发指派 vision → CAS 失败 409（L687-692/753-757）。`NullablePatch`（L363-401）区分 Missing（不动）/Null（$unset 回落全局默认）/Value。
- `delete_provider`（L796-843）：active → 400"先启用其它"；vision-active → 409；删除 filter 再带 `isActive:false ∧ isVisionActive:false` CAS（L824-836）。
- `activate_provider`（L845-979）：预先 `build_registry_entry`（L1301-1328）构造 client——**非法配置永远成不了 active 指针**（L866）；事务内：`updatedAt` CAS 重读 target（他副本已改 → 409 revision_changed）→ `update_many` 降级其它 active → promote（再带 updatedAt CAS）→ bump generation；commit 同上带 fingerprint 仲裁；**提交后**才 swap 本进程 registry（L967-975），其它副本靠 generation 刷新。
- `set_vision_active`（L1002-1130）：与文字主模型正交（不碰 is_active、不 swap registry，vision client 按需构造，L1000-1001）；active=false 只清本条；active=true 要求 `supportsVision=true`（400），事务内清同 ws 旧指派 + `isVisionActive:{$ne:true}` CAS promote；幂等（已是 vision → 直接 Ok）；commit 仲裁读 `supportsVision:true ∧ isVisionActive:true ∧ updatedAt` 权威行（L119-168）。
- `test_provider`（L1152-1299）：按 providerId（inline 字段可覆盖，masked key 回落 DB 真值）或裸表单（apiKey 必须非 mask）；一次性 client `max_retries=1`（失败即返真实错误，L1231-1233）；**过 `llm_concurrency.acquire(Foreground)` 准入**（L1238-1241）；成功且是 DB provider 场景 → 签发上述审批令牌（L1254-1270）；失败把 `LlmUnavailable{kind,detail,hint,retryCount}` 结构化返回（HTTP 200 + ok:false）。

### 2.23 src/routes/domain_schemas.rs（927 行）

- 版本血缘模型（L9-15）：POST 建 lineage v1（inactive）；PUT 从 `expectedVersion` **追加 inactive 新版**（不可变，L257-313）；DELETE 精确删 `expectedVersion` 的非 active 版（active → 400；delete filter 带 `is_active:false` CAS，L315-361）；activate 事务切唯一 active。
- `validate_schema_payload`（L567-645）红线：fields ≤64；name 非空、不撞 `BASE_FIELD_BLACKLIST`（33 个 chunk 主表字段，L46-81）、schema 内唯一；kind ∈ {string,enum,number,date,reference}（L83）；enum 必须非空 allowedValues；aliasDict 必须 object 且每个 value 是存在的 field name。
- `activate_exact_version`（L413-539，pub，被 HTTP 与写侧复用）：事务内读 target（`version=expectedVersion` CAS）→ 活动行 limit 2 查重（≥2 → 409 multiple_active）→ 幂等短路 → demote 旧 + promote（`is_active:false` CAS）→ commit 无限重试 Unknown。
- `load_active_domain_schema`（L651-673，pub）：0 行 → None（写侧 no-op 直通）；≥2 行 → External 错误（fail-closed）。
- `enforce_domain_attributes`（L690-741，纯函数，chunk 写侧消费）：① alias_dict 别名 key → canonical key 透明改写（canonical 已显式给出时别名项丢弃，L702-705）② required 缺失/Null → 400 ③ enum 值必须字符串且 ∈ allowed_values → 否则 400；**schema 未声明字段原样保留（约束层非白名单，L688-689）**。
- expectedVersion 必须正整数（`require_expected_version` L380-387）。

### 2.24 src/routes/domain_profiles.rs（1634 行）—— 草稿→发布→激活三态

- 核心语义（L9-21）：`release_status`（draft→published）+ `current_version`（血缘发布指针）+ `is_active`（workspace 运行时指针）**三者解耦**。publish/rollout/rollback 只动 current，**绝不改 is_active**；activate 单独动 is_active。故"旧版本 active + 新版本 current"合法并存；AI 生成候选必须人审 activate（红线 L20-21）。
- `append_domain_profile_draft`（L134-219，pub(crate)，create/update/guide 共用）：事务内 `max(version)+1`（checked_add）；current 存在但非 published → 409 `domain_profile_current_not_published`；恒写 draft/非 current/非 active；commit 3 次重试后读权威行仲裁（`commit_domain_profile_transaction` L65-107）。
- `switch_domain_profile_current`（L224-356）：publish（expected_status="draft"）与 rollout/rollback（"published"）共用；事务内 target 状态 CAS、current 唯一性 limit-2 查重、幂等短路、demote+promote（promote 同时把 draft 置 published，L312-318）；draft 却已 current/active → 409 state_invalid（L251-255）。
- `switch_domain_profile_active`（L361-480）：target 必须 `published ∧ current_version=true`（L372-384，非 current 不可激活）；`validate_domain_profile_activation_target`（L113-129：维度校验 + generatedStateMachine 非空 states + `validate_state_machine`——**永久性非法内容零写失败**，瞬态副作用失败才走 partial，L109-112）；demote 旧 active + `is_active:false` CAS promote + **事务内 bump DOMAIN_PROFILE_NAMESPACE generation**（L449-455）。
- handlers：
  - list（L493-517）/get（L542-558）/active（L565-587，limit-2 查重 fail-closed）；投影 `profile_view`（L529-540）：整体 serde 后 **created_at/updated_at 强制转 RFC3339 字符串 + 删 `_id` 换 hex id**——注释 L521-528 记录了裸 bson DateTime 扩展 JSON 导致前端白屏的历史事故，测试 L1582-1613 锁 wire 形态；
  - create（L607-638）：profileId 非空且 canonical（无首尾空白）；后端管理字段强制覆盖（version=0 由 append 分配、seeded_by 默认 "manual"）；
  - update（L642-677）：**不可变编辑**——从选中版本 merge `strip_backend_managed_keys`（L1155-1178：剥 12 个后端管理键；只 merge body 出现的键 → 未编辑字段不清零）后**追加新 draft**（previous_version=existing.version，seeded_by="manual"）；`UpsertRequest.profile_id` 带 `#[serde(default)]`（前端 PUT 不含 profileId，L592-596 注释 + 测试 L1357-1375）；
  - delete（L680-717）：只允许删 `draft ∧ !current ∧ !active`（400），delete filter 全条件 CAS（409 draft_changed）；
  - publish（L722-774）：只收 draft（400）；响应带 `requiresActivation:true` + **`riskyFields`**——与当前 active 行对比 13 个危险字段（`RISKY_FIELD_NAMES` L1083-1097：soul_override/methodology_override/conversation_mode_policy/commitment_markers/conversation_modes/operation_mode/grounding_gate_bypass_without_claim/distrust_self_reported_low_risk/outcome_polarity/threshold_overrides/transaction_facts_enabled/reviewer_orientation/mode_gate_policy_override；`risky_fields_changed` L1105-1147 纯函数）；
  - rollout（L778-812）：只收 published；rollback（L815-873）：source 必须是 published current（409），经 previous_version 找目标（必须 published）；
  - **activate（L878-1074）**：核心指针事务提交后：① 立即失效进程 domain_profile 缓存（L889-890）② `publish_state_machine_version`（profile 无内嵌机器时**显式发布系统默认机器**而非保留旧行业机器，L900-907）→ 返回 StateMachinePublishReport → statePolicies completed|partial ③ **H13 幻影态迁移**：`operation_state` 已设且 `$nin` 新机器 key 集的存量 contact 批量重置到新机器 initial 态（附 reason 标记 + 清 confidence，L948-1046）；**`new_keys` 为空时硬跳过**（`$nin:[]` 会匹配全量的防御，L972-985）。三步全 best-effort：任一失败 → 响应 `status:"partial", retryable:true` + errors 数组（重试激活可补偿），profile 激活本身不回滚。

### 2.25 src/routes/guide_profile.rs（944 行）—— AI 生成候选 profile（挂 /admin/domain-profiles/generate）

- 红线（L9-13）：AI 生成的 profile = 候选草稿（draft/非 current/非 active，`seeded_by="generated_by_ai"` L418），必须人审 publish+activate；生成器 system 走 active profile 的 `methodology_generator_preamble`，缺省回落领域中性 `PLAYBOOK_METHODOLOGY_SYSTEM`（L344-355）。
- `generate_domain_profile_candidate`（L324-489）：businessDescription/profileId 非空（400）；拉最近 40 条知识切片标题作行业线索（只标题控 token，L193-232）；`generate_agent_json` 出草案后的**归一化流水**：
  1. **stateMachine 顶层抽出绕过 snake_case 归一**（L376-387）：引擎读 camelCase `allowedFrom/allowFromAny/initial`，若过 `normalize_json_keys` 会被 mangle 成 snake 导致引擎静默读不到——测试 L883-943 以正反两证钉死此不变量；抽出后过 `validate_state_machine`，不合法 → 回落 None（warn，运行时回落 DEFAULT，不阻断生成，L426-455）；
  2. **流 C**：normalize 前 `extract_suggested_values`（L138-179）取各维度 AI 建议取值并 remove 该键（防污染反序列化）；
  3. `normalize_json_keys`（L62-77 递归 camel→snake；`to_snake_case` L42-59 已知限制：末尾连续大写不分隔，测试 L530-535 锁定）+ `coerce_scalar_string_fields`（L92-125：LLM 偶发把 description/prompt_fragment/soul_override/methodology_override/conversation_mode_policy 及嵌套 `profile_dimensions[].description` 给成对象/数组 → 压平成 JSON 文本防 from_document 失败，G32）；
  4. 后端管理字段强制注入/覆盖（L396-420）→ `append_domain_profile_draft` 落库；
  5. 建议取值**落 taxonomy_candidates 候选层**（`upsert_candidate` scope="global"、confidence=10 上界档、suggestedDisplayName=label；**绝不直进 system_taxonomies**——守"AI 永不自动 verify"；失败 `let _` 软化不阻断，L460-480）。

### 2.26 src/routes/lessons_learned.rs（640 行）

- `list_lessons_learned`（L60-91）：GET /admin/lessons-learned?patternKind=&limit=。workspace 强制；patternKind 精确匹配（**server 侧不设白名单**，让上游 schema 自由演进，L15-16）；limit 默认 50 夹 [1,200]；裸 Document 投影 `lesson_doc_to_json`（L430-489，缺字段安全默认，review_status 缺省 "pending_review"）。
- `promote_lesson_to_peer_case`（L147-190 → `promote_lesson_once` L192-422）：POST :lesson_id/promote-to-peer-case。
  - 输入校验 `validate_promote_request`（L109-136）：title 非空 ≤200 字符、body 非空 ≤4000 字符（chars 计数，中文安全，测试 L629-639）、summary 空白折叠 None。
  - **幂等/一对一身份**：lesson `review_status="promoted"` 时校验 `promoted_chunk_id` 指向的 chunk 存在且 `provenance.source="lesson_promotion" ∧ source_doc_id=lesson_id`（不匹配 → 409 identity_mismatch），命中 → 返回已有 chunk_id `alreadyPromoted:true`（L232-258）；非 pending_review 或已有 promoted_chunk_id → 409。
  - **事务**（L202-421）：① 插入 chunk——**`_id` 复用 lesson 的 ObjectId**（L279-281，天然防重复插入），`chunk_type="peer_case"`、`status="draft"`、`integrity_status="needs_review"`（**不绕 chunk review queue**，红线 L10-13）、business_context=`lessons_learned::<pattern_kind>`、provenance 完整；② `apply_chunk_revision_with_session`（Create/Human 修订流水）；③ lesson CAS（pending_review/缺失/null + 无 promoted_chunk_id）置 promoted + 回填 chunk hex；④ 同事务写 `agent_events` kind=`lesson_promoted_to_peer_case`。
  - 外层重试（L160-189）：TransientTransactionError / duplicate-key 最多 5 次、20ms 间隔（并发双 promote 收敛到同一 chunk id）；耗尽 → 409。

### 2.27 src/routes/chunk_locks.rs（541 行）—— presence + WebSocket 协议

- 定位（L4-9）：presence 是**进程内 DashMap 协作提示**（TTL 300s，L47；心跳建议 60s），不授予写权不阻止提交；真并发保护靠 mutation 的事务/CAS。事件总线 broadcast 容量 256（L51），慢订阅者丢老事件由前端 reload 自愈。
- `acquire_chunk_lock`（L219-270，POST /operation-knowledge/chunks/:id/lock）：body 是 **`deny_unknown_fields` 空对象**（L98-100）；先 `ensure_chunk_in_workspace`（L191-212：`_id+workspace` 存在性，防跨租户探测/占位）；`acquire_presence`（L122-166，DashMap entry 原子）：未过期且他人持有 → **409** `chunk_presence_by_other`（附 advisory:true + 当前 lock）；本人续期 refreshed=true（保留 locked_at）；过期任何人可接管。成功广播 `Locked` 事件。
- `release_chunk_lock`（L275-318，DELETE 同路径）：不存在/已过期 → 200 `released:false`（幂等）；他人持有 → **403** `presence_owned_by_other`；本人 → remove + 广播 `Unlocked`。测试 L492-539 锁 workspace 复合 key 隔离与"旧 owner 不能删新 owner"。
- **WebSocket `GET /ws/chunks`**（L324-386）：经 require_session（cookie 随同源握手携带）；升级后立发 `{"kind":"hello","workspace":…}`；循环 select：广播事件 → **按 workspace 过滤**（`event_workspace` L388-394）→ 序列化推送；`RecvError::Lagged` → 推 `{"kind":"lagged"}`（前端 reload）；Closed/客户端 Close/发送失败 → 结束。客户端上行文本一律忽略（L376-383）。
- `ChunkEvent`（L71-92）：tag=`kind`，snake_case → `locked{chunk_id,workspace_id,owner_user_id,owner_username,expires_at}` / `unlocked{…}` / `revised{chunk_id,workspace_id,revision_kind,actor}`（测试 L434-472 锁 wire 形态）。
- `broadcast_chunk_revised_in`（L413-426）：知识编辑路径（wiki_edit.rs/crud.rs 共 12 处调用，Grep 亲验）完成修订后推 `Revised`。**`broadcast_chunk_revised`（L398-410）零调用方且 workspace_id 写死空串**——见 §5 偏差 6。

### 2.28 src/routes/worker_controls.rs（136 行）

- **独立 ACL**：`require_system_operator`（L18-26）——`admin.username` 必须**精确大小写匹配** `config.system_operator_usernames`（`SYSTEM_OPERATOR_USERNAMES` env CSV，默认空，config.rs:359/786-789）；**空名单 = 全拒（fail-closed）**（测试 L89-94）；否则 403 `system_operator_required`。这是全仓唯一超出"登录即全权"的路由级权限门。
- `list_worker_controls`（L51-71）：`background_worker_controls` 全量（**无 workspace 过滤**——worker 熔断是进程/部署级资源，非租户数据）；投影 `worker_control_json`（L36-49）**显式白名单**：workerName/status/rapidPanicCount/circuitGeneration/hasPanicDiagnostic(bool)/时间戳/resumeRequestedBy——**probe_token 与 last_panic 原文绝不外泄**（L34-35，测试 L96-115）。
- `resume_worker_control`（L73-82）：POST /admin/worker-controls/:worker/resume → `supervisor::resume_worker_circuit`（supervisor.rs:252-288 亲验）：worker 必须 ∈ `SUPERVISED_WORKERS`（否则 400）；CAS `status:"open"` → `half_open` + 记录 resume_requested_by/at + `$unset` probe_token/probe_locked_until/probe_started_at；返回 `resumed = modified_count==1`（非 open 态调用 → false，不报错）。

### 2.29 src/routes/evolution.rs（1453 行）

- 隔离红线（L12-15）：每个 handler 顶部贴 `// FORBIDDEN: enqueue agent_send_outbox / mcp call` anchor（L70/203/253/311/719/739/811 亲验），配合 CI lint。
- `list_evolution_experiments`（L65-112）：limit 默认 20 夹 [1,100]；逐 experiment 拉 proposal 摘要；附 `aggregate7d`（L117-195）：**服务端完整 7 日窗**独立于列表分页（L114-116），统计 experiments/proposals/released/rolledBack/significancePassRate（无评估 → null）+ coverage 元数据。
- `get_evolution_proposal_detail`（L198-244）：proposal（ws 过滤 404）+ experiment 信封 + `cohortRunIds`（按 kind 取 threshold/prompt cohort，L635-652）+ shadowReplays 聚合（completed/failed 计数 + 前 5 样本，L389-425）+ `currentState` diff 对照（L427-512：threshold → 最新未回滚 override 或 **AppConfig 内置基线**（`baseline_threshold_value` L514-524：fact_risk 6.0 / pressure 7.0 / human_like 6.0 / emotional 6.0 / product_accuracy 7.0，与 5 闸规则同源）；prompt → current_version 模板内容/版本）。
- `release_evolution_proposal`（L247-302）/ `rollback_evolution_proposal`（L305-360）：**confirmation 必须是精确字面量 "RELEASE"/"ROLLBACK"**（L45-46/254-258/312-316，否则 400）；按 `proposal_kind ∈ {threshold, prompt}` dispatch 到 `evolution::release::{release,rollback}_{threshold,prompt}`（操作者 = `admin.username`）；未知 kind → 400。`EvolutionError` 映射（L526-540）：InvalidStatus/RedlineGateRejected → 400，Mongo → Db，Budget/Internal → External。
- `get_evolution_runtime_flag`（L715-731）：文档缺失返回逻辑默认"未配置=关停"（**读路径零写**，L712-714）；附 `envEvolutionEnabled`（env 是最外层熔断）。
- `put_evolution_runtime_flag`（L734-794）：rollout_percent 钳 ≤100；**`thresholdAutoReleaseEnabled=true` 且当前人工发布政策（HC-017 `CURRENT_AUTO_RELEASE_POLICY_ENABLED=false`）→ 400 拒绝**（L749-756）；None 不改/false 可显式清；upsert 后回读返回。**`updated_by` 来自请求体**，空则落常量 "admin"（L742-746）——见 §5 偏差 8。
- `list_threshold_override_audit`（L806-836）：gateKey 过滤 + limit 默认 50 夹 [1,200]，按 decided_at 倒序；投影含 previous/new value、decidedBy、hitRateObserved、significanceMetrics（Phase C/C5 不可变审计）。

### 2.30 src/routes/simulations.rs（371 行）

- `simulate_user_operation_dialogue`（L37-70，POST /user-operations/simulations/dialogue）：`validate_account`（ws 内账号存在性）；**`apply_memory=true` → 400 "shadow simulation cannot apply memory yet"**（L43-47，影子模式硬门）；messages trim 去空、**最多取 12 条**（L48-54），空 → 400；contact 必须属于该 account（400）；委托 `agent::simulate_user_dialogue`（走生产 gateway→review 同源链路）。响应 `runMode:"shadow", applied:false`。
- `run_user_operation_evaluation`（L72-138）：场景集按 active profile 的 `transaction_facts_enabled` 分交易域/关系域两套（`evaluation_scenarios` L140-190，各 4 场景：reject_intro/buying_interest/product_question/silence_follow_up）；可按 scenario 名过滤（未知 → 400）、maxScenarios 截断（min 1）。
- `judge_user_operation_scenario`（L192-254）：**不自算阈值**——直接读生产终态 `turn.status`（S1.3 重构，L206-221 注释记录旧 0-100 阈值与 reviewer 0-10 档错配的死闸历史）：`would_send|no_reply` → passed；review_blocked/gateway_blocked/blocked_by_safety_guard/blocked_unverified_product_claim/held_by_ai_policy 及一切其它终态 → failed + issue 文案（测试 L316-346 锁"非发送终态绝不假绿"）。scores 仅透传展示。

### 2.31 src/routes/contract_snapshot.rs（246 行，`#[cfg(test)]`）

- 前后端契约快照机制（L1-8）：每个投影函数配测试调 `assert_contract_fixture`（L47-79）→ 与 `frontend/src/contracts/<name>.fixture.json` 逐字节对账（canonicalize 递归排键，L15-29）；`UPDATE_SNAPSHOTS=1` re-bless 写文件。fixture 是前后端唯一真相源。
- `every_projection_has_contract_test`（L106-245）：运行时扫描 `src/routes/**` 所有 `-> Value` 的 `*_json`/`*_view` 函数，断言每个非豁免投影都被某个含 `assert_contract_fixture` 的 `#[test]` 块覆盖；ALLOWLIST 7 项逐条注明理由（L111-119）；**后缀集含 `_view` 的原因**：`profile_view` 曾漏扫致 `$date` 白屏事故（L132-138 历史教训注释）。注释 L103-104 自证："现有 no_orphan_pub_async_route_handlers (mod.rs) 手维护清单已腐烂,故用运行时扫描"。

### 2.32 src/routes/management_prompt_edit.rs（7 行）

纯 re-export：`pub(super) use crate::prompt_guard::{review_prompt_edit, validate_prompt_edit, PromptEditVerdict}`（L7）。实现已下沉 `crate::prompt_guard`（管理员手动编辑与 evolution release 共用），本文件仅保调用路径不破（L3-5）。无路由。

---

## 3. 跨文件机制

### 3.1 一次登录到带权请求的完整链路

```
① POST /api/auth/login {username, password}
   auth.rs:51-93
   ├─ 白名单直通 middleware（middleware.rs:33 "/auth/login"）
   ├─ authenticate_public_endpoint（auth.rs:254-301）
   │   ├─ auth_rate_limiter.begin(直连IP, 小写化用户名)   ← Argon2 之前占槽（rate_limit.rs:87-93）
   │   │   三维：client≤20 / target≤10 / global(pending)≤100，窗 300s（config.rs:790-797）
   │   │   拒绝 → AppError::AuthRateLimited{retry_after} + 每窗口一次审计（auth_security_events）
   │   ├─ authenticate（session.rs:109-136）
   │   │   用户不存在 → verify_against_dummy 抗时序（password.rs:49-51）→ 恒 InvalidCredentials
   │   └─ mark_success（清同 client+target 失败记录）/ mark_invalid（留窗计数）
   ├─ create_session（session.rs:141-163）
   │   token=UUIDv4 → 库存 sha256-v1 摘要；current_workspace = default_workspace∩ACL 或 ACL[0]
   │   ACL 空 → NoAuthorizedWorkspace → 401（登录被拒，session 不落库）
   ├─ ensure_workspace_taxonomies（auth.rs:80）
   └─ Set-Cookie wa_session=<raw>; HttpOnly; SameSite=Strict; [Secure]; Path=/; Max-Age=168h

② 之后每个 /api/* 请求
   middleware.rs:36-121（layer 于 mod.rs:1062-1065）
   cookie → lookup_session（摘要优先查、明文行透明迁移、expires_at 双保险）
          → get_admin_user（每请求重查 → ACL 改动即时生效）
          → is_workspace_authorized(session.current_workspace, user.workspaces)
          → 注入 Extension<AuthenticatedAdmin>{user_id, username, current_workspace}
   cookie 失效且 JWT_ENABLED → Bearer verify_jwt(RS256, leeway=0) → 同样重查 user+ACL
   全 miss → 裸 401

③ handler 内的租户边界
   一切查询强制 admin.current_workspace（如 admin_outbox.rs:68、observability.rs:150）
   可带 workspaceId 覆盖的路由（llm_providers/domain_schemas/domain_profiles）走
   resolve_authorized_workspace（shared.rs:1905-1946）再过一次 ACL；
   worker_controls 额外叠加 system-operator 精确用户名门（worker_controls.rs:18-26）

④ 切换 workspace：POST /api/auth/workspace（auth.rs:132-161）
   校验 target∈ACL → 原地更新 session.current_workspace；JWT 用户则需重新 POST /auth/token 换签
```

### 3.2 一次配置发布的版本协议（三个变体）

**变体 A：ops 三表单-current（admin_ops_versions.rs）**——同一 scope 恒一条 `current_version=true`：
publish = 事务 {CAS 校验 current 未变 → max+1 新行 → demote 旧} ；rollout/rollback = 事务 {demote current → promote target}；taxonomy 变体在同事务 bump workspace generation → 提交后失效本进程缓存，他副本按 generation 懒重建；改动写 `taxonomy_version_changed` 审计事件（fail-soft）。运行时读方只认 `current_version=true`。

**变体 B：domain_profiles 三态解耦（domain_profiles.rs）**——发布指针与运行时指针分离：
draft（不可变，编辑=追加新 draft）→ publish（draft→published + 切 current，**响应列 13 危险字段 diff** 供激活前审阅）→ activate（唯一入口改 is_active；事务内 bump DOMAIN_PROFILE generation）→ 提交后三个 best-effort 附属步骤（状态机 publish【复用变体 A 的 `publish_state_machine_version`，含 no-op 幂等 + policy 重派生】→ 存量 contact 幻影态迁移），失败返回 `partial/retryable` 由 admin 重试激活补偿。AI 候选（guide_profile）与人工草稿走同一条链，`seeded_by` 区分。

**变体 C：llm_providers 主动式激活（llm_providers.rs）**——指针切换叠加"内容先验"：
测试成功 → 一次性审批令牌（绑 draft 指纹）→（改 active 行时）expectedUpdatedAt + 显式确认 + 令牌三重门 → 事务 {updatedAt CAS demote+promote + generation bump} → commit 不确定时读权威行按 runtime_fingerprint 仲裁 → 提交后 swap 进程 LlmRegistry + mark generation。vision 指派是正交的第二指针。

共性：全部要求 MongoDB replica set 事务；commit `UnknownTransactionCommitResult` 的处理分两派——无限重试（ops 三表/候选审批/suspected/relationship/lessons/domain_schemas）与有限 3 次 + 读权威行仲裁（taxonomy CRUD/llm_providers/domain_profiles）。

### 3.3 人审闭环的公共形状

taxonomy_candidates / relationship_suggestions / suspected_deals / lessons_learned / evolution proposals / domain profile drafts 六条审核链共享：pending 默认视图 → approve 走事务 + pending CAS（并发只有一人成功）→ reviewed_by 恒来自服务端身份（ReviewActor / admin.username），请求体不可伪造 → reject 必填 reason → ask_human_inbox 以同一 filter 聚合计数。AI 产物（候选、建议、信号、lesson、AI profile）一律停在候选层等人审——"AI 永不自动 verify" 在本组的每个入口都成立。

---

## 4. 事实卡速查

**公开路径白名单**（middleware.rs:32-34；完整 URL 前缀 /api，main.rs:357）：
`/api/health`、`/api/auth/login`、`/api/auth/token`。此外 `/webhooks/wechat` 不在 /api 下、不经 admin auth（main.rs:358-361）；前端静态资源走 fallback。其余 /api/* 一律需 cookie session 或（JWT_ENABLED 时）Bearer。

**鉴权与限流参数**（config.rs 亲验）：
| 参数 | env | 默认 | 出处 |
|---|---|---|---|
| session TTL | SESSION_TTL_HOURS | 168h | config.rs:778 |
| cookie Secure | SESSION_COOKIE_SECURE | false | config.rs:779 |
| bootstrap 管理员 | BOOTSTRAP_ADMIN_USERNAME/PASSWORD | 无 | config.rs:780-785 |
| 限流窗口 | AUTH_RATE_LIMIT_WINDOW_SECONDS | 300s | config.rs:790 |
| client 容量 | AUTH_RATE_LIMIT_CLIENT_CAPACITY | 20 | config.rs:792 |
| target 容量 | AUTH_RATE_LIMIT_TARGET_CAPACITY | 10 | config.rs:794 |
| global(pending) 容量 | AUTH_RATE_LIMIT_GLOBAL_CAPACITY | 100 | config.rs:796 |
| JWT 开关 | JWT_ENABLED | false | config.rs:801 |
| JWT TTL | JWT_TTL_MINUTES | 60min | config.rs:802 |
| JWT 密钥 | JWT_PRIVATE/PUBLIC_KEY_PEM | 无（开则必填） | config.rs:803-807 |
| 默认 workspace | DEFAULT_WORKSPACE_ID | "default" | config.rs:477 |
| system operator | SYSTEM_OPERATOR_USERNAMES | ""（全拒） | config.rs:786-789 |
| 审计保留 | （硬编码） | 90 天 | rate_limit.rs:21 |
| presence TTL | （硬编码） | 300s | chunk_locks.rs:47 |
| provider 测试审批 TTL | （硬编码） | 10min | llm_providers.rs:50 |

**全部 admin/管理面端点表**（本组；挂载行=mod.rs）：

| 端点 | 方法 | handler（file:line） |
|---|---|---|
| /health | GET | health.rs:8 |
| /auth/login, /auth/logout, /auth/me, /auth/workspace, /auth/token | POST/POST/GET/POST/POST | auth.rs:51/95/108/132/209 |
| /admin/worker-controls[,/:worker/resume] | GET/POST | worker_controls.rs:51/73（system-operator 门） |
| /admin/taxonomies[,/:id] | GET,POST/PATCH,DELETE | admin_taxonomies.rs:145/184/284/410 |
| /admin/taxonomies/:id/{publish,rollout,rollback} | POST | admin_ops_versions.rs:1062/1117/1149 |
| /admin/taxonomy-candidates[,/:id/approve,/:id/reject] | GET/POST/POST | admin_taxonomy_candidates.rs:77/118/142 |
| /admin/relationship-type-suggestions[,/:id/approve,/:id/reject] | GET/POST/POST | admin_relationship_suggestions.rs:64/100/266 |
| /admin/suspected-deals[,/:id/approve,/:id/reject] | GET/POST/POST | admin_suspected_deals.rs:72/108/222 |
| /admin/operation-state-policies[,/:id] | GET/GET | admin_state_policies.rs:45/80 |
| /admin/operation-state-policies/:id/{publish,rollout,rollback} | POST | admin_ops_versions.rs:926/980/1009 |
| /admin/operation-domains/:id/{publish,rollout,rollback} | POST | admin_ops_versions.rs:442/811/857 |
| /admin/principal-escalations[,/:sc/resolve,/:sc/reassign] | GET/POST/POST | principal_escalations.rs:26/163/259 |
| /admin/ask-human/{inbox,summary} | GET | ask_human_inbox.rs:675/733 |
| /admin/outbox[,/:id/cancel] | GET/POST | admin_outbox.rs:62/123 |
| /admin/lessons-learned[,/:lesson_id/promote-to-peer-case] | GET/POST | lessons_learned.rs:60/147 |
| /admin/observability/{phase-rollup,performance,worker-health} | GET | observability.rs:497/117/1041 |
| /admin/llm-providers[,/:id] | GET,POST/PUT,DELETE | llm_providers.rs:411/500/562/796 |
| /admin/llm-providers/:id/{activate,vision}, /admin/llm-providers/test | POST | llm_providers.rs:845/1002/1152 |
| /admin/domain-schemas[,/:id,/:id/activate] | GET,POST/PUT,DELETE/POST | domain_schemas.rs:172/200/257/315/363 |
| /admin/domain-profiles[,/active,/:id] | GET,POST/GET/GET,PUT,DELETE | domain_profiles.rs:493/607/565/542/642/680 |
| /admin/domain-profiles/:id/{publish,rollout,rollback,activate} | POST | domain_profiles.rs:722/778/815/878 |
| /admin/domain-profiles/generate | POST | guide_profile.rs:324 |
| /evolution/experiments, /evolution/proposals/:id[,/release,/rollback] | GET/GET/POST/POST | evolution.rs:65/198/247/305 |
| /evolution/threshold-overrides/audit, /evolution/runtime-flag | GET / GET,PUT | evolution.rs:806/715/734 |
| /operation-knowledge/chunks/:id/lock | POST,DELETE | chunk_locks.rs:219/275 |
| /ws/chunks | GET(WS) | chunk_locks.rs:324 |
| /behavior-signal-metrics | GET | behavior_signal_metrics.rs:30 |
| /outcomes/autonomy[,/revisions] | GET | outcomes_autonomy.rs:215/444 |
| /user-operations/simulations/dialogue, /user-operations/evaluations/run | POST | simulations.rs:37/72 |

**status/枚举闭集**（均亲验）：
- OutboxStatus：`pending, in_flight, sent, failed_terminal, canceled, delivery_unknown`；admin 可取消集 `{pending, in_flight}`（admin_outbox.rs:173-176 + 测试 544-557；旧 `failed` 非法）。
- escalation status：`pending | resolved | delivery_failed`（models.rs:4491-4499）；verdict：`approved | rejected | conditional | deferred | delegated_back`（models.rs:4563-4576）；exemption：`none | customer_only | knowledge`（models.rs:4578-4580）；授权窗 (0, 8760] 小时（principal_escalations.rs:66/133-145）。
- 候选/建议/信号审核态：`pending → (approving →) approved | rejected`（approving 仅 taxonomy_candidates 事务内瞬态，admin_taxonomy_candidates.rs:287）。
- lifecycle 闭集 7 常量：STARTED/RUNNING/COMPLETED/FAILED_BEFORE_DECISION/FAILED_AFTER_DECISION/ABORTED_BY_BUDGET/ABORTED_BY_EXTERNAL_SIGNAL（observability.rs:35-39 引 run_envelope；字面量属 04/06 号范围）。
- hold 三类：`held_by_ai_policy, blocked_by_safety_guard, ai_waiting_for_more_context`（observability.rs:687-693）。
- outcomes"升级后" final_review_status 9 值 + `legacy_mode_unchecked` 独立（outcomes_autonomy.rs:232-249/40）。
- post_decision_status：`prepared,pending,retry,processing,completed,failed_terminal,discarded`（observability.rs:1102-1110）。
- gap signal status：`pending,auto_resolved,llm_resolved,applied,dismissed`（observability.rs:1362-1368）。
- lessons：pattern `success|reviewer_misjudge_negative|blocked_by_safety_guard`；review_status `pending_review → promoted`（observability.rs:1510-1514；lessons_learned.rs:232/259）。
- knowledge 待审：`needs_review, needs_human_audit`（ask_human_inbox.rs:137-139）。
- chat task status：`pending,running,completed,failed,cancelled`（models.rs:5934-5935）。
- proposal status：`pending_eval,evaluating,eligible_for_release,rejected_below_threshold,released,rolled_back`（evolution.rs:545-552）；确认串 `"RELEASE"/"ROLLBACK"`（evolution.rs:45-46）。
- domain profile：release_status `draft|published` × current_version × is_active 三轴（domain_profiles.rs:9-18）。
- performance path：`direct,escalated,rewrite,revision,no_reply,manual`（observability.rs:51-58）。
- domain schema kind：`string,enum,number,date,reference`；fields≤64（domain_schemas.rs:83/571）。
- 5 闸内置基线：fact_risk 6.0 / pressure 7.0 / human_like 6.0 / emotional 6.0 / product_accuracy 7.0（evolution.rs:514-524）。
- 分页/限幅：outbox 50/200；lessons 50/200；experiments 20/100；threshold audit 50/200；autonomy revisions 50/200；behavior metrics 60/365；审核列表硬 500；inbox 各源 50-100；performance 采样 20_000。

---

## 5. 偏差与疑点

1. **【偏差·过期注释】`TaxonomyEntry/TaxonomyCandidate` "无 workspace_id" 的说法已失实**。admin_ops_versions.rs:1235-1237（"TaxonomyEntry 无 workspace_id、只有 scope"）与 admin_taxonomy_candidates.rs:191-192 的注释，与代码矛盾：两模型均有 `workspace_id` 且所有 filter 都带它（admin_ops_versions.rs:1077/1086、admin_taxonomy_candidates.rs:261/388）。注释残留自 workspace 字段引入前，读者据此推断隔离边界会得出错误结论。
2. **【偏差·护栏缺口】mod.rs 死路由 tripwire 的 include 名单不全**（mod.rs:1083-1129）：缺 campaigns.rs、ask_human_inbox.rs、principal_escalations.rs、domain_profiles.rs、guide_profile.rs、media_assets.rs、referral_cards.rs、send_ledger.rs、operation_view.rs、worker_controls.rs、management_prompt_edit.rs——这些文件新增 `pub async fn` 忘挂载不会被该测试抓到。contract_snapshot.rs:103-104 的注释自证该手维护清单"已腐烂"（投影护栏因此改为运行时扫描，但路由挂载护栏没有同步重构）。
3. **【疑点·wire 形态不一致】`principal_escalations::list` 与 `ask_human_inbox::InboxItem` 把裸 `bson::DateTime` 直接序列化**（principal_escalations.rs:52/54 的 `createdAt`/`authorizationExpiresAt`；ask_human_inbox.rs:25 的 `created_at`），wire 上会是扩展 JSON `{"$date":{"$numberLong":…}}` 对象——正是 domain_profiles.rs:521-528 记录的、曾把「行业配置」页整页白屏的同款形态（该处已修为 RFC3339 并加契约测试，本两处未修）。两处都同时下发了 `ageHours`/`age_hours` 数值，前端很可能只消费后者，故未爆雷；是否真被前端解析须由 13/14 号前端任务核证。
4. **【疑点·聚合口径措辞】gap_signals 的"sweep 命中率"实际不存在**：observability.rs:1409-1410 明确注释输出的是 `historicalResolvedShare`（保留历史中已解决状态占比），集合无 run 标识、无法反推单轮 sweep 命中率。但同文件 observability.rs:1342 的行内注释（"auto_resolved/applied/dismissed 之比是 sweep 命中率"）与 mod.rs:974 的挂载注释仍沿用旧口径——三处相互矛盾，以 L1409-1410 与实际输出键名为准。
5. **【偏差·过期注释】observability.rs:1037 说 worker_health "workspace_id 强制 default"**，实际代码用 `admin.current_workspace`（observability.rs:1045）。多租户行为正确，注释未随改。
6. **【偏差·死代码+误导注释】`chunk_locks::broadcast_chunk_revised`（chunk_locks.rs:398-410）**：`workspace_id: "".into()` 且注释称"调用方覆盖"——broadcast send 后不可能再覆盖；空 workspace 事件会被 WS 端 `event_workspace != workspace` 过滤，永不送达。全仓 Grep 亲验零调用方（12 处调用全是 `broadcast_chunk_revised_in`）。属可删除的死函数。
7. **【疑点·部署相关】限流的 client 维度只认 TCP 直连 IP**（auth.rs:242-246，不解析 X-Forwarded-For）。若部署在反向代理后，所有真实客户端共享代理 IP 的单一 client 指纹（容量 20/5min），登录高峰可能互相挤兑；直连部署则语义正确。这是设计取舍还是遗漏，代码内无注释说明——需结合部署拓扑判断。
8. **【偏差·审计字段可伪造】`put_evolution_runtime_flag` 的 `updated_by` 取自请求体**（evolution.rs:704-705/742-746），缺省落常量 `"admin"`，未用 `admin.username`。同仓已有明确先例把审计身份服务端化（ReviewActor，shared.rs:30-32"inner value cannot be supplied by request JSON"；SR-058 测试）。灰度旗改动的 who 可被客户端指定任意字符串——与 release/rollback（用 admin.username，evolution.rs:276/285）不一致。
9. **【事实·刻意设计，易被误读】REST 侧 `approve_taxonomy_candidate` 无 scope 校验**：同 workspace 任一 admin 可 approve 任何账号私有 scope 的候选；scope 门只在管理 Agent 工具侧入口存在（admin_taxonomy_candidates.rs:194-205 注释声明"维持现状不回归"）。在无 RBAC 的模型下自洽，但若未来引入账号级权限需回补。
10. **【事实·需知】`/ws/chunks` 的鉴权依赖 cookie 随 WebSocket 握手发送**（mod.rs:575 在 require_session layer 内）；SameSite=Strict 下仅同源页面可建立连接；JWT-only 客户端（浏览器 WS 无法带 Authorization 头）无法订阅该总线。
11. **【偏差·风格】reject 类 handler 的终态回读缺 workspace 过滤**（admin_relationship_suggestions.rs:302-304、admin_suspected_deals.rs:258-260 仅按 `_id` 回读）。前一步 update 已在本 workspace matched=1，回读的必是同一行，**无越权后果**；但与同文件"绝不泄漏存在性"的查询风格不一致，若未来有人复制该模式到先读后写场景会成真问题。
12. **【疑点·轻微】`outcomes_autonomy` 的 accountId 参数不做 `validate_account`**（outcomes_autonomy.rs:220-223），任意字符串按空数据返回（workspace 过滤仍在，无跨租户泄漏）；horizon 非法值静默回退 24h 而非 400（L80-90）——与 observability 对 path/hours 的严格 400 风格不同。
13. **【事实·需知】CORS 全开**（main.rs:365-369 `allow_origin(Any)`）：cookie 模式因浏览器不允许 wildcard+credentials 且 SameSite=Strict 而实际不可跨站；JWT 模式本就允许任意来源持 token 调用。配置本身未见风险，但若未来改 allow_credentials 需警惕。
14. **【疑点·未读透传值】`OPERATION_STATE_ACTION_VALUES`（admin_state_policies.rs:76）与 `derive_state_policy_lists`（admin_ops_versions.rs:562）的具体取值**位于 `src/agent/`（04 号）与 `src/db/migrations/m013`（02 号）范围，本组仅核证了引用关系与用途，未逐行读其定义。
15. **【事实·需知】`llm_providers` 的进程锁在多副本下不保证互斥**（llm_providers.rs:43-48 文档自认），正确性兜底是事务+CAS；届时同时激活的两副本一个会拿到 409。当前单进程部署无碍。

---

## 6. 覆盖自证

逐行读完（100% 全文，行数 = `wc -l` 当日值）：

| # | 文件 | 行数 |
|---|---|---|
| 1 | src/auth/mod.rs | 108 |
| 2 | src/auth/middleware.rs | 147 |
| 3 | src/auth/session.rs | 268 |
| 4 | src/auth/password.rs | 94 |
| 5 | src/auth/jwt.rs | 178 |
| 6 | src/auth/rate_limit.rs | 450 |
| 7 | src/routes/auth.rs | 328 |
| 8 | src/routes/health.rs | 15 |
| 9 | src/routes/admin_ops_versions.rs | 1390 |
| 10 | src/routes/admin_outbox.rs | 827 |
| 11 | src/routes/admin_relationship_suggestions.rs | 427 |
| 12 | src/routes/admin_state_policies.rs | 233 |
| 13 | src/routes/admin_suspected_deals.rs | 390 |
| 14 | src/routes/admin_taxonomies.rs | 643 |
| 15 | src/routes/admin_taxonomy_candidates.rs | 695 |
| 16 | src/routes/principal_escalations.rs | 309 |
| 17 | src/routes/ask_human_inbox.rs | 1202 |
| 18 | src/routes/observability.rs | 1799 |
| 19 | src/routes/behavior_signal_metrics.rs | 103 |
| 20 | src/routes/outcomes_autonomy.rs | 544 |
| 21 | src/routes/llm_providers.rs | 1605 |
| 22 | src/routes/domain_schemas.rs | 927 |
| 23 | src/routes/domain_profiles.rs | 1634 |
| 24 | src/routes/lessons_learned.rs | 640 |
| 25 | src/routes/chunk_locks.rs | 541 |
| 26 | src/routes/worker_controls.rs | 136 |
| 27 | src/routes/evolution.rs | 1453 |
| 28 | src/routes/contract_snapshot.rs | 246 |
| 29 | src/routes/simulations.rs（兜底：11 号清单未列） | 371 |
| 30 | src/routes/management_prompt_edit.rs（兜底，可能与 11 号"management"重叠） | 7 |
| 31 | src/routes/guide_profile.rs（兜底，挂 /admin/domain-profiles/generate，可能与 11 号"guides"重叠） | 944 |
| | **本组小计** | **18,654** |
| 32 | src/routes/mod.rs（交叉核证全文读，11 号亦覆盖） | 1249 |

片段级核证（非全文，只为支撑本组断言）：src/routes/shared.rs L1-217 与 L1905-1946（ReviewActor/parse_object_id/validate_account/insert_domain_stage_fields/resolve_authorized_workspace）；src/config.rs 鉴权相关字段与默认值（L21/349-386/418-458/477/778-807/786-789）；src/main.rs L63-67/159-160/188-192/350-369；src/supervisor.rs L252-288（resume_worker_circuit）；src/models.rs L4488-4555/4560-4585/5930-5941（escalation 闭集、ALLOWED_TASK_STATUS）。`routes/knowledge/**` 归 08 号任务，未读。

未覆盖声明：本组对 `agent::escalation::{resolve,reassign}_escalation`、`evolution::release::*`、`prompt_guard`、`config_generation`、`LlmRegistry` 等被调用模块只核证了调用契约（参数/返回/错误映射），其内部实现分属 04/05/09/10 号任务。
