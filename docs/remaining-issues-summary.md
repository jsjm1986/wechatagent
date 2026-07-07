# 剩余待处理问题汇总（2026-07-07）

基于当前分支 `fix/dispatcher-send-timeout-alignment` 的完整梳理。

---

## 一、MCP Server 真实接入（P0 阻断级）

### 1.1 前端集成 AccountLogin 组件 ⚠️ 待完成
**状态**：组件已写好（`frontend/src/features/command-center/AccountLogin.tsx`），但未添加到路由

**需要**：
- [ ] 在前端路由中添加 `/command-center/account-login` 路由
- [ ] 在账号管理页面添加"登录微信账号"入口按钮
- [ ] 或在账号列表添加"添加账号"按钮跳转到登录页

**工作量**：~30 分钟

**阻断影响**：无法从前端 UI 发起微信账号登录，必须手动在 MCP Guide 页面登录

---

### 1.2 MCP Server 配置 Webhook URL ⚠️ 待配置
**状态**：后端 webhook 端点已实现（`POST /webhooks/wechat`），但 MCP Server 侧未配置推送地址

**需要**：
- [ ] 在 MCP Server 管理后台配置 `messageWebhookUrl = https://<域名>/webhooks/wechat`
- [ ] 或通过 MCP 工具 `set_webhook_url` 配置（如果支持）
- [ ] 验证 webhook 签名（`.env` 的 `MCP_API_KEY` 与 MCP Server 一致）

**工作量**：管理员操作 ~10 分钟

**阻断影响**：消息无法推送到本项目，webhook 永远不会被调用，自动回复无法触发

---

### 1.3 端到端测试 ⚠️ 待验证
**状态**：技术基础已完成，需真实环境验证业务闭环

**测试清单**：
- [ ] 登录流程：前端扫码 → 同步账号 → DB 有记录（`wechat_accounts` 表 alias/wxid/online）
- [ ] 消息推送：发消息 → webhook 收到 → `conversation_messages` 有 inbound 记录
- [ ] 自动回复：webhook 触发 → gateway 决策 → MCP 发送 → `mcp_logs` 有 `message_send_text` 记录
- [ ] 账号状态同步：在线/离线事件正确更新 `wechat_accounts.online`

**工作量**：~1 小时

**阻断影响**：无法确认生产环境可用

---

## 二、MCP Server 真实接入（P1 核心级）

### 2.1 前端账号管理页面增强 ⚠️ 部分实现
**当前**：账号列表有基本展示（alias/displayName/online 状态点）

**缺失**：
- [ ] 完整的账号详情页（wxid/nick_name/mcp_api_key/last_sync_at）
- [ ] 登录状态展示（区分"从未登录"vs"已登录但掉线"）
- [ ] 一键操作按钮（"登录"/"同步"/"重连"/"登出"）
- [ ] 在线状态实时刷新（WebSocket 或轮询）

**工作量**：~2-3 小时

**影响**：体验欠佳，无法直观看到账号状态和一键操作

---

### 2.2 定时同步账号状态 worker ⚠️ 缺失
**当前**：只有手动调用 `POST /api/accounts/sync` 或前端登录成功后自动同步

**需要**：
- [ ] 后台 worker 定期（如每 5 分钟）调用 MCP `account_list` 同步账号状态
- [ ] 或订阅 MCP 账号事件（如果 MCP 支持 SSE 推送）
- [ ] 更新 `wechat_accounts` 表的 `online`/`last_sync_at`

**工作量**：~1 小时

**影响**：账号状态长期不准确，掉线后不能及时发现

---

### 2.3 登录成功自动触发 sync ⚠️ 前端实现，后端未实现
**当前**：前端 `AccountLogin.tsx` 登录成功后调用 `/api/accounts/sync`

**优化**：
- [ ] 后端 `login_poll` 检测到 `status=success` 时自动调用 sync
- [ ] 减少前端逻辑，确保即使前端跳过也能同步

**工作量**：~30 分钟

**影响**：前端实现已足够，后端实现更健壮

---

### 2.4 账号登录状态字段 ⚠️ 缺失
**当前**：`wechat_accounts.online` 只有布尔值，无法区分"从未登录"vs"已登录但掉线"

**需要**：
- [ ] 新增字段 `login_status: Enum<never_logged_in | logged_in | offline>`
- [ ] migration 迁移现有数据（`online=false` → 检查 `wxid` 是否有值判断是 `never_logged_in` 还是 `offline`）
- [ ] 前端展示不同状态的图标和文案

**工作量**：~1 小时

**影响**：UI 展示不够精确，用户不知道是"未登录"还是"掉线"

---

## 三、已知技术债（P2 增强级）

### 3.1 账号掉线告警 ⚠️ 缺失
**需要**：
- [ ] 检测到账号 `online: false` 时发送告警（邮件/Slack/钉钉）
- [ ] 配置告警接收人和频率（避免频繁告警）

**工作量**：~2 小时

**影响**：账号掉线后无人知晓，影响业务连续性

---

### 3.2 Webhook 配置界面 ⚠️ 缺失
**当前**：webhook URL 需要在 MCP Server 管理后台手动配置

**优化**（如果 MCP 支持）：
- [ ] 前端界面配置 webhook URL（调用 MCP 工具 `set_webhook_url`）
- [ ] 展示当前配置的 webhook URL
- [ ] 测试 webhook 连通性（发送测试消息）

**工作量**：~2-3 小时（需先确认 MCP 是否支持此工具）

**影响**：配置流程依赖外部管理后台，不够自助

---

### 3.3 MCP 工具目录刷新 ⚠️ 缺失
**当前**：`tools/list` 只在启动时调用一次，MCP Server 新增工具后前端看不到

**需要**：
- [ ] 前端"刷新工具列表"按钮，调用 `GET /api/management/tools/list`
- [ ] 后端缓存 tools 列表，TTL 5 分钟，支持强制刷新

**工作量**：~1 小时

**影响**：MCP Server 更新工具后需要重启后端才能使用

---

## 四、端到端测试 findings（参考历史，非当前阻断）

### 4.1 LLM 可用性（C 类 BLOCKED）
**来源**：`docs/superpowers/specs/2026-06-26-full-business-logic-test-findings.md`

**现象**：文章进库测试失败，LLM 返回 503 Service Unavailable

**根因**：上游 LLM 平台侧问题（非本项目 bug）

**处理**：标记为 C 类 BLOCKED，测试时先探活 LLM 端点，不健康则跳过相关测试

**当前状态**：已在审查报告中标注，非阻断

---

### 4.2 MCP 依赖工具（C 类 BLOCKED）
**来源**：`docs/smoke/2026-07-05-newuser-journey-four-way-audit.md`

**3 项 MCP 依赖已标 BLOCKED**：
1. 联系人导入 query（`contacts_search`）
2. AI 总控编排（MCP `chat_complete`）
3. Webhook 发送步（`message_send_text`）

**当前状态**：MCP Server 已接入，这 3 项应该可以解除 BLOCKED（待端到端测试验证）

---

## 五、优先级排序

### 🔴 P0 阻断级（必须完成才能生产部署）
1. **MCP Server 配置 Webhook URL**（管理员操作 ~10 分钟）
2. **端到端测试**（验证业务闭环 ~1 小时）
3. **前端集成 AccountLogin 组件**（~30 分钟）

### 🟡 P1 核心级（影响体验但不阻断）
4. 前端账号管理页面增强（~2-3 小时）
5. 定时同步账号状态 worker（~1 小时）
6. 账号登录状态字段 + migration（~1 小时）

### 🟢 P2 增强级（锦上添花）
7. 账号掉线告警（~2 小时）
8. Webhook 配置界面（~2-3 小时，需先确认 MCP 支持）
9. MCP 工具目录刷新（~1 小时）
10. 登录成功自动触发 sync（后端实现 ~30 分钟）

---

## 六、建议行动路径

### 阶段 1：立即可做（本地开发）
1. ✅ 前端集成 AccountLogin 组件（~30 分钟）
2. ✅ 提交代码并合并到 main

### 阶段 2：部署后验证（需真实环境）
3. ⚠️ 在 MCP Server 配置 webhook URL
4. ⚠️ 运行端到端测试（登录 + 消息推送）
5. ⚠️ 验证通过后解除 3 项 C 类 BLOCKED

### 阶段 3：体验增强（迭代优化）
6. 前端账号管理页面增强
7. 定时同步 worker
8. 账号登录状态字段
9. 其他 P2 增强项

---

## 七、当前可立即提交的代码

```bash
git add docs/mcp-integration-completion-report.md
git commit -m "docs(mcp): 添加 MCP 接入完成报告和剩余问题汇总"
git push origin fix/dispatcher-send-timeout-alignment
```

---

**总结**：P0 的 3 项中，2 项需要部署后操作（配置 webhook + 测试），1 项可以本地完成（前端集成组件）。建议先完成本地可做的，提交代码，部署后再验证。
