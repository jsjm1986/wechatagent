# KD-04 修复设计：领导微信回复识别改用 decider_chain

- 日期：2026-07-12
- 分支：`fix/kd04-principal-reply-decider-chain`（基于最新 origin/main d832562，已核对基点非滞后）
- 来源：深度审查批 D 唯一 High（台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KD-04）
- 修复方案：**方案 A**（用户裁定）——lookup_principal_config 改用 resolve_ask_human_policy 判成员

## 问题（KD-04 根因，已主控亲验，最新 main 仍成立）

领导用微信回复裁决时，`webhooks.rs:443` 靠 `lookup_principal_config(ledger.rs:215)` 的 `.is_some()` 决定是否把入站消息分流到请示通道（`handle_principal_reply`）。但 `lookup_principal_config` 直接 Mongo 查 `doc!{"principal_decider": from_wxid, "current_version": true}`——**只认旧标量字段 principal_decider**，完全不看 `ask_human_policy.decider_chain`。

而唯一策略写路径 `put_ask_human_policy(domains.rs:233)` 只 `$set ask_human_policy`、**从不写 principal_decider**；默认种子 principal_decider=None。

**后果**：用推荐配置（只配 decider_chain）时 principal_decider 恒 None → lookup_principal_config 恒返 None → handle_principal_reply 永不被调 → 领导的微信裁决消息掉进普通客户入站链路（领导若恰是 managed contact，甚至被 AI 当客户自动回复）。**推荐配置下确定性发生、核心交互失效**。

## 关键约束（已亲验，决定修复低风险且可安全落地）

1. **lookup_principal_config 全仓只有一个调用者**：`webhooks.rs:443`，且**只用 `.is_some()`**——返回的 `domain: String` 被丢弃。可安全改内部逻辑与返回值语义（只要 Some/None 正确反映"from_wxid 是否决策人"）。
2. **resolve_ask_human_policy(policy.rs:21) 已是权威解析器**：`ask_human_policy` 存在时取其 decider_chain；None 时回落 `[principal_decider]`（policy.rs:36-40 字节等价）。lookup_principal_config 却没用它——这正是缺陷。
3. **scan_escalation_timeouts(mod.rs:365) 已有加载范式**：`.find(doc!{"current_version": true})` → 逐个跑 resolve_ask_human_policy。修复镜像此范式。
4. **handle_principal_reply(mod.rs:286) 内部靠 pending.principal_wxid 匹配**：`list_pending_for_principal(from_wxid)` 取挂在该 wxid 名下的 pending；reassign 会把 principal_wxid 更新成 next。故只要 gate（本 lookup）放行，链中任一决策人 / 改派后的 next 决策人回复都能被正确处理。

## 设计（方案 A）

### 组件 1：纯谓词 `is_decider_for_config`（新增，无 IO，可单测）

```rust
/// 纯谓词：from_wxid 是否是该 config 解析后 decider_chain 的成员。
/// resolve_ask_human_policy 已内含旧 principal_decider 回落，故新旧配置都覆盖。
pub(crate) fn is_decider_for_config(cfg: &OperationDomainConfig, from_wxid: &str) -> bool {
    resolve_ask_human_policy(cfg)
        .decider_chain
        .iter()
        .any(|d| d.wxid == from_wxid)
}
```
- 放置：`src/agent/escalation/policy.rs`（与 resolve_ask_human_policy 同文件，同 `pub(crate)`）。
- 职责单一：判定 wxid 是否为某 config 的决策人。依赖：resolve_ask_human_policy + OperationDomainConfig + DeciderRef。

### 组件 2：lookup_principal_config 重写（改 `src/agent/escalation/ledger.rs:215`）

```rust
pub(crate) async fn lookup_principal_config(
    state: &AppState,
    workspace_id: &str,
    from_wxid: &str,
) -> AppResult<Option<String>> {
    use futures::TryStreamExt;
    let mut cursor = state
        .db
        .operation_domain_configs()
        .find(
            doc! { "workspace_id": workspace_id, "current_version": true },
            None,
        )
        .await?;
    while let Some(cfg) = cursor.try_next().await? {
        if crate::agent::escalation::policy::is_decider_for_config(&cfg, from_wxid) {
            return Ok(Some(cfg.domain));
        }
    }
    Ok(None)
}
```
- 从 `find_one` 改 `find` + 遍历，命中第一个含该 wxid 的 config 即短路返 Some(domain)。
- 复用权威解析器 → 自动兼容旧 principal_decider（无额外兼容代码）+ 覆盖链中全部决策人 + 改派 next。
- 返回类型 `Option<String>` 不变，单一调用者语义（Some/None）不变。

### 可见性接线（写实现时先亲验，不猜）

- 写实现前 grep 确认 `is_decider_for_config` 从 ledger.rs 是否可见（policy 模块 pub(crate) 项在 escalation 模块内互访）。若不可见，在 `escalation/mod.rs` 补 `pub(crate) use policy::is_decider_for_config;` 或用全路径 `crate::agent::escalation::policy::is_decider_for_config`（设计里已用全路径）。
- resolve_ask_human_policy 现为 `pub(crate)`（policy.rs:21），is_decider_for_config 同级即可被 ledger.rs 全路径调用。

## 边界与错误处理（与现实现 fail 语义一致）

- **DB 查询失败**：`find().await?` + `try_next().await?` 用 `?` 上抛，与现 `find_one().await?` 语义一致；调用点 webhooks.rs:443 `.await?` 整体返错（webhook 层有重试/幂等）。**正确 fail 方向**：DB 抖动时宁可这条不处理，也不误把领导当客户/误把客户当领导。
- **无匹配 / 空 decider_chain / 未启用请示通道**：遍历完无命中 → `Ok(None)` → webhooks 走正常客户链路。语义正确（该 wxid 不是决策人）。
- **短路**：命中即返（同一 wxid 在多 domain 都是决策人时返哪个 domain 无所谓，值被丢弃）。

## 不改动的（修复严格限定范围）

- `webhooks.rs:443` 调用点、`handle_principal_reply`、`resolve_ask_human_policy`、`put_ask_human_policy`、principal_decider 字段本身——全部不动。
- 不做方案 B/C 的"同步写 principal_decider"（YAGNI：principal_decider 修复后仅剩 resolve 内部回落项，无其他读点，同步写是无用冗余，且加深新旧字段漂移——正是 KD-04 元家族根因）。

## 测试策略（确定性 lib 单测，进 baseline，无需 Docker）

抽纯谓词 is_decider_for_config 使逻辑核心可确定性单测（DB 加载部分是薄壳）。5 个测试放 `src/agent/escalation/policy.rs` 内 `#[cfg(test)] mod tests`（与被测纯函数同文件，沿用 policy.rs 现有单测惯例），`cargo test --lib` 可跑（只增不减，baseline lib≥350 只涨）：

1. **KD-04 复现+修复**：config 只设 `ask_human_policy.decider_chain=[{wxid:"leader1"}]`、principal_decider=None → `is_decider_for_config(&cfg,"leader1")==true`（缺陷版旧逻辑只认 principal_decider 会 false）。
2. **链中非首位决策人**（覆盖改派 next）：decider_chain=[leader1,leader2] → `is_decider_for_config(&cfg,"leader2")==true`。
3. **旧配置兼容回落**：只设 principal_decider="oldboss"、ask_human_policy=None → `is_decider_for_config(&cfg,"oldboss")==true`（resolve 回落）。
4. **非决策人**：decider_chain=[leader1] → `is_decider_for_config(&cfg,"stranger")==false`。
5. **空链/未启用**：ask_human_policy=None + principal_decider=None → 任何 wxid `==false`。

**为何抽纯函数而非集成测试**：本地磁盘紧、集成测试需 Docker/testcontainers（CLAUDE.md 本地只跑 --lib + PBT）。纯函数单测确定性、进 baseline、CI 每次跑——比生产/Docker 复现更严谨可控（符合审查阶段"复现留修复阶段写确定性 lib 单测"定调）。DB 加载壳（find+遍历）逻辑已被纯函数覆盖。

## 验证

- `cargo check` + `cargo test --lib`（新增 5 测试全绿，baseline≥350 不回退）。
- 不改前端、不改 API 契约、不改配置结构——无迁移、无 .env 变更。

## 交付

- 单一 src 文件逻辑改动：policy.rs（+纯谓词+5 测试）、ledger.rs（重写 lookup_principal_config）。
- 独立修复 PR（基于最新 main），与审查台账 PR#178（docs）解耦。
