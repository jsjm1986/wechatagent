# WechatAgent Development Docs

这组文档用于指导 WechatAgent 从当前的私聊 Agent 原型，演进为完整的微信私域 AI 运营平台。

## 阅读顺序

1. [产品模块规划](product-modules.md)
2. [系统架构](architecture.md)
3. [Agent 策略与自动化边界](agent-policy.md)
4. [数据与接口设计原则](data-and-api.md)
5. [开发路线图](development-roadmap.md)
6. [前端设计系统](frontend-design-system.md)

## 当前阶段

当前代码已经具备：

- Rust Axum 后端
- MongoDB 数据层
- MCP Server 调用能力
- DeepSeek/OpenAI-compatible 模型调用
- React 管理后台
- 私聊好友 `normal / managed` 纳管
- managed 好友自动回复、画像、记忆和跟进任务

下一阶段的核心不是继续堆工具，而是按业务模块扩展：

```text
工作台
用户运营
微信群运营
朋友圈运营
内容资产
Agent 策略
任务与日志
账号与系统
```

## 文档维护规则

- 新增一级业务模块前，先更新 `product-modules.md`。
- 新增后端能力前，先更新 `architecture.md` 和 `data-and-api.md`。
- 新增自动化行为前，先更新 `agent-policy.md`。
- 新增前端频道、子标签或布局规则前，先更新 `frontend-design-system.md`。
- 文档要服务开发决策，不写无法落地的愿景描述。

