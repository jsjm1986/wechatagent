# 部署步骤 - WechatAgent MCP 接入版本

**分支**: `fix/dispatcher-send-timeout-alignment`  
**目标服务器**: 117.72.54.28  
**部署日期**: 2026-07-07

---

## 步骤 1: 在服务器上拉取最新代码

```bash
# SSH 登录服务器
ssh root@117.72.54.28

# 进入项目目录
cd /root/wechatagent

# 拉取最新代码
git fetch origin

# 查看待合并的 commits
git log --oneline origin/fix/dispatcher-send-timeout-alignment ^origin/main

# 应该看到 14 个 commits:
# c960af7 docs: 添加端到端测试 + MCP 接入完整工作总结
# 937a65c docs(mcp): 添加 MCP Server 真实接入最终交付总结
# 2966497 feat(frontend): 完成 P0-3 前端账号管理集成
# 35a6125 feat(mcp): 补全 P0 阻断级缺口——微信登录流程 + Webhook 配置文档
# 9c34d80 feat(mcp): 支持 Workspace Key + account_alias 自动注入
# b640ce4 feat(mcp): 支持 MCP Streamable-HTTP 会话握手 + SSE 响应解析
# 102af33 docs(smoke): B-1 修复后端到端复验记录
# 6410ff4 fix(gateway): 升档 run 授予更高 token 上限
# 301f88a feat(config): 加 run_token_budget_escalated 配置字段
# 66789e0 feat(budget): RunBudget 增加可授予的升档 token 上限
# ... (其他 commits)
```

---

## 步骤 2: 合并代码到 main

```bash
# 切换到 main 分支
git checkout main

# 合并 fix/dispatcher-send-timeout-alignment 分支
git merge origin/fix/dispatcher-send-timeout-alignment

# 如果有冲突，解决冲突后：
git add .
git commit -m "Merge fix/dispatcher-send-timeout-alignment"

# 推送到远程
git push origin main
```

---

## 步骤 3: 更新环境变量

编辑 `.env` 文件，添加/更新以下配置：

```bash
# MCP Server 配置
MCP_BASE_URL=http://117.72.54.28:3001
MCP_API_KEY=gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f

# Webhook 签名验证（生产环境必须开启）
WEBHOOK_VERIFY_SIGNATURE=true

# B-1 修复：升档 run token 预算（新增配置）
RUN_TOKEN_BUDGET_ESCALATED=100000
```

**重要**: 确认 `MCP_API_KEY` 与 MCP Server 配置完全一致（包括 `gwa_` 前缀）

---

## 步骤 4: 编译后端

```bash
# 编译 release 版本
cargo build --release

# 编译时间约 10-15 分钟，请耐心等待
# 编译成功后会显示：
# Finished `release` profile [optimized] target(s) in XXm XXs
```

**如果编译失败**，检查：
- Rust 版本是否 >= 1.70
- 依赖是否完整（`cargo fetch`）
- 磁盘空间是否充足

---

## 步骤 5: 编译前端

```bash
# 进入前端目录
cd frontend

# 安装依赖（如果 package.json 有更新）
npm install

# 编译生产版本
npm run build

# 编译成功后会在 frontend/dist 生成静态文件
```

---

## 步骤 6: 重启后端服务

```bash
# 停止旧进程（假设使用 systemd）
systemctl stop wechatagent

# 或使用 pm2
pm2 stop wechatagent

# 或手动 kill 进程
pkill -f wechatagent

# 启动新进程
systemctl start wechatagent

# 或使用 pm2
pm2 start wechatagent

# 或直接运行
./target/release/wechatagent

# 检查启动日志
journalctl -u wechatagent -f

# 或
pm2 logs wechatagent

# 应该看到类似日志：
# [INFO] Server listening on 0.0.0.0:8080
# [INFO] MCP base URL: http://117.72.54.28:3001
# [INFO] Webhook signature verification: enabled
```

---

## 步骤 7: 验证部署成功

### 7.1 检查后端健康

```bash
# 本地 curl（服务器上）
curl http://localhost:8080/api/health

# 预期响应：
{"status":"ok"}
```

### 7.2 检查前端可访问

```bash
# 浏览器访问
http://117.72.54.28:8080

# 或公网域名
https://your-domain.com

# 应该看到前端界面正常加载
```

### 7.3 检查新功能可用

```bash
# 1. 检查账号管理频道是否出现在左侧导航
#    - 点击左侧菜单，应该看到"账号管理"频道（图标为 Contact）

# 2. 检查登录端点
curl http://localhost:8080/api/accounts/login/begin \
  -X POST \
  -H "Content-Type: application/json" \
  -H "Cookie: wa_session=<登录后的 session>" \
  -d '{"login_type":"mac","login_flow":"auto"}'

# 预期：返回 qr_data_url、login_page_url、session_id（或 401 未登录）
```

---

## 步骤 8: 配置 MCP Server Webhook URL

### 方式 A: 通过 MCP Server 管理后台（推荐）

1. 浏览器访问：`http://117.72.54.28:3001/admin`
2. 登录管理员账号
3. 进入"账号配置"或"Webhook 设置"页面
4. 为每个微信账号配置 `messageWebhookUrl`:
   ```
   http://117.72.54.28:8080/webhooks/wechat
   ```
   或使用公网域名：
   ```
   https://your-domain.com/webhooks/wechat
   ```
5. 保存配置

### 方式 B: 通过 MCP 工具（如果支持）

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
        "webhook_url": "http://117.72.54.28:8080/webhooks/wechat"
      }
    }
  }'
```

**验证 Webhook 配置**:
```bash
# 发送测试 POST 请求
curl -X POST http://117.72.54.28:8080/webhooks/wechat \
  -H "Content-Type: application/json" \
  -d '{}'

# 预期：
# - 如果签名验证开启：401 Unauthorized（正常，因为没有签名）
# - 如果签名验证关闭：400 Bad Request（正常，因为消息体不合法）
# - 如果 404：说明端点不可达，检查路由配置
```

---

## 步骤 9: 端到端测试

### 9.1 测试微信账号登录

1. 浏览器访问前端：`http://117.72.54.28:8080`
2. 登录管理员账号
3. 点击左侧菜单"账号管理"
4. 点击"登录微信账号"按钮
5. 填写配置：
   - Account Alias: `t-1`（如果使用 Workspace Key）
   - 登录平台: Mac
   - 登录流程: Auto
6. 点击"开始登录"
7. 使用微信扫描二维码
8. 登录成功后，应该自动跳转回账号列表，看到新账号

### 9.2 验证账号同步

```bash
# 方式 1: 前端点击"同步账号"按钮

# 方式 2: 手动调用 API
curl -X POST http://117.72.54.28:8080/api/accounts/sync \
  -H "Cookie: wa_session=<登录后的 session>"

# 检查 MongoDB
mongo
> use wechatagent
> db.wechat_accounts.find().pretty()

# 预期看到：
{
  "_id": ObjectId("..."),
  "workspace_id": "...",
  "account_id": "...",
  "alias": "t-1",
  "wxid": "wxid_xxx",
  "nick_name": "昵称",
  "online": true,
  "mcp_api_key": "gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f",
  "mcp_base_url": "http://117.72.54.28:3001",
  ...
}
```

### 9.3 测试消息推送

1. 使用另一个微信号向已登录的账号发送消息："你好"
2. 检查后端日志：
   ```bash
   tail -f /var/log/wechatagent.log
   # 或
   journalctl -u wechatagent -f
   ```
3. 应该看到类似日志：
   ```
   [INFO] wechat_webhook: received webhook event, appId=xxx, typeName=Text, fromWxid=xxx
   [INFO] wechat_webhook: enqueued managed message for contact xxx
   ```

### 9.4 验证自动回复

1. 等待几秒，检查是否收到自动回复
2. 查看 MongoDB 数据：
   ```javascript
   // 检查入站消息
   db.conversation_messages.find({direction: "inbound"}).sort({created_at:-1}).limit(1)
   
   // 检查 Agent run 记录
   db.agent_runs.find().sort({created_at:-1}).limit(1)
   
   // 检查 MCP 调用日志
   db.mcp_logs.find({tool_name: "message_send_text"}).sort({created_at:-1}).limit(1)
   ```

### 9.5 验证升档路径（B-1 修复）

1. 使用一个**从未联系过的**新微信号发送消息
2. 检查 Agent run 是否成功（不被 `blocked_by_budget` 拦截）
3. 查看 MongoDB：
   ```javascript
   db.agent_runs.find({contact_wxid: "<新联系人wxid>"}).sort({created_at:-1}).limit(1)
   
   // 预期：
   // - gateway_status 不是 "blocked_by_budget"
   // - 如果首次触发走了升档路径，token_budget_granted 应该是 100000
   ```

---

## 步骤 10: 检查点清单

部署完成后，逐项确认：

- [ ] 后端编译成功（`cargo build --release` 无错误）
- [ ] 前端编译成功（`npm run build` 无错误）
- [ ] 后端服务启动成功（日志显示监听 8080 端口）
- [ ] 前端界面可访问（浏览器打开正常）
- [ ] 新增"账号管理"频道出现在左侧菜单
- [ ] 环境变量正确配置（`.env` 包含 MCP_BASE_URL/MCP_API_KEY/RUN_TOKEN_BUDGET_ESCALATED）
- [ ] MCP Server 已配置 webhook URL
- [ ] Webhook 端点可达（curl 测试返回 401 或 400，不是 404）
- [ ] 微信账号登录流程可用（扫码成功）
- [ ] 账号同步成功（`wechat_accounts` 表有记录）
- [ ] 消息推送成功（后端日志有 webhook event）
- [ ] 自动回复触发（`mcp_logs` 有 message_send_text）
- [ ] 升档路径工作（新联系人不被预算拦截）

---

## 步骤 11: 解除 C 类 BLOCKED 标记

如果以上测试全部通过，更新文档：

```bash
# 编辑 docs/smoke/2026-07-05-newuser-journey-four-way-audit.md
# 找到 C 类 BLOCKED 的 3 项，标记为已解除：

## C 类 BLOCKED（MCP 依赖）

1. ~~联系人导入 query（contacts_search）~~ ✅ 已解除（2026-07-07 部署后验证通过）
2. ~~AI 总控编排（MCP chat_complete）~~ ✅ 已解除（2026-07-07 部署后验证通过）
3. ~~Webhook 发送步（message_send_text）~~ ✅ 已解除（2026-07-07 部署后验证通过）
```

---

## 常见问题排查

### Q1: 编译失败 "linker `cc` not found"
```bash
# 安装 build-essential
apt-get update && apt-get install -y build-essential
```

### Q2: 前端编译失败 "out of memory"
```bash
# 增加 Node.js 内存限制
export NODE_OPTIONS="--max-old-space-size=4096"
npm run build
```

### Q3: 后端启动后立即退出
```bash
# 检查端口占用
lsof -i :8080

# 检查 .env 文件格式
cat .env | grep -v "^#" | grep "="

# 检查日志
journalctl -u wechatagent -n 50
```

### Q4: Webhook 收不到消息
```bash
# 1. 检查 MCP Server 是否配置了 webhook URL
curl http://117.72.54.28:3001/mcp \
  -H "Authorization: Bearer gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"1","method":"tools/call","params":{"name":"account_list","arguments":{}}}'

# 2. 检查防火墙是否允许 MCP Server 访问
iptables -L -n | grep 8080

# 3. 检查后端日志是否有签名验证错误
grep "invalid signature" /var/log/wechatagent.log

# 4. 临时关闭签名验证测试
# 修改 .env: WEBHOOK_VERIFY_SIGNATURE=false
# 重启后端，再次测试
```

### Q5: 账号登录后前端不显示
```bash
# 1. 手动同步账号
curl -X POST http://localhost:8080/api/accounts/sync \
  -H "Cookie: wa_session=<session>"

# 2. 检查 MongoDB
mongo
> use wechatagent
> db.wechat_accounts.find()

# 3. 检查前端 API 调用
# 浏览器 F12 → Network → 查看 /api/accounts 请求是否成功
```

---

## 回滚步骤（如果部署失败）

```bash
# 1. 回滚代码到之前的稳定版本
git checkout <previous-stable-commit>

# 2. 重新编译
cargo build --release
cd frontend && npm run build

# 3. 重启服务
systemctl restart wechatagent

# 4. 恢复 .env 配置（如果有改动）
git checkout .env
```

---

## 部署完成后通知

部署成功后，在项目文档中更新：

1. `docs/FINAL-SUMMARY-2026-07-07.md` — 标记"部署完成日期"
2. `docs/remaining-issues-summary.md` — 更新 P0 状态为"已验证"
3. `docs/mcp-integration-final-delivery.md` — 添加"生产环境验证通过"标记

---

**预计总时间**: 30-60 分钟（取决于编译速度和网络状况）

**关键联系人**: 如遇问题，参考文档或联系开发团队
