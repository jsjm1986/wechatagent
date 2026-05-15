# WechatAgent

一个基于 Rust + MongoDB + MCP Server 的微信私聊运营 AI Agent。

第一版只做私聊好友运营：所有好友默认是普通好友，只有人工加入 `managed` 后才由 Agent 自动回复、更新画像、创建跟进任务。

## 架构

```text
React Admin
  -> Rust Axum API
    -> MongoDB: 好友、消息、画像、任务、日志
    -> MCP Server: 微信账号、联系人、发消息
    -> DeepSeek/OpenAI Compatible API: 画像生成和运营决策
```

核心规则：

- 只运营 `agent_status = managed` 的好友。
- 普通好友消息只入库，不自动回复。
- Agent 使用自然语言运营备注 + 历史消息 + 长期记忆生成回复。
- MCP 发文本消息必须使用 `recipient` 和 `content`。

## 配置

复制 `.env.example` 为 `.env`，填入真实配置：

```powershell
Copy-Item .env.example .env
```

必填：

```text
MONGODB_URI
MCP_BASE_URL
MCP_API_KEY
OPENAI_BASE_URL=https://api.deepseek.com
OPENAI_API_KEY
OPENAI_MODEL=deepseek-v4-flash
```

如果当前 MCP Key 是 Account Key，可以保持默认不传 `account_alias`。服务端会绑定到该 Key 对应的微信账号。

## 本地开发

启动 MongoDB 后：

```powershell
cargo run
```

前端开发模式：

```powershell
cd frontend
npm install
npm run dev
```

前端开发服务会代理 `/api` 到 `http://localhost:8080`。

生产构建：

```powershell
cd frontend
npm run build
cd ..
cargo run
```

Rust 服务会托管 `frontend/dist`。

## 前端设计规范

后台 UI 按企业级运营工作台设计语言扩展，新增页面和组件前先阅读：

```text
docs/frontend-design-system.md
```

## 开发文档

完整开发文档从这里开始：

```text
docs/README.md
```

## 后台流程

1. 打开后台。
2. 点击“同步账号”，从 MCP 读取账号状态。
3. 搜索并导入好友，例如 `AI应用开发`。
4. 选择好友，填写自然语言运营备注。
5. 点击“加入 Agent 运营”。
6. 该好友后续 webhook 消息会触发 Agent 自动回复。

## Webhook

配置微信消息回调到：

```text
POST {APP_BASE_URL}/webhooks/wechat
```

Webhook 会尽量从载荷中解析：

```text
appId / app_id
fromWxid / fromUserName
content / msgContent
newMsgId / msgId
```

解析到的消息会写入 `conversation_messages`。只有已纳管好友会触发 Agent。

## 已实现接口

```text
GET    /api/health
GET    /api/accounts
POST   /api/accounts/sync

GET    /api/contacts
POST   /api/contacts/search-import
GET    /api/contacts/:id
POST   /api/contacts/:id/enable-agent
POST   /api/contacts/:id/disable-agent
PUT    /api/contacts/:id/profile-note

GET    /api/conversations/:contact_id/messages
GET    /api/events
GET    /api/tasks

POST   /webhooks/wechat
```

## 验证

已验证：

```powershell
cargo check
cd frontend
npm run build
```
