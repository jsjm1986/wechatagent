# 隔离业务矩阵验收裁定

**日期**: 2026-08-14  
**会话**: Cursor「服务器部署测试」（`gpt-5.6-sol`，composer `9ad299bc`）收口  
**分支**: `fix/dependency-security-remediation`（未提交）  
**正式服务**: 未切换、未重启

---

## 裁定

候选代码的定向高风险域已经在隔离环境通过。严格全矩阵 `full-4` 在域 1–6 与 campaign 通过后，被上游 DeepSeek `HTTP 402 Insufficient Balance` 挡住剩余 LLM 域。这是外部账户余额阻塞，不是候选回归证据。

**现在不能把候选切到正式服务。** 余额恢复后，只复跑被 402 挡住的域；1–6 与 campaign 不必重跑。

---

## 身份

| 项 | 值 |
|---|---|
| 候选目录 | `/opt/wechatagent/releases/bizfix-20260813T172538Z-memory-semantic/app` |
| 候选 ELF SHA-256 | `b2eccd443d6746a5330809391ffb09bb3790ad7895476d6e030c1e85d7bcec94` |
| 运行器 | `/opt/wechatagent/releases/wechatagent_isolated_targeted_handoff.sh` |
| 证据前缀 | `.../switch-20260812T134009Z/handoff-full-20260813T204752Z-57` |
| 正式 PID | `1020101`（2026-08-12 21:40:14 CST 起，NRestarts=0） |
| 正式 ELF SHA-256 | `9472129e456a9c41353e7312e586c5070f4b81c57446f7a70cab389fc999a3b0` |
| 正式工作目录 | `/opt/wechatagent/releases/bizfix-20260812T112833Z/app` |
| 正式健康 | `GET /api/health` → `{"ok":true}`（2026-08-14 06:05 与 06:12 两次） |

候选哈希 ≠ 正式哈希。隔离测试没有改正式进程。

---

## 产品修复（未提交，未上正式）

接 Codex「weagent部署」剩余失败后，本会话落地并定向验证过：

- 文本先送达后过早清掉同一决策的名片/媒体授权（`webhooks.rs`）
- 投影误刷新发送频控锚
- 记忆投影 Prompt 补齐语义更正协议（玩笑/反讽不得入库；高置信更正走 `conflict` 候选）
- Reviewer 事实视图保留受控 `namecardToSend`
- 特殊折扣请示规则；领导证据可支持「面向该客户」的授权
- Lean/Relational 紧凑状态键
- 验收脚本：真实频控等待、精确 run/task/candidate 绑定、隔离种子禁用非 `biztest_` 名片

本地库测记录为 2577 通过（GPT 会话内）。本次收口未重跑 `cargo test --lib`。

---

## 隔离结果

### 定向（同一候选哈希 `b2eccd…`）

- `handoff-targeted-20260813T203303Z-11100`：GPT 记录为六个高风险域全绿（不得当作全矩阵）
- `handoff-targeted-20260813T202413Z-25042`：`batch_a_domain6`、`batch_b_industry` 退出码 0，清理 0 残留

更早候选哈希上还单独通过过域 ④⑤⑧、域 ⑨ 最小语义链、域 ⑩。

### 严格全矩阵 `full-4`（`BIZTEST_SUITE_MODE=full`）

`run_all` 退出码 1。通过：

- cleanup / preflight
- 域 ①②③④⑤⑥
- campaign
- 结束 cleanup

被 402 挡住（日志均含 `LLM HTTP 402` / `llm_unavailable` / `account_unavailable`）：

- 域 ⑧⑨⑩⑪⑬
- management / guide / digital-twin / evaluation
- 行业域

预检：`ACTIVE PROVIDER = deepseek-v4-flash`，`VISION=NONE`（域 ⑬ vision 子项本就会标 BLOCKED）。

零残留（`handoff-full-20260813T204752Z-57-final.json`）：

```json
{
  "bizContacts": 0,
  "bizProfiles": 0,
  "bizKnowledge": 0,
  "bizManagement": 0,
  "activeBizProfiles": 0,
  "externalMcpConfig": 0,
  "externalReferralCards": 0,
  "open": [0, 0, 0, 0, 0]
}
```

---

## 2026-08-14 06:12 新鲜探活

对生产库 active provider `deepseek` / `deepseek-v4-flash` / `api.deepseek.com` 发 1 token：

- HTTP 402
- `{"error":{"message":"Insufficient Balance",...}}`

因此未再启动隔离矩阵。再跑只会重复 402，不能增加业务证据。

`.env` 默认网关 `gateway.oeezzk.cn` / `gpt-5.6-auto` 返回 Cloudflare 403（1010），与本次矩阵用的 DeepSeek 不是同一条路径。

---

## 余额恢复后怎么收口

候选目录和 SHA-256 保持上表不变。定向模块：

```text
BIZTEST_TARGET_MODULES=batch_a_domain8,batch_a_domain9,batch_a_domain1011,batch_b_industry,batch_c_digital_twin
```

`batch_a_domain13`、`batch_c_management`、`batch_c_guide`、`batch_c_evaluation` 不在当前 runner 白名单里，要用 `BIZTEST_SUITE_MODE=full` 或先把这四项加进白名单再定向跑。不要重跑已绿的域 1–6 和 campaign，除非候选哈希变了。

通过后才允许讨论切正式。未经单独要求不创建 Git commit。
