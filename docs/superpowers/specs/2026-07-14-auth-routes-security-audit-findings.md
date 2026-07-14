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

（主控亲验后填入）

## 簇2 配置/凭证域 findings

（主控亲验后填入）

## 簇3 媒体/运营动作域 findings

（主控亲验后填入）

## 簇4 admin/指标/观测域 findings

（主控亲验后填入）

## 簇5 knowledge 端点层 findings

（主控亲验后填入）
