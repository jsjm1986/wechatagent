# System Architecture

## Current Architecture

```text
React Admin
  -> Rust Axum API
    -> MongoDB
    -> MCP Server
    -> DeepSeek/OpenAI-compatible API
```

当前系统是一个 Rust 单体服务：

- 托管 React 静态文件
- 暴露后台 API
- 接收微信 webhook
- 调用 MCP 工具
- 调用 LLM
- 执行任务 worker
- 写入 MongoDB

## Layering

系统应保持以下分层：

```text
Product Modules
  用户运营 / 群运营 / 朋友圈 / 内容资产 / 策略 / 日志

Agent Layer
  意图判断 / 回复生成 / 画像更新 / 任务生成 / 策略执行

Application Services
  联系人服务 / 群服务 / 朋友圈服务 / 内容资产服务 / 任务服务

Infrastructure
  MCP Client / LLM Client / MongoDB / Webhook / Worker
```

原则：

- Product Module 不直接裸调 MCP。
- Agent 不直接关心 HTTP 和数据库细节。
- MCP Client 只负责协议和错误包装。
- 自动化边界由 Agent 策略决定，不散落在业务代码里。

## Current Backend Modules

```text
src/main.rs       启动、路由、静态文件、worker
src/config.rs     环境变量配置
src/db.rs         MongoDB 连接和索引
src/models.rs     数据结构
src/mcp.rs        MCP JSON-RPC 客户端
src/llm.rs        OpenAI-compatible LLM 客户端
src/agent.rs      私聊 Agent 决策和执行
src/routes.rs     后台 API
src/webhooks.rs   微信消息 webhook
src/tasks.rs      跟进任务 worker
```

## Recommended Evolution

随着模块扩展，后端应逐步拆出 service 层：

```text
src/services/contact_service.rs
src/services/group_service.rs
src/services/moment_service.rs
src/services/content_asset_service.rs
src/services/agent_policy_service.rs
src/services/task_service.rs
```

不要为了抽象而抽象。只有当业务逻辑开始跨路由、worker、webhook 复用时再拆。

## Webhook Flow

当前私聊流程：

```text
POST /webhooks/wechat
→ 解析 appId/fromWxid/content/messageId
→ 定位微信账号和联系人
→ 保存 inbound message
→ 如果 contact.agent_status != managed，停止
→ 构建 Agent 上下文
→ 调用 LLM 生成决策
→ 调用 MCP message_send_text
→ 保存 outbound message
→ 更新画像/记忆/任务
→ 写入事件日志
```

后续群聊 webhook 应使用独立流程，不复用私聊自动回复逻辑。

## Worker Flow

当前任务 worker：

```text
定时扫描 pending task
→ 到期任务置为 running
→ 调用 MCP 发送
→ 成功置 sent
→ 失败置 failed
→ 写入事件日志
```

后续应补充：

- 重试次数
- 下次重试时间
- 失败分类
- 任务来源模块
- 幂等键

## Deployment Shape

第一阶段保持简单：

```text
one Rust process
one MongoDB
external MCP Server
external DeepSeek API
```

当任务量或 webhook 量上升后，再考虑：

- API 和 worker 进程拆分
- 队列系统
- 多实例部署
- webhook 签名校验
- 日志/指标采集

