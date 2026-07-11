# 核心业务逻辑全链路深度审查 · 设计

- 日期：2026-07-11
- 背景：上一轮全量系统测试（`2026-07-10-full-system-test-findings.md`，22 findings）以"19 前端频道 × 5 维度"为组织轴，本质是**从 UI 页面往里看**的广度走查 + 代表性抽验。用户判定其**深度不足**：凡是没有对应页面的后端逻辑（webhook→决策→发送核心链路、outbox 幂等/竞态、后台 worker、guards/review 边界、越权、无事务 MongoDB 数据一致性）从"频道"这个轴结构性地扫不到。
- 本轮目标：**从前端每个频道入口出发，穿透到后端逻辑/DB/外部依赖，端到端逐环节验证整条业务链的逻辑正确性。** 频道是入口，审查跟着数据流走到底。

## 用户已确认的四个决策

1. **主轴**：核心的、全面完整的业务逻辑，全链路测试——从前端每个频道入口穿透到后端的完整业务链。
2. **深度**：逐行读码 + 真跑复现（最高确信度）。能复现的才定真 bug。
3. **验证环境**：117 生产环境真跑（受约束，见下）。
4. **分批**：按业务链分批、命脉优先。

## 规模现实（亲验）

- 后端 **188 个 .rs、约 12.7 万行**。最大文件：`models.rs`(6950)、`agent/gateway.rs`(6373)、`planner/mod.rs`(3729)、`agent/memory.rs`(3291)、`agent/review/gates.rs`(2696)。
- 前端频道注册表 `frontend/src/app/channels.ts`：20 个频道，其中 groupOps/momentOps 是占位页（指向 OverviewFeature，无业务逻辑），**18 个有实质业务逻辑**。
- 结论：逐行审全部不现实。必须**多批次**，一批一（组）频道，每批深度到底。硬塞一个大 spec = 重蹈"广而浅"覆辙。

## 分批地图（命脉优先）

- **批 A — 自动回复命脉链**（最先，上轮最大盲区）。入口：command / userOps / autonomy。链路：`webhook → 去抖聚合 → gateway 闸 → 决策 → 知识路由 → 独立 review → revision → outbox 幂等 → MCP 发送 → 回写 events/metrics/run log`。
- **批 B — 知识链**：knowledgeWiki / content / quality（录入→审核→grounding→问答召回→质量校验）。
- **批 C — 成交活动链**：campaign / productsDeals / sendAnalytics（圈人→触达→成交登记→成效聚合）。
- **批 D — 请示配置链**：askHuman / askHumanConfig / llmProviders / systemStrategy（请示裁决→决策人链→provider 热切换→prompt pack）。
- **批 E — 其余**：accountManagement / overview / operations / evolution / referralCards。

## 每批的五步闭环

**① 链路测绘（读码建图）**：从频道前端入口出发，Grep/Read 画出完整调用链（前端组件→API 端点→handler→业务函数→DB 集合读写→外部依赖 MCP/LLM）。产出"环节清单"，每环节标 file:line。**引用必亲验**，不靠记忆/memory 旧描述。

**② 逐环节业务逻辑审查**：对每个环节，对照 `.kiro/specs/*` requirements + `docs/agent-policy.md` 逐条核。找：状态机/阈值/预算边界错误、幂等/竞态缺口、错误处理方向错误（该 fail-close 却 fail-open）、红线闸能否被绕过、无事务下数据一致性中间态、越权。每个疑点带 file:line 证据。

**③ subagent 并行分工**：一条链拆几段派 opus subagent 只读审（指令硬要求先读懂再断言、产出带 file:line 证据、凭猜产出打回）。主控**逐条亲验**再入账，绝不接受未亲验的 subagent 结论。subagent 一律 `model: opus`（CLAUDE.md，注：harness 拒 model:"opus" 参数时省略以继承主会话 opus）。

**④ 117 真跑复现**：能在生产安全复现的疑点，用真实 webhook（带方案 B 签名 HMAC-SHA256）灌入 + 直连 Mongo 核对落库，实证 CONFIRMED。无法在生产安全构造的（精确时序竞态等）标 PLAUSIBLE + 说明为何不能真跑。

**⑤ 入台账**：统一台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`。**只入账、先不修**；审完一批按用户定的优先级进入修复（修复走各自 PR，同前几轮 batch 模式）。

## 117 真跑硬约束（踩过的雷，锁死）

- **真发只对 2 个测试联系人**：吴界 `wxid_ydzaomn4scsb12`、AI应用开发 `wxid_czpvyjvhzizj22`（均账号 102）。吴界现有真人在对话，碰它极其小心。
- **绝不与套件并发**：生产 LLM 端点仅 2 线程，探针必须串行。
- **webhook 灌消息带方案 B 签名**：HMAC-SHA256（账号 webhook_secret），内容 `<ts_ms>.`+raw_body，头 `x-webhook-signature: sha256=<hex>` + `x-webhook-timestamp`；验签 `WEBHOOK_VERIFY_SIGNATURE=true` skew 300s。
- **部署/远程一律用 paramiko 脚本**（`scripts/_remote_run.py`），env: DEPLOY_HOST=117.72.54.28 / PORT=22 / USER=root / PASS / PYTHONUTF8=1 / MSYS_NO_PATHCONV=1。绝不系统 ssh。
- **可恢复**：造的测试数据必清，联系人状态不乱改，收尾核对零残留。
- **登录凭据**：117 本机 .env BOOTSTRAP_ADMIN_USERNAME/PASSWORD，绝不回显值。

## 防假绿铁律（贯穿全程）

- 端点/MCP 失败标 BLOCKED，不算过。
- 真跑拿到真实输出才算数（不接受 skip 假绿）。
- 发现 ≠ 修复：审查阶段只入账不改 src。改任何 src 前确认普适、碰取舍问用户。
- 过拟合红线：绝不为过测试改业务逻辑/prompt/guards/阈值。
- subagent 结论必主控亲验（file:line 证据）后才入账。

## 批 A 具体落地

**入口穿透**：command 总控（管理 Agent 触发运营动作）、userOps（managed 联系人画像/记忆/边界如何喂进决策）、autonomy（可观测指标是否如实反映真实闸行为）。

**核心链路环节（待①测绘亲验，预期覆盖）**：
1. `webhooks.rs` — 解析 / 账号联系人解析 / 签名校验 / managed 门 / 写 inbound。
2. 去抖 pipeline — `register_inbound` deadline 刷新 / generation 抢占 / barge_in / 窗口内聚合。
3. `agent/gateway.rs` — `run_user_operation_gateway_inner`（约 1294 行、无单测的巨型函数，命脉）: managed / cooldown / min-interval / 日上限 / 过期闸。
4. `agent/decision.rs` + `agent/knowledge_router.rs` — 决策 + 渐进式知识路由。
5. `agent/review/gates.rs` — 独立 review + 阈值闸（FactRisk≥6 / PressureRisk≥7 / HumanLike<6 / ProductAccuracy<7）+ 最多一次 revision。
6. `agent/outbox` — 幂等键 / claim / second-pass safety gate / retry。
7. `mcp.rs` — `message_send_text` / result.isError 检查 / 超时。
8. 回写 — events / outcome metrics / decision review / run log / operation_state 从 customer_stage 派生。

**验证重心（真跑优先扎）**：③ gateway 巨型函数闸逻辑 + ⑤ 阈值闸临界值 + ⑥ outbox 幂等竞态。

**批 A 不做**：审查阶段不改任何 src（只入账）；不碰前端（上轮已覆盖）；知识库内部逻辑留批 B。

## 台账 finding 字段（固定结构）

`[编号] 一句话标题 / 入口频道 / 链路环节 / 类型(逻辑正确性|竞态|幂等|错误处理|红线|越权|一致性) / 严重度 / 复现步骤 / 现象 / 根因(file:line 亲验) / 验证状态(CONFIRMED 真跑|PLAUSIBLE 读码) / 修复建议 / 状态(Open)`

## 关联

- 上轮台账 [`2026-07-10-full-system-test-findings.md`]（本轮补其深度盲区，不重复其已覆盖的前端 UI/UX 面）。
- `docs/agent-policy.md`、`.kiro/specs/agent-autonomy-loop/requirements.md`（红线与闸阈值的权威依据）。
- 已判非本轮：多租户隔离（就绪债）、知识库子系统专项（已有历史审查，批 B 复核而非从零）。
