# 指挥中心做厚：管理者自然语言操控整个项目（统一管理对话入口）设计

- 日期：2026-06-26
- 状态：设计待复审
- 关联：客户侧 principal decision channel（决策请示通道，同构状态机范本）；agent-first 立场

## 1. 背景与愿景

产品哲学："全 AI 自治"——客户永远只跟 AI 对话，AI 遇到超职权事项向幕后领导请示、拿结论后用自己口吻转述（principal decision channel，**已跑通**）。

本设计把同一个"自主执行 + 越界请示人类 + 人确认后执行"的循环，从**客户侧**延伸到**管理侧**：让管理者把整个项目当成一个 agent 来对话操控——用自然语言下达意图，agent 自己能做的自主做，碰到高风险/宽影响的事项提议改动、汇报给人确认、确认后执行。最终管理者感觉"整个项目就是一个能用自然语言操控的 agent"。

### 1.1 现状（落笔前亲核 origin/main）

**已有雏形**（不新建，做厚）：
- 后端 `src/routes/management.rs`：`create_management_session` / `post_management_message` / `build_management_plan`（NL→意图 plan）/ `execute_management_tool` / dry-run 双层（session 级 + 消息级）/ `get_tool_catalog`。`ManagementPlan{intent, risk_level, requires_confirmation, missing_information, summary, tool_calls[]}`。
- 审计：`AgentCommandRun` / `AgentToolCall` 集合全程留痕。
- 前端 `frontend/src/features/command-center/` 频道 + `commandStore` + 已注册 + 有测试。

**雏形的三个缺口**（本设计要补）：
1. **工具集只有 7 个客户运营态工具**（`wechatagent.search_contacts/import_contacts/enable_contact_agent/disable_contact_agent/create_follow_up_task/send_contact_message/update_contact_profile`，`execute_management_tool` match 在 management.rs:700）。配置/策略/知识的 REST 端点都已存在但没进 catalog。
2. **"提议→确认→执行"循环没闭合**：高风险 plan 标 `pending_confirmation` 且 `take(0)` 不执行（management.rs:191-195/283），但路由（mod.rs:773-784）只有 create-session / post-message / get-command / get-tool-catalog 四个端点，**没有"确认后执行"入口**。人确认了无处执行。
3. **执行结果不核实**：`execute_management_tool` 返 `Ok` 即标 `succeeded`（management.rs:228-237），不核实 response 真实内容；assistant_text 直接回放 `plan.summary`（管理者预先看到的"打算做X"），等于拿"计划"当"结果"报喜。MCP 工具尤其危险（RPC 成功≠业务成功，如"账号离线"）。

## 2. 核心循环（管理侧"提议→确认→执行"）

```
管理者在指挥中心打字
  → build_management_plan：LLM 解析意图 → plan{intent, risk_level, requires_confirmation, tool_calls[], summary}
  → 风险分级裁定走向（§4，代码裁定非 LLM）：
     ┌─ readonly（只读查询）        → 直接执行 → 基于真实结果直答
     ├─ low（可逆低风险写）          → 直接执行 → 核实结果后告知
     └─ dangerous（高风险/宽影响）   → 标 pending_confirmation, take(0) 不执行
                                      → 汇报"将执行：具体改动清单 + 影响范围"
                                      → 管理者确认【新增 confirm 端点】
                                      → 执行 → 核实结果（§3）→ 如实汇报 → 全程审计
```

与客户侧 principal escalation **同构**：pending → 人确认 → resolve → 执行。区别仅：确认入口从"领导发微信自然语言"换成"管理者 REST 结构化确认"；执行从"AI 口吻转述给客户"换成"对项目执行改动"。客户侧特有的微信回流 LLM 解读、relay 转述出站安全门，管理侧不需要。

## 3. 执行结果核实 + 如实汇报（核心，防"执行了不知道成没成"）

**原则：agent 报告的成功/失败必须基于核实后的真实结果，绝不拿"计划"或"调用返回 Ok"当"业务成功"。**

三层核实：

### 3.1 工具级结果断言（outcome assertion，确定性规则非 LLM）
每个工具静态声明"怎么算真成功"：
- `send_contact_message` → MCP `response.success==true` 且有 `msgId`，否则 `failed`（堵"账号离线 RPC 仍返 Ok"）。
- `update_*` / patch 类 → `matched/modified ≥ 1`，`matched:0` 判"未命中、实际没改动"。
- `publish_domain_profile` → 区分 `published` 与 `pendingActivation:true`，如实回报真实状态而非笼统"成功"。
调用返 `Ok` 但断言不过 → status 标 `failed` 或 `executed_unverified`，**不报成功**。

### 3.2 汇报基于真实结果，不基于 plan.summary
确认执行后的 assistant_text **由真实 tool 执行结果生成**（成功了哪几个、哪个失败为什么、改动的真实数字），不回放 LLM 预写的 summary。严格区分"我打算做"与"实际做成了什么"。

### 3.3 不确定就说不确定
核实不到结果的（response 体无法判定），如实标 `executed_unverified` 并告知"已执行，但无法确认结果，请核对"，绝不假报成功（"诚实优于好看"，与 reaction/grounding 一脉相承）。

## 4. 工具集扩展 + 风险分级

### 4.1 工具集（按 5 类操作面扩，包装已有 REST 端点，不写新执行逻辑）

| 类 | 工具（新增除注明已有） | 风险档 | 包装的端点/函数 |
| --- | --- | --- | --- |
| 观测查询 | query_runs / query_metrics / query_health / query_inbox | readonly | GET /agent-runs、/agent-outcome-metrics、/contacts/:id/operation-health、/admin/ask-human/inbox |
| 运营态 | （已有）enable/disable/follow-up/update_profile/search/import/send | low（send 为 dangerous） | contacts.rs 现有端点 |
| 配置 | update_operation_domain / update_runtime_params / set_assist_mode | low→dangerous 视范围 | PUT /operation-domains/:domain 等 |
| 策略 | publish/activate domain_profile / prompt_template / soul / playbook、taxonomy approve | dangerous | domain_profiles.rs / prompt_templates.rs / souls.rs / playbooks.rs / admin_taxonomies.rs |
| 知识 | verify / reject / archive chunk、import-apply | dangerous | knowledge/verify.rs 等 |

### 4.2 风险档静态声明、代码裁定（不让 LLM 现场判风险）
每个工具在 catalog 定义里静态声明 `risk: readonly|low|dangerous`。`build_management_plan` 的 LLM 只负责"选哪个工具 + 填参数"；`requires_confirmation = plan.tool_calls.iter().any(|t| risk_of(t)==dangerous)` **由代码裁定**。LLM 选错工具最多选错，但"高风险必确认"由代码兜底，LLM 绕不过。符合"语义判断交 LLM、安全裁定交代码"的一贯做法。

### 4.3 两条红线在工具层硬约束
- 知识类**没有 auto-verify 工具**，只有 verify（人确认动作）——"AI 永不自动 verify"在工具集层堵死。
- 发客户消息 / 改全局 prompt / 切 provider 一律 dangerous，绝不 agent 直接执行。

## 5. 数据结构 + 端点 + 前端 + 测试

### 5.1 数据结构（最小增量，复用现有集合）
- `AgentToolCall.status` 闭集扩展：`running/dry_run/succeeded/failed` + 新增 `executed_unverified`。DB 写入点校验闭集（项目惯例：未知状态拒写）。
- `AgentCommandRun.status`：现有 `running/pending_confirmation/succeeded/failed/dry_run` 够用。
- 工具定义加静态声明：`risk` + `outcome_assertion`，集中在 catalog 定义处，不散落。
- 不新建集合。

### 5.2 新增端点（闭合循环）
- `POST /management-agent/commands/:id/confirm`：取出 pending command 暂存的 plan.tool_calls，执行 + 核实（§3）+ 审计。同构 principal `resolve` 乐观锁——仅 `status==pending_confirmation` 可确认（`find_one_and_update` 条件更新），二次点击返回已处理即幂等。IDOR：workspace_id 约束。
- `POST /management-agent/commands/:id/reject`（可选）：管理者否决待确认计划 → 标 canceled。

### 5.3 前端（command-center 做厚，遵守现有设计系统 docs/frontend-design-system.md）
- 对话流：管理者消息 → agent plan 预览卡（dangerous 时显示"将执行：改动清单 + 影响范围"）→ 确认/否决按钮 → 执行后每个 tool 显真实终态（✅成功 / ❌失败原因 / ⚠️已执行待核实）。
- 复用 ask-human 收件箱已建的 ReviewQueue/确认原语，不重造。

### 5.4 测试边界
- 纯函数确定性单测：风险档裁定（plan→requires_confirmation）、outcome_assertion（各类 response→成功/失败/unverified）。
- 状态机测试：pending_confirmation→confirm 乐观锁、二次确认幂等、reject。
- 不接真 LLM/MCP 测意图解析质量（留 CI nightly 真模型套件）。
- 守基线 lib ≥350/0 不回归。

## 6. 范围边界（YAGNI）

**只做**：把已有 REST 端点包成工具接入对话 + 闭合确认执行循环 + 执行结果核实。

**明确不做**（留后续或本就不该对话改）：
- 对话改状态机转移算法 / evolution 核心逻辑（只能改代码的不进对话）。
- 批量 / 定时管理操作。
- 客户侧 principal escalation 的任何改动（另一条已跑通的链，不碰）。
- 第一期工具集只接已存在的 REST 端点，不为对话新建执行端点。

## 7. 红线与安全

- **不碰"全 AI 自治"客户侧红线**：本设计是管理侧操控入口，与客户对话链解耦。
- **AI 永不自动 verify**：知识工具集无 auto-verify，verify 是人确认动作。
- **高风险必人确认**：dangerous 档代码兜底，LLM 绕不过；发客户消息/改全局 prompt/切 provider 强制确认。
- **诚实汇报**：执行结果核实后如实回报，不确定标 unverified，绝不假报成功。
- **审计**：AgentCommandRun/ToolCall 全程留痕，复用现有。
- **不引入 no-human-takeover 禁用词**：管理对话入口用 AI 内部口径，"请示/确认/执行"是管理者对 agent，不是"人工接管"客户对话。
