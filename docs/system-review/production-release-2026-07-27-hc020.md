# HC-020 Management 副作用协议部署后验证记录（2026-07-27）

当前正式 release 的 `management.rs`、HC‑020 红线测试、Command Center 页面与 Store 四份源码和本地逐字一致；运行中后端含 `execution_unknown` 保守恢复协议，正式前端静态制品含 `planHash` 绑定及“结果未知，已停止重放”提示。本轮没有切换二进制或修改正式业务库。

服务器以真实 `wa_session` Cookie middleware、生产 `api_router`、随机 MongoDB、生产 LLM 计划入口和仅回环 WireMock MCP 运行 `hc020_management_command_protocol` 2/2。写工具即使由模型标成 low/无需确认仍停在 `pending_confirmation`；错账号和错 plan hash 均返回 409 且联系人零写；正确绑定后只写一条 `staff_confirmed`，actor 为认证管理员 `hc020-admin`。预置 stale `running + executing` 后重试，tool 与 command 均收敛为 `execution_unknown`，联系人零写且 MCP `tools/call=0`。

前端 Command Center/Store 专项 14/14；同一工作树的 TypeScript 和生产构建已通过。测试库 126→126、差集为空；正式服务前后均为 PID `2021387`、`NRestarts=0`，磁盘与运行中二进制 SHA-256 均为 `dabddf043a717c0c172d3bd9722b1f4b4975f544d201ef4f852228b926089096`，健康正常且 Evolution 关闭。

成功证据目录：`/opt/wechatagent/releases/deploy-20260726T175122Z/audit/hc020-management-20260727T160500Z`。`SHA256SUMS.final` 已逐文件验证通过，其 SHA-256 为 `25b0ead03b764c73cdfa4be1b2454a6812b52fe104353d68782f281e500e4c20`。此前两次包装器尝试分别在 rustc 探测和 `/run` noexec 阶段退出，均未进入业务测试或创建随机库。

本证据不等同于真实外部 MCP 目录/远端副作用已执行，也不替代真实管理员浏览器切号或杀进程级崩溃恢复演练；Campaign、知识修复、Provider 等下游业务自身缺陷继续独立结算。
