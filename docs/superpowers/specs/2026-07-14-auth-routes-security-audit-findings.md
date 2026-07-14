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

（主控亲验后填入）

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
