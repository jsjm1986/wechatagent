# WechatAgent

> Documentation snapshot: checked against commit `d60d3d85f8e193160dca8df185de0daef004a6b6` plus the uncommitted SR-001--SR-183 closure worktree on 2026-07-24. This is not deployment verification; runtime code and `.env.example` remain authoritative when this summary drifts.

一个基于 Rust + MongoDB + MCP Server 的微信私聊运营 AI Agent。

第一版只做私聊好友运营：所有好友默认是普通好友，只有人工加入 `managed` 后才由 Agent 自动回复、更新画像、创建跟进任务。

## 架构

```text
React Admin
  -> Rust Axum API
    -> MongoDB: 好友、消息、画像、任务、日志
    -> LLM Provider Registry: 画像、知识路由、回复与独立 Review
    -> Agent Gateway -> durable Outbox -> second safety gate -> MCP Server
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

启动必填（其余项有默认值，可按部署覆盖）：

```text
MCP_API_KEY
OPENAI_API_KEY
```

`MONGODB_URI`、`MCP_BASE_URL`、`OPENAI_BASE_URL` 和 `OPENAI_MODEL` 均有代码默认值；生产部署仍应显式配置，避免误连开发默认端点。

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

## 演化器（Evolution worker / M4）

可选的后台演化器：每 6 小时（`EVOLUTION_TICK_SECONDS` 默认 21600）按 cohort 选样，生成阈值与运营 prompt 候选，进入 shadow replay + 显著性检验；当前产品政策要求 admin 在前端手工 release / rollback。

- 双闸：`EVOLUTION_ENABLED=false` 是默认关闭的运维硬锁；只有显式设为 `true` 后，workspace 的 Mongo runtime flag 才能进一步放行灰度。
- 自动发布：即使旧配置字段被打开，代码政策常量 `CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 仍强制当前全部人工发布。
- 已发布的 `threshold_overrides` / `prompt_templates` 在主开关关停后**不回退**，由 admin 手工 rollback。
- 演化器走独立模块 `src/evolution/`，CI lint（`scripts/check-evolution-isolation.{sh,ps1}`）守住"不调用 gateway / outbox / MCP"红线，确保演化器对生产链路零副作用。
- 完整设计与安全边界见 `docs/agent-policy.md` 自我演化章节。

`.env.example` 已包含 14 条 `EVOLUTION_*` 配置，复制 `.env` 时一并落入即可。

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

CI 合并门：执行 `scripts/check-baseline.ps1`（Windows）或 `scripts/check-baseline.sh`（Linux / CI），核验 `cargo test --lib >= 350`、`cargo check --tests` 在 `-D warnings` 下通过，以及 4 个 PBT 文件累计 `>= 33`；任一不达标即 `exit 1`。

文本严禁词 lint：`scripts/check-no-human-takeover.{sh,ps1}` 扫 `src/agent/ src/routes/ src/evolution/ frontend/src/` 新增行禁用 `human / 人工 / 接管 / takeover / hand-off`；演化器隔离 lint：`scripts/check-evolution-isolation.{sh,ps1}` 扫 `src/evolution/` 是否引用 gateway / outbox / MCP（M4 演化器必须保持独立）。
