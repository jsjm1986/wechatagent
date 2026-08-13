# Swarm 业务逻辑审查与修复报告

> 审查基线：`0e15181f4ced064d21d97f11bfe1af77fbe01aff`
> 复核与修复日期：2026-08-05
> 范围：授权有效期、知识审核并发一致性、Admin 裁决输入与前后端能力对齐。
>
> **2026-08-13 核对追记**：本报告三项结论（限时转述与长期豁免语义、知识审核版本绑定、Admin 裁决
> 严格校验）对照当前代码复核仍准确。一处需知悉的后续演进：2026-08-13 优化线 E 落地"预授权底线
> （standing order）"——请示卡超时到链尾时，若命中管理员**预先书面授权**的兜底策略，系统自动以
> `resolved_via="standing_order_policy"` 裁决并复用既有 resolve→relay 链路转述（`resolved_via`
> 由此新增第三个取值，此前为 `admin` / `wechat`；`src/agent/escalation/mod.rs`）。该路径执行的是
> 人类预授权（双字段成对校验，缺一不生效），本报告第三节的 Admin 结构化裁决入口严格校验不受影响。

## 结论摘要

本报告原确认的两个 S2 代码缺陷已修复：

1. 知识审核现已绑定管理员实际审阅的 chunk 版本，并在同一 MongoDB 事务内检查版本与 D2 证据后写入 verify revision。
2. Admin 裁决现使用严格 DTO 与业务校验，不再复用 LLM 输出的容错降级。

原“S1：限时产品豁免永久生效”经交叉复核后确认是两套契约冲突，而不是可直接认定的实现偏差。本次采用兼容较新专项设计的产品口径：

- `authorizationWindowHours` 只控制本次裁决转述的可用期；
- `customer_only` / `knowledge` 产生的客户级豁免长期有效，直至管理员显式撤销；
- Admin UI、REST DTO、Management 工具描述和历史展示均明确区分这两个维度。

此前文档中“Admin 后台路径不可触发、唯一只能由微信 LLM 触发”的结论错误，现已删除。REST、Management 和 Admin UI 均可显式提交 `exemptionType`。

## 已修复：限时转述与长期客户豁免契约冲突

**采用的明确语义**：

- `authorizationWindowHours`：本次 relay 使用裁决内容的有效期，可空；合法范围为 `(0, 8760]` 小时。
- `exemptionType=none`：不授予后续产品豁免。
- `exemptionType=customer_only`：当前客户长期豁免，可由管理员撤销。
- `exemptionType=knowledge`：当前客户长期豁免，并按既有流程沉淀通用知识。

**实现**：

- `PrincipalDecision` 与解释 Prompt 已明确授权窗只约束本次转述，不控制长期豁免。
- Admin UI 增加豁免范围选择，并明确展示长期属性与撤销方式。
- 已裁决历史分别展示“本次转述到期”和“长期豁免”。
- Management 工具描述公开 `exemptionType` 及两种有效期语义。

**边界**：本次没有给联系人豁免新增自动过期字段，因为那会违反已采用的“长期常驻、显式撤销”设计。

## 已修复 S2：知识审核绑定被审核版本

**原问题**：管理员查看版本 A 后，版本 B 可能在审核提交前或事务外 D2 检查后写入；旧接口没有携带管理员看到的版本令牌，因此可能审核未看过的内容。

**修复**：

- 单条 Verify 强制携带 RFC3339 `expectedUpdatedAt`。
- 批量 Verify 改为 `items: [{ id, expectedUpdatedAt }]`，每条独立绑定版本。
- 服务端在同一 MongoDB 事务内：
  1. 读取当前 chunk；
  2. 精确比较 `updated_at`；
  3. 检查 `source_quote` 与 `source_anchors`；
  4. 写入 `chunk_revisions(op=verify, source=human)` 并 CAS 更新 chunk。
- 版本不符返回 `409 chunk_revision_conflict`，且不写 revision。
- Inspector、评审列表、统一收件箱审核卡、Management 单条/批量工具均传递版本令牌。
- Chat Apply 回执增加应用后的 `updatedAt`；Go-Live 在 Apply 后使用新版本核验，避免误用 Apply 前快照。
- 单条详情端点与列表统一使用 camelCase 投影，`updatedAt` 为 RFC3339，deep-link 无需解析 BSON Extended JSON。

## 已修复 S2：Admin 裁决严格校验

Admin API 不再调用 `sanitize_verdict` 容错处理结构化请求，现执行以下规则：

- 未知字段由 `deny_unknown_fields` 拒绝。
- `verdict` 必须属于既定闭集，非法值返回 400，不再静默变成 `deferred`。
- `approved` / `conditional` 必须有非空 `substance`。
- `exemptionType` 必须是 `none|customer_only|knowledge`。
- 只有 `approved` / `conditional` 可携带非 `none` 豁免或授权窗。
- `authorizationWindowHours` 必须有限、为正且不超过 8760 小时；0、负数、无穷和超上限均返回 400。
- constraints 会 trim 并删除空项。
- LLM 自然语言解释路径仍可保留 fail-safe 容错，未与 Admin 结构化接口混用。

Management 执行器会先移除仅用于路由的 `shortCode` / `chunkId`，再反序列化严格 DTO，避免路径参数被 `deny_unknown_fields` 误拒。

## 验证结果

已执行并通过：

- `cargo check --all-targets`
- 前端 `tsc --noEmit`
- Admin 严格裁决单测：4 passed
- 知识旧快照冲突集成测试：1 passed
- 批量核验集成测试：4 passed
- D2 缺 anchor 拒绝测试：1 passed
- Admin resolve 端到端：2 passed
- Knowledge fallback 回归：3 passed
- 最新前端针对性测试：5 files / 29 tests passed
- chunk 详情投影契约快照：1 passed
- `cargo fmt --check` 与 `git diff --check`

其中旧快照回归明确验证：并发更新后使用旧 `expectedUpdatedAt` 会被拒绝，且不会留下 verify revision。

## 未覆盖边界

- 未执行真实 LLM、MCP 或微信环境端到端验证。
- 未执行仓库完整测试矩阵；已执行与本次改动直接相关的 Rust、MongoDB 与前端测试。
- 长期客户豁免仍依赖显式撤销，这是本次选定的产品语义，不是自动过期实现遗漏。

## 后续建议

1. 在真实 Admin 页面走一次 `none/customer_only/knowledge` 三种裁决验收，确认文案与运营预期一致。
2. 在预发布环境验证 Management 工具携带 `exemptionType` 和知识 `expectedUpdatedAt` 的确认流程。
3. 若未来产品希望“长期豁免也受授权窗约束”，应作为新契约变更设计迁移和历史数据策略，而不是复用当前 relay 到期字段。
