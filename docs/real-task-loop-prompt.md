# 长期运营Agent 能力跑通 · 启动提示词

> 第一性目标：把 WechatAgent 的"自运营 / 自优化 / 自治理"运营 Agent，按 `docs/real-task-runbook.md` 全量功能矩阵跑到生产可用，每一项已声明能力至少被验证一次。

进来就执行下列 7 步，不要先解释、不要先问。

## Step 0  并行 Read 三份文件（决策依据）

- `docs/real-task-runbook.md`（全文，循环执行底稿）
- `CLAUDE.md`（红线 / baseline 真实约束源）
- `src/agent/run_envelope.rs`（`FINAL_REVIEW_STATUS_VALUES` / `GATEWAY_STATUS_VALUES` 真实枚举源，行 60-180）

## Step 1  锁定本轮 Round 号（footer JSON 优先 + 4 项 ≥ 4 的四分支表）

读 runbook §8（Round 历史日志）+ §8.1（footer JSON 模板 + 历史 footer）。

a. 收集所有形如 `### Round N` 的标题，取最大编号 `maxTitle`
b. 收集所有合法 footer JSON（含 `round_no` 字段），取已有 footer 的最大 `round_no` 即 `maxFooter`，并组成 `footerSet`
c. 决策（严格按下表，不许补任何"用户主动收尾 / 已归档 / 看上去结束了"之类的脑补条件）：

| 条件 | 行为 |
|---|---|
| `maxTitle` 不在 `footerSet`（该 Round 标题已开但 footer 缺失） | **续跑该 Round**（Round 号 = `maxTitle`） |
| `maxTitle` 在 `footerSet` 且 `maxFooter` 这一轮 4 项自评 ≥ 4 项 ≥ 4 分，且**前一轮也 ≥ 4 项 ≥ 4** | 触发 §0.2 收口决策：**停跑**，输出收口 JSON，不再开新 Round |
| `maxTitle` 在 `footerSet` 但不满足"连续两轮 ≥ 4/5" | **开新 Round**，Round 号 = `maxTitle` + 1 |
| §8 完全为空 | **开 Round 1** |

runbook 行 5 的 Round 锚点是辅助说明，决策按本 Step 1 表为准。

【严禁补丁规则】Step 1 的判定输入只有 "§8 标题集合 + footer JSON 集合"。任何 §8 自然语言（"用户主动收尾"/"已归档"/"我已停止"/"先这样吧"）一律不参与 Round 号决策；如需"放弃续跑直接开新 Round"，按 runbook §8.0 Round 状态机要求先手动写一条 footer JSON。

## Step 2  环境自检

- `curl -s http://localhost:8080/api/health` 返回 200 → 跳过启动序列
- 非 200 / 端口未监听 → 严格按 runbook §2 全 6 步启动（Step 1/6 cargo build → Step 2/6 cargo run → Step 3/6 健康轮询 90s → Step 4/6 MCP sync → Step 5/6 联系人核验 → Step 6/6 Outbox dispatcher 心跳确认）
- 联系人核验：contacts/search-import 对 `fengrui86` 命中 0 → 重启后端 + 重试，连续 ≥ 3 次仍 0 命中再写 §9 issue（不是直接红线）

## Step 3  写本轮 header（5 行）

```
### Round N（YYYY-MM-DD HH:MM CST 起）
- 焦点：{从 §4 未通过的 Sxx + §9 未关 issue 提炼，列具体场景号或文字描述}
- 新增 Sxx：{若有新场景从 §4 矩阵已有列里选，没有写"无"}
- 退出条件：本轮跑完写 §8.1 footer JSON + 按 runbook §0.2 决策树继续 / 停
- 开始执行 runbook §5 七步循环（第 1 步：盲区核对）
```

## Step 4  TaskCreate 7 个任务

对应 runbook §5 七步：

1. 盲区核对（按 §5 第 1 步）
2. 列本轮 Sxx 焦点
3. 投递 + 观察（按 §5 第 3 步，每条投递后立即从 `agent_run_logs` 拿 `run_id`）
4. 证据填写（§4 / §4.0 矩阵更新）
5. 修复缺口（如需要）
6. 回归扫描（§5 第 6 步）
7. 自评 + 写 §8.1 footer JSON（§5.1 自评表 + §0.2 决策）

## Step 5  按 runbook §5 七步执行

每场景投递后**立即**用 mongo / API 观察 `agent_run_logs`，拿到 `run_id` 再写 §4 / §4.0 证据，不要积压。
所有发送只能流经 `agent::run_user_operation_gateway`（CLAUDE.md 已强约束）。
新场景的"期望证据"列必须对齐 §4.0 / runbook header 状态枚举速查（D1/D2 已修订，gateway / final_review_status 真实集合见 `src/agent/run_envelope.rs`）。

## Step 6  本轮收尾

- 把本轮过程总结写入 runbook §8 的 `### Round N` 节（追加，不覆盖历史）
- 写 §8.1 footer JSON（4 项自评必填）
- 按 runbook §0.2 决策树判断"开新 Round / 停跑收口 / 写 §9 红线"

---

## 流程级硬约束（违反即红线，本提示词唯一显式列出的 4 条）

1. webhook payload 中 `content` 必须是 UTF-8 中文（用 `python scripts/rt_send.py <slot> "<中文>"` 投递；不要直接在 win-bash 用 curl 拼参数，会被 GBK 打坏）
2. 测试只发 Jsjm（`fromWxid=fengrui86`，appId=`wx_wi_8NITtM8d0csT6tYDYX`），其他联系人一律不发
3. 所有发送必须流经 `agent::run_user_operation_gateway`，禁止直连 MCP / 直插 outbox
4. 禁词集（`human / 人工 / 接管 / takeover / hand-off`）在 `src/agent/`、`src/routes/`、`src/evolution/`、`frontend/src/` 0 命中（CI lint 兜底，本地新增代码自查）

## git 安全

- 不改 git config
- 不 force push / reset --hard / clean -f
- 不 commit / 不 push 除非用户显式要求
- 测试期间 `src/` 改动用 `git stash` 暂存或单独 branch，不直接污染 main

## 红线

runbook §6 R1–R9（9 条，每条带 `(red-line:RN)` tag），本提示词不复述。命中任意 1 条立刻停 Round + 写 §9。

## 状态枚举对齐

`final_review_status` / `gateway_status` / `agent_events.details.reason` 三层信号区分见 runbook header（行 5-13）+ `src/agent/run_envelope.rs` 行 60-180。写新场景"期望证据"前先比对，不许把模式名（如 `local_decision_review`）当 `final_review_status` 写。

## 资源指针（不要再问）

- 决策树：runbook §0.2
- 启动序列：runbook §2（6 步）
- 矩阵：runbook §4 + §4.0
- 七步循环：runbook §5 / 单场景探针 §5.2
- 自评表：runbook §5.1
- 红线：runbook §6（R1-R9）
- Round 状态机：runbook §8.0
- footer 模板：runbook §8.1
- issue 写法：runbook §9
- 启动 / 数据库 / outbox / 工具速查：runbook §10.1–10.7

## 子代理使用

- 单个具体功能验证（投递 + 抓 `run_id` + 打分） → 主线程直接做，不要起子代理
- 大范围 grep / 多文件读 / 跨模块审查 → Agent + `subagent_type=Explore`（只读）
- 改代码 → 主线程做，不要把代码修改委托给子代理（子代理改完看不到 diff，会引入红线）

## 参数 $ARGUMENTS

- 留空 → 走默认全量循环（Step 1-6）
- 填具体场景号（如 `S2 S3`） → 走 runbook §5.2 单场景探针模式：不写 §8 Round 节、不写 §8.1 footer、不计入"连续 2 轮 ≥ 4/5"判定，只跑指定场景 + 输出单条 JSON

## 绝不出现的反模式

- 把"用户主动收尾 / 看起来已归档"当 Round 决策依据
- 跳过 Step 0 直接执行
- webhook 用拼字符串的 curl（GBK 打坏中文）
- 给 Jsjm 之外的联系人发消息
- 直接调 MCP 绕过 gateway
- 把 `src/` 改动 commit / push 上 main 不经用户允许
- 在 §4 / §4.0 写未经实际投递验证的"期望证据"

---

起跑。从 Step 0 开始，三份文件并行 Read。
