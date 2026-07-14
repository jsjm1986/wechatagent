# 以「手动加入运营」为唯一真相 —— 非真人号被自动回复缺陷治本设计

> 状态：设计已获批，待写实现计划（writing-plans）。
> 日期：2026-07-14

## 背景与根因

生产库中 AI 在给一批**非真人微信号**自动回复（nickname="福州晚报"的新闻号、"福建经济广播"电台号、"AI应用开发"营销号、甚至账号 102 自己的 wxid "Demi"）。AI 对这些号推送的新闻/链接卡片每隔几小时机械回同一句「收到您分享的链接啦～方便文字说下您想了解哪方面吗？」——本质是给公众号/营销号狂发无意义骚扰，且福州晚报已真发 22 条、福建经济广播 5 条。

### 已亲验的根因链（file:line 均已核实）

1. **回复门只看 agent_status**：`reload_managed_contact`（`src/webhooks.rs:273-287`）只 `filter(agent_status == Managed)`。webhook 主流程 `wechat_webhook`（`src/webhooks.rs:603-665`）只有 `managed` 才进 `run_debounce_pipeline` 自动回复。**normal 只落库不回复——此规则正确且生效**。
2. **升 managed 的真实写库路径只有 3 个，全部是前端管理员手动操作**：`enable_agent`（`src/routes/contacts.rs:884`，单个）、`batch_enable_endpoint`（`src/routes/contacts.rs:719`，批量）、`management.rs:1383`（管理页）。worker/planner/campaign 里的 `agent_status:"managed"` 全是**查询过滤条件**（在已 managed 的人里挑跟进对象），无任何自动升级路径。
3. **非真人号是被「从运营池批量启用」加进来的**：铁证是 `human_profile_note` 字段——福州晚报/福建经济广播/Demi = `"从运营池批量启用"`，AI应用开发 = `"我的好朋友 也是老客户"`。管理员批量框选时把非真人号混选进去，`batch_enable` 照单全收。
4. **移出池不停回复（矛盾态根因）**：`hide_from_pool`（`src/routes/contacts.rs:969-989`）只写 `hidden_from_pool:true`，**不改 agent_status**。回复门不看 `hidden_from_pool`。→ 生产 4 个号 managed+hidden：从运营池列表消失，却仍在自动回复。
5. **判据盲区**：`is_operatable_person`（`src/webhooks.rs:1065-1069`）= `!(gh_前缀 || @chatroom || is_system_account)`。营销号/新闻号用普通 `wxid_` 前缀天然过闸（单测 `webhooks.rs:1481` 明确断言福州晚报 == true）；`@openim` 企业微信号不拦；账号自身 wxid 不拦（单测 `webhooks.rs:1470` 断言账号自己 == true）。
6. **加入/移出运营无任何审计日志**：三个 enable 端点与 hide_from_pool 从头到尾无 `events` 写入，谁在何时把哪些号加入/移出运营全无痕迹。

## 设计决策（核心方向）

**放弃用系统自动判据识别「是不是真人」**——`wxid_` 营销号纯靠 wxid 字符特征判不准，硬判会误伤真人。**以管理员手动加入运营为唯一真相**：加入/移出是显式动作、可审计、语义自洽（移出=停回复、账号不能运营自己）。系统只保留「硬事实」拦截（公众号/群/企业微信号不是私聊真人）与「逻辑铁律」拦截（账号不能运营自己），不做「真人猜测」。

## 四部分设计

### 第 1 部分：判据取向 —— 保留硬事实拦 + 补盲区，放弃营销号自动判据

`is_operatable_person`（`src/webhooks.rs:1065-1069`）：
- **保留** `gh_` 前缀（公众号）、`@chatroom`（群）拦截 —— 这是硬事实（公众号/群不是私聊真人联系人），非「猜真人」。
- **新增** `@openim` 后缀拦截（企业微信/开放 IM 号，非私聊真人）。**必须同步**在 `non_human_exclusion_filter`（`src/mcp.rs:518-528`）的 `$nor` 数组补 `@openim` 正则——该 DB 侧过滤器被 count 端点（`contacts.rs:265`）与 `mcp.rs:974` 复用，与 Rust 侧 `is_operatable_person` 是声明式同源，任一处补漏另一处会导致 count/list 口径漂移（历史坑 `bug_pool_counts_vs_list_non_human_drift`）。
- **不新增** `wxid_` 营销号拦截 —— 放弃自动判据，营销号能否运营交给管理员手动决定。

**新增独立函数 `is_self_account(wxid, account_self_wxid)`**（自反身判断，与真人判据**解耦**，不塞进 `is_operatable_person`）：判断某 wxid 是否等于当前账号自身 wxid。语义是「不能自己运营自己」的逻辑铁律，不是「真人判断」。

### 第 2 部分：加入运营入口统一自反身硬拦

三个手动启用入口统一加一道**自反身硬拦**（复用各端点已查出的 `WechatAccount.wxid`，见 `src/models.rs:65` `pub wxid: Option<String>`；`enable_agent`/`batch_enable` 已在 `contacts.rs:733/901` 查 `accounts()` 做注册校验）：
- `batch_enable_endpoint`（`contacts.rs:719`）：遍历 candidates 时，`cand.wxid == account.wxid` 的跳过并计数（返回体加 `rejected_self` 计数）。
- `enable_agent`（`contacts.rs:884`）：目标 wxid == account.wxid → 返回 400 BadRequest。
- `management.rs:1383` 启用路径：同 enable_agent。
- **不加 `wxid_` 营销号拦截**（遵照决策，营销号手动决定）。
- webhook 建档侧（`upsert_webhook_contact`，`webhooks.rs:1071`）补一道 `is_self_account` 兜底：账号自身 wxid 不建 contact（防账号自反身记录进库）。

**取舍**：自反身拦截放「加入运营」这道关（伤害真正发生处 = 自己回自己），而非只在建档拦（拦不住已存在的账号自身记录）。

### 第 3 部分：移出池联动停回复 + 加入/移出写审计日志

**A. 移出池联动停回复**（修 4 个矛盾号直接根因）：
- `hide_from_pool`（`contacts.rs:969-989`）改为 `$set` 时**同时写** `agent_status:"normal"` + `hidden_from_pool:true`。从池移除即停止 AI 回复。
- 语义：`hidden_from_pool=true` 恒等于「不再运营」。
- 回复门（`webhooks.rs:286`）**保持只看 agent_status 不变** —— 移出已在写入侧联动改了 agent_status，无需在读取侧再加 hidden 过滤（**单一真相源**：在写入侧保证一致，比读取侧多一道过滤更干净）。

**B. 加入/移出写审计日志**（当前完全缺失）：
复用 `write_event_for_account(state, account_id, contact_wxid, kind, status, summary, details)`（`src/agent/gateway.rs:5119`，写 `events` 集合，admin 可见）：
- 加入运营（3 个 enable 入口成功时）：`kind="contact.enabled_for_ops"`，details 记 admin 标识、shared_note/note、playbook_id。
- 移出运营（hide_from_pool + `disable`）：`kind="contact.removed_from_ops"`。
- 自反身被拦：`kind="contact.enable_rejected_self"` 留痕。
- **文案红线**：kind/summary/details 文案不得含 `人工/接管/转接/takeover/hand-off` 等 no-human-takeover lint 禁词，用 AI 自治语义命名（如「纳入 AI 运营 / 移出 AI 运营」）。

### 第 4 部分：存量矛盾号即时清理 + 测试

**A. 存量清理**（即时止血，与代码修复解耦）：
- 生产 4 个 managed+hidden 矛盾号 → `agent_status:"normal"`：
  - 福州晚报 `wxid_8874178741811`、福建经济广播 `wxid_2540165401612`、AI应用开发 `wxid_czpvyjvhzizj22`、Demi=账号102自己 `wxid_3yeirsb75afd22`。
- **形态**：一次性 mongosh 更新（清历史脏数据，非结构变更 → 不用 migration，migration 每次启动跑不合适）。
- **执行前先备份这 4 条当前 `agent_status`/`hidden_from_pool`**，可回滚。
- `@openim` 号 `25984984932102183@openim`：当前 managed 但 runs=0 未造成伤害，一并核查并按同口径处理（改 normal）。

**B. 测试**：
- 单测：`is_self_account` 判定（等于账号 wxid → true）；`is_operatable_person` 对 `@openim` 返 false、对 `gh_`/`@chatroom` 仍 false、对普通 `wxid_` 仍 **true**（营销号不拦的回归保护）。
- enable/batch_enable 自反身拦截测试（账号自身 wxid → 拒绝/跳过）。
- `hide_from_pool` 联动测试：调用后 `agent_status == normal` 且 `hidden_from_pool == true`。
- 基线门 `cargo test --lib` 不回归；`scripts/check-baseline` 双门绿；`check-no-human-takeover` / `check-no-model-hint` 两 lint 门（新增行/审计文案避禁词）。

**C. 存量单测修正**：
- `webhooks.rs:1470`（账号自身 `wxid_3yeirsb75afd22` 断言 `is_operatable_person==true`）：**保留 true 不变**。理由：`is_operatable_person` 只判「形态上是否私聊真人 wxid」（账号自身 wxid 形态上确实是普通 wxid），账号自反身是**独立维度**由新函数 `is_self_account` + enable 门 + 建档兜底覆盖。二者语义正交、不冲突，该断言无需改。新增对 `is_self_account("wxid_3yeirsb75afd22", account_self="wxid_3yeirsb75afd22")==true` 的独立单测。
- `webhooks.rs:1481`（福州晚报 `is_operatable_person==true`）：**保留 true**（不在此层拦营销号）。

## 不做的事（YAGNI / 明确排除）

- 不做 `wxid_` 营销号自动识别/拦截（判不准，交手动）。
- 不在回复门加 `hidden_from_pool` 过滤（改用写入侧联动，单一真相源）。
- 不加「移出池恢复」端点（原 hide_from_pool 注释已声明 YAGNI）。
- 不改回复门 `reload_managed_contact` 的 agent_status 唯一判据语义。

## 影响面

- 后端：`src/webhooks.rs`（判据 + 建档兜底）、`src/routes/contacts.rs`（enable/batch_enable/hide_from_pool）、`src/routes/management.rs`（启用路径）、`src/mcp.rs`（`@openim` 若并入 `non_human_exclusion_filter` DB 侧口径需同步）。
- 数据：生产一次性清理脚本（4~5 个存量号）。
- 前端：无必须改动（enable 返回体加 rejected_self 计数为可选增强，不阻断）。
