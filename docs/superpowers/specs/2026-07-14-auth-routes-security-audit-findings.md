# auth + routes 安全隔离面深度审查 findings 台账（第二批）

> 接续第一批 agent 旁挂能力审查（20 findings，PR#207 已合）之后的第二批。范围 = auth + routes 安全隔离面。核心命题 = **授权隔离/IDOR 落实**（认证链 middleware 已亲验干净）。**只审不修**——先出台账，再按 P0-P3 分批修（若有 High 优先级高于第一批遗留 5 个 Medium）。
>
> 设计：`docs/superpowers/specs/2026-07-14-auth-routes-security-audit-design.md`
> 计划：`docs/superpowers/plans/2026-07-14-auth-routes-security-audit.md`

## 审查范围（6 簇）

- **簇S 根因层**：`src/auth/`（681 全部）+ `src/routes/shared.rs`（2427 授权/落库 helper）— 先审，结论作基准。
- **簇1 客户数据域**：contacts.rs(1980) + conversations/tasks/reviews/send_ledger/operation_view/contract_snapshot。
- **簇2 配置/凭证域**：management.rs(3004) + llm_providers.rs(699) + accounts/souls/playbooks/prompt_templates/domain_profiles(1236)/domain_schemas/domains。
- **簇3 媒体/运营动作域**：campaigns.rs(1168) + media_assets(604) + referral_cards/ask_human_inbox(789)/principal_escalations/simulations/products/chunk_locks/assets。
- **簇4 admin/指标/观测域**：admin_*.rs(7) + observability(859)/evolution(1255)/evaluations/outcomes_autonomy/outcome_metrics/lessons_learned/guides/guide_profile/behavior_signal_metrics/events。
- **簇5 knowledge 端点层**：`src/routes/knowledge/`（10 文件 11544 行）。

## 方法论

6 个只读审查 subagent（继承 Opus）；簇S 先派等回、结论喂基准给簇1-5；簇1-5 并行审 + 主控逐条亲验 file:line（复核越权链，驳回夸大）。两态 PLAUSIBLE(读码)/CONFIRMED(可构造越权)。元家族=middleware 保证认证不保证授权，授权靠 handler 自觉锁 workspace。

## 严重度校准（防夸大）

- **High**：推荐配置下**确定性可达的跨 workspace/account 越权读写**或认证绕过。
- **Medium**：需多条件叠加/多租户启用才触发/仅信息泄漏无写入。
- **Low**：观测/边缘/输入校验无越权后果/就绪债。
- **⚠️ 单租户默认部署下不可达的隔离缺陷 = 多租户就绪债，不夸大成 High**（memory project_multitenant_isolation_debt 口径）。

## IDOR 检查清单（逐 handler）

①workspace 来自 AuthenticatedAdmin.current_workspace（非请求体/query 可伪造）②DB filter 含 workspace_id ③按 id 取单条锁 workspace ④list 端点不漏 workspace 过滤 ⑤account_id 校验归属当前 workspace。

## Finding 字段模板

```
### [X-NN] 一句话标题
- 入口频道: —
- 所属簇: S|1|2|3|4|5
- 类型: IDOR|认证|授权|输入校验|敏感泄漏|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 越权链: （谁能越权访问谁的什么资源；非越权类填 —）
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

---

## 环节汇总（收尾时填）

- 总 findings 数：（待填）
- 严重度分布：H / M / L（待填）
- 越权类元家族归纳：（待填）
- 后续 P0-P3 修复路线建议：（待填）

---

## 簇S 授权根因层 findings

> 主控亲验结论：**认证链干净 + 根因层无系统性漏锁**。亲验 find_contact_by_id(shared.rs:167-182) filter 恒含 `{_id, workspace_id}` 复合锁；resolve_authorized_workspace(shared.rs:1592-1617) 把 body/query workspaceId 回落 current_workspace 后经 is_workspace_authorized ACL 校验(:1614)不在 ACL→400。三个 helper(find_contact_by_id/validate_account/resolve_authorized_workspace)构成一致的 workspace 强制锁定基座，body workspaceId 无可伪造旁路。**安全锁 workspace 基准姿势（喂后续簇标尺）**：①workspace 唯一可信源=current_workspace 或经 resolve_authorized_workspace 过 ACL 的返回值 ②按 _id 取单条必走 find_contact_by_id 式复合锁 filter 恒含 workspace_id ③涉 account 先 validate_account(current_workspace,account_id) ④写操作 filter 独立复述 workspace_id 不只靠上游读锁定 ⑤单租户默认不可达=就绪债不夸大 High。

### [S-01] pub 落库 helper 写 filter 不自带 workspace_id，靠上游锁定（纵深防御缺口）
- 入口频道: —
- 所属簇: S
- 类型: 就绪债
- 严重度: Low（主控亲验裁定：当前所有调用点都先经 workspace-locked 读取拿到对象再写，上游锁定成立；但 helper 自身 filter 只 `{_id}` 不复述 workspace_id，是纵深防御缺口——未来某调用点若传入未锁定的 _id 会 IDOR）
- 现象/风险: apply_contact_changes(shared.rs:772 一带)/apply_playbook_changes(shared.rs:858) 写 filter 不自带 workspace_id。
- 越权链: 当前不可达（调用点都上游锁定）；未来若有调用点用未锁定 _id → 可越 workspace 改他人对象。
- 根因（亲验 file:line）: shared.rs 写 helper filter 依赖上游传入已锁定 _id，未在写 filter 独立复述 workspace_id。
- 复现设想: 无当前触发路径；靠 code review 发现纵深缺口。
- 验证状态: PLAUSIBLE（当前调用点安全，纵深缺口为推演）
- 修复建议: 写 helper filter 独立复述 workspace_id（`{_id, workspace_id}`），与 find_contact_by_id 一致，去掉"靠上游"隐式契约。
- 状态: Open

### [S-02] `resolve_authorized_workspace` 空 user_id 直接跳过 ACL（sentinel 型 fail-open）
- 入口频道: —
- 所属簇: S
- 类型: 就绪债
- 严重度: Low（主控亲验裁定：注释论证全仓唯一空 user_id 构造点是 management_admin 内部委托、真实请求 user_id 恒非空，故当前不可越权；但"空 user_id⟹跳过 ACL"是 sentinel 型 fail-open，依赖"无其它空 user_id 来源"的隐式不变量）
- 现象/风险: shared.rs:1599 `if admin.user_id.is_empty() { return Ok(resolved) }` 跳过 ACL 校验。
- 越权链: 当前不可达（唯一空 user_id=可信内部 management_admin）；未来若新增其它空 user_id 构造点→绕过 workspace ACL。
- 根因（亲验 file:line）: shared.rs:1599 空 user_id 短路返回 resolved 不过 is_workspace_authorized；:1594-1598 注释说明是可信内部委托。
- 复现设想: 无当前触发路径。
- 验证状态: PLAUSIBLE（当前不可达，隐患为推演）
- 修复建议: 用显式类型/标志区分"可信内部委托"与"真实请求"（如 AuthenticatedAdmin 加 is_internal_delegation 布尔），而非靠空 user_id sentinel。
- 状态: Open

### [S-03] 直接用 admin.current_workspace 的 handler 无每请求 ACL 复核，session/JWT 撤权滞后
- 入口频道: —
- 所属簇: S
- 类型: 就绪债
- 严重度: Low（主控亲验裁定：current_workspace 来自登录时的 session/JWT claims，若登录后管理员的 workspace 授权被撤销，已签发的 session/JWT 在有效期内仍带旧 workspace；单租户默认无害，多租户+动态撤权才有意义）
- 现象/风险: 直接读 admin.current_workspace 的 handler（不经 resolve_authorized_workspace）不做每请求 ACL 复核，session/JWT 有效期内撤权不即时生效。
- 越权链: 多租户+管理员 workspace 授权被撤销后、session/JWT 未过期前，仍可访问被撤 workspace。
- 根因（亲验 file:line）: current_workspace 注入自 middleware.rs:54-56/79（session/JWT claims），有效期内不重查 ACL。
- 复现设想: 多租户环境撤销管理员某 workspace 授权，用其未过期 session 访问该 workspace 数据。
- 验证状态: PLAUSIBLE（单租户默认不可达；多租户动态撤权才触发）
- 修复建议: 敏感 handler 经 resolve_authorized_workspace 做每请求 ACL 复核；或缩短 session/JWT TTL 限制撤权滞后窗口。属多租户就绪债。
- 状态: Open

## 簇1 客户数据域 findings

> 主控亲验结论：**handler 总体守住 workspace 锁定基准**——list/read/单条取用一律经 find_contact_by_id 复合锁或显式 `workspace_id: &admin.current_workspace` filter，涉 account 端点先过 validate_account；**无推荐配置(单租户默认)下确定性可达的跨 workspace/account 越权，无 High**。亲验 enable_agent(contacts.rs:932)contact 取用已锁 workspace，但 :939 account 存在性校验漏 workspace（1-01）。

### [1-01] `enable_agent` account 存在性校验漏 workspace 作用域（跨租户读）
- 入口频道: admin（启用 Agent 运营）
- 所属簇: 1
- 类型: IDOR（跨租户存在性读）
- 严重度: Medium（主控亲验裁定：CONFIRMED 代码偏离——contact 已锁 workspace 但 account 校验只按 account_id 无 workspace_id；多租户+共享 default account_id 时确定性可达命中他 workspace 同名 account 判"存在"通过；封顶 Medium 因**仅存在性判定、无 PII 泄漏、无跨 ws 写**；单租户默认不可达）
- 现象/风险: enable_agent 校验 contact.account_id 是否在 wechat_accounts 注册时，find_one 只按 account_id 不含 workspace_id。
- 越权链: 多租户环境，admin A 对本 workspace contact 启用 Agent，若其 account_id 与他 workspace 的 account 同名（如共享 default account_id），会命中他 workspace 记录判"已注册"通过——跨租户存在性读（不读内容、不写、不泄 PII）。
- 根因（亲验 file:line）: contacts.rs:932 find_contact_by_id 已锁 workspace（正确），但 :936-940 `accounts().find_one(doc!{"account_id": &contact.account_id}, None)` 漏 workspace_id；复合唯一键 indexes.rs:14-15 亲验 + 共享 default account_id 使多租户可达。
- 复现设想: 多租户 + 两 workspace 共享 default account_id，admin A 启用本 workspace contact 观察 account 存在性校验命中他 workspace 记录。
- 验证状态: CONFIRMED（代码偏离 + 多租户可达性亲验；单租户不可达）
- 修复建议: account 存在性校验 filter 加 workspace_id：`find_one(doc!{"account_id": &contact.account_id, "workspace_id": &admin.current_workspace})`。属多租户就绪债。
- 状态: Open

### [1-02] `apply_generated_profile_to_contact` 写 filter 未复述 workspace_id（读门兜住）
- 入口频道: —
- 所属簇: 1
- 类型: 就绪债（防御纵深）
- 严重度: Low（主控亲验裁定：当前上游已 workspace 锁定不可利用；写 filter 未复述 workspace_id 是回归护栏缺失，同 S-01）
- 现象/风险: apply_generated_profile_to_contact 写 filter 未独立复述 workspace_id。
- 越权链: 当前不可达（上游 workspace 锁定）；未来回归风险。
- 根因（亲验 file:line）: contacts.rs apply_generated_profile_to_contact 写 update filter 未复述 workspace_id，靠上游读锁定。
- 复现设想: 无当前触发路径。
- 验证状态: CONFIRMED（当前不可利用）/ PLAUSIBLE（未来回归风险）
- 修复建议: 写 filter 复述 workspace_id。
- 状态: Open

### [1-03] `analyze_contact_profile` 写 filter 未复述 workspace_id（读门兜住）
- 入口频道: —
- 所属簇: 1
- 类型: 就绪债（防御纵深）
- 严重度: Low（主控亲验裁定：同 1-02，当前上游锁定不可利用）
- 现象/风险: analyze_contact_profile 写 filter 未独立复述 workspace_id。
- 越权链: 当前不可达；未来回归风险。
- 根因（亲验 file:line）: contacts.rs analyze_contact_profile 写 update filter 未复述 workspace_id。
- 复现设想: 无当前触发路径。
- 验证状态: CONFIRMED（当前不可利用）/ PLAUSIBLE（未来回归风险）
- 修复建议: 写 filter 复述 workspace_id。附记 subagent out-of-cluster 观察：guides.rs 调用的 shared.rs guide-apply 写 helper 同属 _id-only 写 filter 模式（与 S-01 同族），可随本条一并补齐。
- 状态: Open

## 簇2 配置/凭证域 findings

> 主控亲验结论：**授权隔离扎实、无真越权、凭证泄漏面干净**。亲验 llm_providers.rs:79 api_key 恒 mask_api_key 不回明文、accounts.rs:56 只回 mcpKeyConfigured 布尔从不回 mcp key 明文、management.rs:1891 provider_activate 把 body workspaceId 强制覆盖为可信 workspace_id。4 条 finding 全是"读门已兜住、写 filter 未复述 workspace_id"的防御纵深偏离（同 S-01 元家族），无跨 workspace 泄漏。domain_schemas.rs 是模范（每写 filter 独立复述 workspace_id）。

### [2-01] playbooks.rs 三处写 update_one 仅按 `_id` 未复述 workspace_id（读门兜住）
- 入口频道: —
- 所属簇: 2
- 类型: 就绪债（防御纵深）
- 严重度: Low（主控亲验裁定：update:175/set_default:223/optimize:401 前置 find_one 已归属校验不可越权；写 filter 只 `{_id}` 是纵深缺口同 S-01）
- 现象/风险: playbooks 三处写操作 filter 仅 `{_id}`。
- 越权链: 当前不可达（前置 find_one 锁定）；未来调用点若传未锁定 _id → 越 workspace 改他人 playbook。
- 根因（亲验 file:line）: playbooks.rs:175/223/401 update_one filter 未复述 workspace_id，靠前置 find_one。
- 复现设想: 无当前触发路径。
- 验证状态: PLAUSIBLE
- 修复建议: 写 filter 独立复述 workspace_id。
- 状态: Open

### [2-02] prompt_templates.rs publish 终态 update_one 仅按 `_id`（读门兜住）
- 入口频道: —
- 所属簇: 2
- 类型: 就绪债（防御纵深）
- 严重度: Low（主控亲验裁定：读门兜住，同 2-01）
- 现象/风险: prompt_templates publish 写 filter 仅 `{_id}`。
- 越权链: 当前不可达。
- 根因（亲验 file:line）: prompt_templates.rs:376 update_one filter 未复述 workspace_id。
- 复现设想: 无当前触发路径。
- 验证状态: PLAUSIBLE
- 修复建议: 写 filter 复述 workspace_id。
- 状态: Open

### [2-03] domain_profiles.rs publish/rollout/rollback/activate 的 promote/demote 仅按 `_id`（读门兜住）
- 入口频道: —
- 所属簇: 2
- 类型: 就绪债（防御纵深）
- 严重度: Low（主控亲验裁定：前置 find_one({_id,workspace_id}) 校验，scope 用已校验记录自身 workspace_id，不可越权）
- 现象/风险: domain_profiles 状态流转的 promote/demote 写 filter 仅 `{_id}`。
- 越权链: 当前不可达（前置 find_one 锁定 + scope 用已校验记录 workspace_id）。
- 根因（亲验 file:line）: domain_profiles.rs promote/demote update filter 未复述 workspace_id，靠前置 find_one({_id,workspace_id})。
- 复现设想: 无当前触发路径。
- 验证状态: PLAUSIBLE
- 修复建议: 写 filter 复述 workspace_id。
- 状态: Open

### [2-04] get_tool_catalog 无归属校验即按 query.accountId 调 MCP
- 入口频道: admin（工具目录查询）
- 所属簇: 2
- 类型: 就绪债
- 严重度: Low（主控亲验裁定：仍须登录，只回工具名清单无凭证/无 PII；query.accountId 未校验归属当前 workspace 但仅拉工具列表）
- 现象/风险: management.rs:606 get_tool_catalog 按 query.accountId 调 MCP 拉工具清单，未校验该 account 归属当前 workspace。
- 越权链: 已登录 admin 可用他 workspace 的 accountId 拉该账号 MCP 工具名清单（无凭证/无客户数据，信息价值极低）。
- 根因（亲验 file:line）: management.rs:606 未 validate_account 即用 query.accountId 调 MCP。
- 复现设想: 已登录传他 workspace 的 accountId 观察返回工具清单。
- 验证状态: PLAUSIBLE（信息泄漏面极小）
- 修复建议: 调 MCP 前 validate_account(current_workspace, account_id) 校验归属。
- 状态: Open

## 簇3 媒体/运营动作域 findings

> 主控亲验结论：**运营动作触发面（圈人群发/推名片/影子模拟/换素材）在真正的租户边界 workspace_id 上锁得干净——无跨 workspace 触发他人租户运营动作的真 High IDOR**。所有 send/action-trigger 写端点目标（contacts/campaign/card/asset）恒被 workspace_id 过滤。simulations.rs 是范式标杆（validate_account+find_contact_by_id+归属双核+shadow 不真发）。**主控关键校准：subagent 标 3-01 为 Medium，亲验 create_campaign(campaigns.rs:227-263)后降为 Low**——workspace_id 恒由会话注入(:245)、圈人 resolve_segment_contacts 恒同传 campaign.workspace_id(:283/:353)、脏 account_id 只能圈 0 人(dispatch :359-361 命中0直接 BadRequest)或落本 workspace 死作用域标签，无越权后果（校准口径「输入校验无越权后果=Low」），且 :131-134 注释明写「不额外校验账号归属」是显式设计。

### [3-01] 4 个 action-capable 端点收 account_id 但不过 validate_account（自限不跨租户）
- 入口频道: admin（圈人活动创建/素材上传/名片创建/内容资产创建）
- 所属簇: 3
- 类型: 输入校验（偏离基准 #3：涉 account 未 validate_account）
- 严重度: Low（主控亲验降级裁定：subagent 初判 Medium，但 workspace_id 恒会话注入是真租户边界，脏 account_id 自限——圈人 0 命中即 BadRequest 不发送/素材落本 workspace 死作用域标签，**无跨租户读写、无动作触发**；校准口径「输入校验无越权后果=Low」；且 create_campaign 注释:131-134 显式声明「不额外校验账号归属」为设计选择）
- 现象/风险: create_campaign(campaigns.rs:239-242)/upload_media_asset(media_assets.rs:105-108,140)/create_referral_card(referral_cards.rs:63,74)/create_content_asset(assets.rs:137) 接收 body/multipart account_id 但不调 validate_account(current_workspace, account_id)。
- 越权链: 无跨租户越权——workspace_id 会话注入为真边界，account_id 仅作 scope 标签，脏值自限本 workspace（圈 0 人/死作用域）。
- 根因（亲验 file:line）: campaigns.rs:239-242 account_id 回落 default 或取 body 不校验归属，:245 workspace_id=admin.current_workspace（真锁），:283/:353 resolve_segment_contacts 恒同传 campaign.workspace_id，:359-361 命中 0 人 BadRequest；referral_cards.rs:63/:73 + media_assets.rs:140/:160 account_id 仅当 scope 标签、读写 filter 恒 `{_id, workspace_id}`（referral :98/:139/:197/:257；media :214/:286/:341/:460/:507/:538）亲验。
- 复现设想: 传外域 account_id 建 campaign → dispatch 圈本 workspace managed 联系人 0 命中 → BadRequest 不发送（无跨租户效果）。
- 验证状态: CONFIRMED（代码偏离 + 自限性亲验：脏 account_id 不可跨租户）
- 修复建议: 4 端点补 validate_account(current_workspace, account_id) 做输入健壮性（廉价保险，非安全阻断）；或显式在注释确认 account_id 纯 scope 标签设计。低优先。
- 状态: Open

### [3-02] ask-human 收件箱 taxonomy 全局候选池无 workspace 过滤（同 4-01 根因）
- 入口频道: admin（ask-human 审核收件箱）
- 所属簇: 3
- 类型: 敏感泄漏（跨租户 evidence 可见性）
- 严重度: Low（主控亲验裁定：TaxonomyCandidate 模型无 workspace_id、隔离靠 scope，account 私有候选 scope=account_id 正确未暴露仅暴露 global；多租户下 global 候选 evidence 跨租户可见为就绪债；单租户默认不可达；与 [4-01] 同根因 taxonomy 一族无 workspace 字段）
- 现象/风险: collect_taxonomy_candidates(ask_human_inbox.rs:233-234)/ask_human_summary(:545) 查 `{scope:"global", status:"pending"}` 无 workspace_id 过滤。
- 越权链: 多租户下租户 A 一次 AI run 生成的 global 候选（evidence 含 A 客户对话片段）出现在租户 B 审核收件箱；任一租户 approve 即全体命名共享字典项。单租户无害。
- 根因（亲验 file:line）: ask_human_inbox.rs:233-234/:545 查 scope=global 无 workspace_id；TaxonomyCandidate(models.rs:3105-3128)无 workspace_id 字段亲验，隔离键仅 scope。
- 复现设想: 多租户下租户 B admin 打开 ask-human 收件箱观察含租户 A 客户对话片段的 global 候选。
- 验证状态: PLAUSIBLE（模型无 workspace_id 亲验；跨租户 evidence 泄漏为多租户推演）
- 修复建议: 见 [4-01] 统一——taxonomy 一族是否加 workspace/scope 门属多租户+RBAC 产品裁决。本条与 4-01 归并到簇4 taxonomy 元家族留痕。
- 状态: Open

### [3-03] chunk 协作软锁未做 workspace 归属校验（软锁不 gate 写入）
- 入口频道: admin（协作编辑软锁 WebSocket）
- 所属簇: 3
- 类型: 授权（协作软锁）
- 严重度: Low（主控亲验裁定：锁表 DashMap 仅以 chunk_id 为键、只比 owner_user_id 不校验 chunk 归属 workspace；但 grep 确认**除 chunk_locks.rs 自身外无任何 chunk 编辑 handler 在写前检查此锁**——软锁纯 UI 协作提示不 gate apply_chunk_revision，绕过零数据完整性影响、无越权写；最坏多租户+已知外域 ObjectId=编辑骚扰+小幅锁元数据泄漏；单租户不可达）
- 现象/风险: chunk_locks.rs:91 锁表 `DashMap<chunk_id, ChunkEditLock>` 只 chunk_id 为键；acquire(:103-163)/release(:168-198) 只比 owner_user_id，不校验 chunk_id 归属 current_workspace。
- 越权链: 多租户+已知外域 chunk ObjectId → 跨租户占锁（对方看到 locked_by_other）+ 409/403 响应回吐锁的 workspace_id/owner_username 小幅元数据；软锁不 gate 写入故无越权写；WS 事件流(:234)按 workspace 过滤受害租户收不到骚扰事件。
- 根因（亲验 file:line）: chunk_locks.rs:91 锁键仅 chunk_id；:103-198 acquire/release 只比 owner_user_id 无 workspace 校验；:89-90 注释辩称 chunk_id 全局唯一 ObjectId 故键安全。
- 复现设想: 多租户下用已知外域 chunk ObjectId 调 acquire 观察占锁 + 响应体锁元数据。
- 验证状态: PLAUSIBLE（锁键/所有权亲验；软锁不 gate 已 grep 确认）
- 修复建议: acquire 前校验 chunk_id 归属 current_workspace（find_one `{_id, workspace_id}` 存在性）再占锁；或维持现状（软锁本就不承担安全边界）。低优先。
- 状态: Open

## 簇4 admin/指标/观测域 findings

（主控亲验后填入）

## 簇5 knowledge 端点层 findings

（主控亲验后填入）
