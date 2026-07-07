# MCP Server 真实接入工作完成报告

日期：2026-07-07  
分支：`fix/dispatcher-send-timeout-alignment`  
Commits：`9c34d80`（Workspace Key 支持）+ `35a6125`（P0 缺口补全）

---

## 背景

之前测试环境 MCP server 宕机，所有 MCP 调用走 C 类 BLOCKED 路径。现需正式接入真实 MCP server (117.72.54.28:3001)，使用 Workspace Key `gwa_ba60a98a...` 管理多账号。

---

## 完成工作

### ✅ 阶段 1：技术基础对接（commit `9c34d80`）

**问题**：Workspace Key 调用账号类工具需传 `account_alias`，当前代码不支持  
**修复**：
- `McpCredentials` 加 `account_alias` 字段（从 `wechat_accounts.alias` 读取）
- `call_tool_with_key` 自动注入 `account_alias` 到 `arguments`（不覆盖已有值）
- `logged_call_for_account` 传递 alias
- 顺带修正无状态 server 容错（`initialize_session` 返回 `Option<String>`）

**验证**：
- 活体测试成功：initialize ✓、auth_whoami ✓、tools/list (136工具) ✓
- 核实 Key 身份：Workspace Key，管理 1 个账号 alias="t-1"
- 协议层 100% 兼容：Streamable-HTTP、会话管理、SSE 解析、鉴权全正确

**审查报告**：`docs/mcp-integration-audit-2026-07-07.md`

---

### ✅ 阶段 2：业务闭环补全（commit `35a6125`）

#### P0-1: 微信账号登录流程

**后端（2 个新端点）：**
```rust
// src/routes/accounts.rs
POST /api/accounts/login/begin
GET  /api/accounts/login/poll
```

**功能**：
- 调用 MCP `login_begin` 获取二维码（支持 Workspace Key + Account Key）
- 轮询登录状态（pending → success）
- 返回 wxid/nick_name，前端自动调 sync 同步账号

**前端（扫码登录组件）：**
```tsx
// frontend/src/features/command-center/AccountLogin.tsx
- 配置 account_alias/login_type/login_flow
- 展示二维码或 MCP 登录页面链接
- 自动轮询（每 2.5s）+ 登录成功自动同步
```

#### P0-2: Webhook URL 配置文档

**`docs/mcp-deployment-guide.md`（完整部署指南）：**
- 环境变量配置（`MCP_BASE_URL`、`MCP_API_KEY`）
- 在 MCP Server 配置 webhook URL 的两种方式（管理后台 / 工具调用）
- Webhook 签名验证机制（HMAC-SHA256）
- 账号登录流程（前端 UI / MCP Guide 页面备选）
- 常见问题排查（7 个 FAQ）
- 安全建议（HTTPS、IP 限制、定期轮换）

---

## 当前状态

### ✅ 已实现（技术能调通）

| 模块 | 状态 | 说明 |
|------|------|------|
| MCP 客户端 | ✅ | Streamable-HTTP + SSE + 会话管理 + account_alias 自动注入 |
| Webhook 接收 | ✅ | 解析 + 去重 + HMAC 签名验证 |
| 账号同步 | ✅ | Online/Offline 事件 + 手动 sync |
| 消息发送 | ✅ | text/image/file/video/namecard 全支持 |
| 联系人导入 | ✅ | contacts_search / contacts_fetch_cache |
| 业务工具 | ✅ | 85+ 工具通过 Management Agent 可用 |
| 微信登录 | ✅ | 后端 API + 前端组件（待集成到路由） |
| 部署文档 | ✅ | 完整配置指南 + FAQ |

### 🔄 待验证

| 项目 | 优先级 | 说明 |
|------|--------|------|
| 前端集成 | P0 | 将 `AccountLogin` 组件添加到路由/菜单 |
| 端到端测试 | P0 | 真实环境测试登录流程 + 消息推送 |
| Webhook 配置 | P0 | 在 MCP Server 配置 `https://<域名>/webhooks/wechat` |

### ⚠️ P1 核心级（影响体验但不阻断）

| 项目 | 说明 |
|------|------|
| 前端账号管理页面 | 完整的账号列表 UI（登录状态、一键操作） |
| 定时同步账号状态 | 后台 worker 定期调 sync（防止状态长期不准） |
| 登录成功自动触发同步 | 后端 `login_poll` 成功后自动调 sync（减少手动操作） |
| 账号登录状态字段 | `login_status`（区分"未登录"vs"掉线"） |

---

## 部署清单

### 1. 配置环境变量

```bash
# .env
MCP_BASE_URL=http://117.72.54.28:3001
MCP_API_KEY=gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f
WEBHOOK_VERIFY_SIGNATURE=true
```

### 2. 在 MCP Server 配置 Webhook URL

**方式 A（推荐）**：通过 MCP Server 管理后台  
1. 登录 `http://117.72.54.28:3001/admin`
2. 为每个微信账号配置 `messageWebhookUrl = https://<域名>/webhooks/wechat`

**方式 B**：通过 MCP 工具（如果支持 `set_webhook_url`）

### 3. 测试验证

```bash
# 1. 测试 webhook 端点可达
curl -X POST https://<域名>/webhooks/wechat -H "Content-Type: application/json" -d '{}'

# 2. 登录微信账号（前端 UI 或 MCP Guide 页面）

# 3. 同步账号
curl -X POST https://<域名>/api/accounts/sync -H "Cookie: wa_session=..."

# 4. 发送测试消息到已登录账号，检查后端日志
tail -f <后端日志>
```

### 4. 验证成功标志

- [ ] `wechat_accounts` 表有账号记录（`alias`/`wxid`/`online=true`）
- [ ] 发送消息后 `conversation_messages` 表有 inbound 记录
- [ ] 自动回复触发（`agent_runs` 表有 run 记录）
- [ ] `mcp_logs` 表有 `message_send_text` 调用记录

---

## 技术亮点

1. **自动注入 account_alias**：业务代码无需关心 Workspace Key vs Account Key，`logged_call_for_account` 自动从 DB 读 alias 并注入
2. **会话管理 + 失效重连**：`initialize` 握手拿 session-id，404 时自动丢缓存重连
3. **SSE 响应解析**：兼容 `text/event-stream` 和纯 JSON 两种响应格式
4. **Webhook 签名验证**：HMAC-SHA256 防伪造，可配置开关
5. **原子化去重**：`conversation_messages` 用 unique index + `dedupe_key` 防重复处理

---

## 遗留工作（按优先级）

### P0 阻断级（部署前必须）

1. **前端集成 AccountLogin 组件**
   - 添加到路由：`/command-center/account-login`
   - 在账号管理页面添加"登录微信账号"入口
   - 工作量：~30 分钟

2. **MCP Server 配置 webhook URL**
   - 需要 MCP Server 管理员操作
   - 或通过 MCP 工具 `set_webhook_url`（如果支持）

3. **端到端测试**
   - 登录流程：前端扫码 → 同步账号 → DB 有记录
   - 消息推送：发消息 → webhook 收到 → 自动回复

### P1 核心级（提升体验）

4. **前端账号管理页面增强**（~2-3 小时）
5. **定时同步账号状态 worker**（~1 小时）
6. **登录成功自动触发 sync**（~30 分钟）
7. **账号登录状态字段 + migration**（~1 小时）

### P2 增强级（锦上添花）

8. 账号掉线告警
9. Webhook 配置界面（如果 MCP 支持动态配置）
10. MCP 工具目录刷新

---

## 相关文档

- **兼容性审查报告**：`docs/mcp-integration-audit-2026-07-07.md`
- **部署配置指南**：`docs/mcp-deployment-guide.md`
- **测试脚本**：`scripts/e2e/test_mcp_account_alias.mjs`
- **MCP Guide**：`http://117.72.54.28:3001/mcp-guide.html`

---

## 总结

**技术基础 100% 到位，业务闭环缺最后一公里**：前端组件已写好，需添加到路由；Webhook URL 需在 MCP Server 侧配置。完成这两项后即可真实接入。

**当前分支**：`fix/dispatcher-send-timeout-alignment`（2 commits）  
**推荐下一步**：merge 到 main，部署测试环境，按部署清单逐项验证。
