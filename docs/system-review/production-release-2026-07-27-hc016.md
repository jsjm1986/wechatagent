# HC-016 Shadow/Simulation 零业务副作用部署后验证记录（2026-07-27）

## 结论

当前正式后端已经包含 SR-048 的 Shadow/Simulation 零业务副作用实现。本批未再次切换二进制；部署后服务器 `rs0` 随机库全库逐文档快照红线 1/1 通过。

完整 mock‑LLM Reply→Review→ClaimGate 执行后，除 `llm_call_logs` 中 3 条 `run_mode=shadow` 成本日志外，所有业务集合逐文档不变。预置 operator preference 的 `last_used_at` 未刷新，未写 Contact、Memory、Knowledge gap/proposal、Taxonomy candidate、Outbox 或 outbound message。

## 证据

- 测试：`simulation_has_no_business_side_effects`，1 passed / 0 failed。
- 测试源码 SHA-256：`0b696d39e1074a7a0d64746b0b6c6c19d90eb9e716b1a44269650f9ce933e3c4`。
- 服务器证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/hc016-shadow-20260727T120000Z`。
- `SHA256SUMS.final` 已逐文件校验通过，其 SHA-256 为 `5d44a24a20dd77573ce542004dacb417337294e511a5cdd4eee9e83340b37d07`。
- 测试库数量：126→126；本轮无残留随机库。
- 正式 PID `2021387`、`NRestarts=0`，磁盘和运行中后端 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`；健康 `ok=true`，Evolution 关闭。

## 边界

本记录证明当前已部署确定性 Shadow 路径的数据库副作用边界。测试使用 mock LLM，未调用真实模型或真实 MCP 外部发送链路，因此不把这些外部能力记为已验证。
