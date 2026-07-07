# MCP Server 部署配置指南

本文档说明如何配置 MCP Server 与 WechatAgent 项目的完整对接。

## 前提条件

- WechatAgent 项目已部署并可访问（假设域名为 `https://your-domain.com`）
- 已获取 MCP Server API Key（格式如 `gwa_xxx...` 的 Workspace Key 或 Account Key）
- 可访问 MCP Server 管理后台：`http://117.72.54.28:3001/admin`（或实际管理地址）

---

## 1. 配置环境变量

在 WechatAgent 项目的 `.env` 文件中配置 MCP 连接信息：

```bash
# MCP Server 地址
MCP_BASE_URL=http://117.72.54.28:3001

# MCP API Key（用于调用 MCP 工具和验证 Webhook 签名）
MCP_API_KEY=gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f

# Webhook 签名验证（可选，默认为 true）
WEBHOOK_VERIFY_SIGNATURE=true
```

**重要**：`MCP_API_KEY` 同时用于：
1. 调用 MCP 工具时的鉴权（`Authorization: Bearer` 头）
2. 验证 MCP Server 推送消息的 HMAC-SHA256 签名

---

## 2. 在 MCP Server 配置 Webhook URL

### 方式 A：通过 MCP Server 管理后台配置（推荐）

1. 登录 MCP Server 管理后台：`http://117.72.54.28:3001/admin`
2. 进入"账号配置"或"Webhook 设置"页面
3. 为每个微信账号配置 `messageWebhookUrl`：
   ```
   https://your-domain.com/webhooks/wechat
   ```
4. 保存配置

**注意**：
- 每个微信账号可以配置独立的 webhook URL
- 如果使用同一个 WechatAgent 实例管理多个账号，所有账号的 webhook URL 应指向同一地址
- MCP Server 会在推送消息时在 HTTP 头中添加 `X-MCP-Signature` 签名（使用 `MCP_API_KEY` 作为密钥）

### 方式 B：通过 MCP 工具配置（如果 MCP Server 支持）

某些 MCP Server 可能提供工具（如 `set_webhook_url`）动态配置 webhook：

```bash
curl -X POST http://117.72.54.28:3001/mcp \
  -H "Authorization: Bearer gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "tools/call",
    "params": {
      "name": "set_webhook_url",
      "arguments": {
        "account_alias": "t-1",
        "webhook_url": "https://your-domain.com/webhooks/wechat"
      }
    }
  }'
```

**检查是否支持**：调用 `tools/list` 查看是否有 `set_webhook_url` 工具。

---

## 3. 验证 Webhook 配置

### 3.1 检查 WechatAgent Webhook 端点

确认 WechatAgent 的 webhook 端点可访问：

```bash
curl -X POST https://your-domain.com/webhooks/wechat \
  -H "Content-Type: application/json" \
  -d '{}'
```

**预期响应**：
- 如果签名验证开启：`401 Unauthorized` 或 `403 Forbidden`（因为没有签名）
- 如果签名验证关闭（`WEBHOOK_VERIFY_SIGNATURE=false`）：`400 Bad Request`（因为消息体不合法，但说明端点可达）

### 3.2 触发测试消息

从微信发送一条测试消息到已登录的账号，检查 WechatAgent 后端日志：

```bash
# 查看后端日志（如果用 cargo run）
tail -f <后端日志文件>

# 或查看 Docker 日志
docker logs -f wechatagent
```

**预期日志**：
```
INFO wechat_webhook: received webhook event, appId=xxx, typeName=Text, fromWxid=xxx
INFO wechat_webhook: enqueued managed message for contact xxx
```

### 3.3 检查 MongoDB

查看 `conversation_messages` 集合是否有新消息记录：

```javascript
db.conversation_messages.find().sort({ created_at: -1 }).limit(1)
```

**预期字段**：
- `account_id`: 对应接收消息的账号
- `wxid`: 发送消息的联系人
- `content`: 消息内容
- `direction`: `"inbound"`

---

## 4. Webhook 签名验证机制

MCP Server 推送消息时会在 HTTP 头中添加 `X-MCP-Signature`，格式为：

```
X-MCP-Signature: <HMAC-SHA256(body, MCP_API_KEY) 的十六进制>
```

WechatAgent 验证逻辑（`src/webhooks.rs:295-308`）：

```rust
let provided = headers
    .get("x-mcp-signature")
    .ok_or_else(|| AppError::Unauthorized("missing signature".to_string()))?;

if !verify_hmac_sha256(
    state.config.mcp_api_key.as_bytes(),
    &body,
    provided.to_str().unwrap_or(""),
) {
    return Err(AppError::Unauthorized("invalid signature".to_string()));
}
```

**关闭签名验证**（仅开发环境）：
```bash
# .env
WEBHOOK_VERIFY_SIGNATURE=false
```

---

## 5. 账号登录流程

### 5.1 通过前端 UI 登录（推荐）

1. 访问 WechatAgent 前端：`https://your-domain.com`
2. 进入"账号管理"或"Command Center"
3. 点击"添加账号"或"登录微信账号"
4. 填写配置：
   - **Account Alias**：如果使用 Workspace Key，必须填写（如 `kefu-a`）；Account Key 可留空
   - **登录平台**：选择 `Mac` 或 `iPad`
   - **登录流程**：选择 `Auto`（推荐）
5. 点击"开始登录"，扫描二维码
6. 登录成功后，账号会自动同步到 `wechat_accounts` 表

### 5.2 通过 MCP Guide 页面登录（备选）

如果前端 UI 未完成，可使用 MCP Server 提供的在线登录页面：

1. 访问：`http://117.72.54.28:3001/mcp-guide.html`
2. 填写 API Key
3. 在"在线测试扫码登录"部分填写 `account_alias`（Workspace Key 必填）
4. 点击"开始登录"，扫描二维码
5. 登录成功后，手动调用 WechatAgent 的同步接口：
   ```bash
   curl -X POST https://your-domain.com/api/accounts/sync \
     -H "Cookie: wa_session=<登录后的 session cookie>"
   ```

### 5.3 验证账号同步

查看 `wechat_accounts` 表：

```javascript
db.wechat_accounts.find()
```

**预期字段**：
- `alias`: MCP Server 分配的账号别名（如 `t-1`）
- `wxid`: 微信 ID
- `nick_name`: 微信昵称
- `online`: `true` 表示在线
- `mcp_api_key`: 账号使用的 MCP API Key
- `mcp_base_url`: MCP Server 地址

---

## 6. 常见问题

### Q1: 消息推送不到 WechatAgent

**排查步骤**：
1. 检查 MCP Server 配置的 webhook URL 是否正确
2. 检查 WechatAgent 的 webhook 端点是否可从 MCP Server 访问（防火墙/网络问题）
3. 检查 `.env` 的 `MCP_API_KEY` 是否与 MCP Server 一致
4. 查看 WechatAgent 后端日志是否有签名验证错误
5. 临时关闭签名验证（`WEBHOOK_VERIFY_SIGNATURE=false`）测试

### Q2: 登录后账号不显示

**排查步骤**：
1. 手动调用 `POST /api/accounts/sync` 同步账号
2. 检查 MongoDB `wechat_accounts` 表是否有记录
3. 检查前端是否正确调用 `GET /api/accounts`
4. 查看浏览器控制台是否有 API 错误

### Q3: 调用 MCP 工具报 "Missing account_alias"

**原因**：使用 Workspace Key 调用账号类工具时必须传 `account_alias`

**解决**：
- WechatAgent 已自动注入 `account_alias`（从 `wechat_accounts.alias` 读取）
- 确保 `wechat_accounts.alias` 字段与 MCP Server 的 `account_alias` 一致
- 手动同步账号：`POST /api/accounts/sync`

### Q4: Webhook 签名验证失败

**原因**：MCP Server 使用的签名密钥与 WechatAgent `.env` 的 `MCP_API_KEY` 不一致

**解决**：
1. 检查 `.env` 的 `MCP_API_KEY` 与 MCP Server 配置是否完全一致（包括前缀 `gwa_`）
2. 重启 WechatAgent 后端使新的 `.env` 生效
3. 如果仍失败，临时关闭签名验证排查：`WEBHOOK_VERIFY_SIGNATURE=false`

---

## 7. 安全建议

1. **生产环境必须开启签名验证**：`WEBHOOK_VERIFY_SIGNATURE=true`
2. **MCP_API_KEY 妥善保管**：不要提交到 Git，使用环境变量或密钥管理系统
3. **Webhook 端点使用 HTTPS**：避免消息在传输过程中被窃听
4. **限制 MCP Server IP**：在防火墙或 Nginx 配置中限制只有 MCP Server IP 可访问 `/webhooks/wechat`
5. **定期轮换 API Key**：建议每 3-6 个月更换一次 MCP API Key

---

## 8. 参考资料

- MCP Guide：`http://117.72.54.28:3001/mcp-guide.html`
- MCP Server 管理后台：`http://117.72.54.28:3001/admin`（实际地址可能不同）
- WechatAgent Webhook 源码：`src/webhooks.rs`
- MCP 客户端源码：`src/mcp.rs`
- 兼容性审查报告：`docs/mcp-integration-audit-2026-07-07.md`
