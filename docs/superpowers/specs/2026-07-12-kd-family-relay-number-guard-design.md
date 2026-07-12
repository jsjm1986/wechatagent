# 批D relay 数字护栏修复设计：删除错误的字符级数字 backstop，忠实度交还 LLM/Review（KD-01 + KD-03）

- 日期：2026-07-12
- 分支：`fix/kd-relay-number-guard`（基于最新 origin/main）
- 来源：深度审查批D relay 授权外数字护栏家族（KD-01 + KD-03；台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`）
- 优先级：P1
- 严重度：均 Medium

## 一句话结论（用户裁定）

`relay_introduces_unauthorized_number` 这个**字符级数字护栏**是一个**威胁模型错误**的代码 backstop：判断"relay 转述是否忠于领导授权"本质是**上下文依赖的语义问题**，用确定性代码逐字提数字做白名单差比对是**范畴错误**——它既漏真幻觉（中文"八折"提取为空），又误杀正确转述（"24小时""第2个问题"等无害数字、"两成=8折"等价折扣）。**删除它在 gateway 的 fail-closed 用法**，relay 忠实度交还给本就理解上下文的 LLM（生成侧 prompt）+ 已在链路里的独立 Review Agent。KD-01（中文盲区）与 KD-03（误杀致裁决黑洞）由此**同源根治**。

## 问题（KD-01/03 根因，已主控逐条亲验最新 main 成立）

### 现有护栏机制（亲验）

`relay_introduces_unauthorized_number`（`src/agent/escalation/logic.rs:256`）内部靠 `extract_number_tokens`（:220，仅 `ch.is_ascii_digit()`）提取阿拉伯数字 token，把授权 substance 的数字集当白名单，转述文本出现白名单外任一数字即返 true。gateway.rs:2487-2491 据此 `outbox_eligible=false` fail-closed（不发转述）。

### KD-01：字符级护栏双向失效（CONFIRMED）

- **绕过**：领导授权"9折"（阿拉伯），LLM 转述"打八折"（中文）→ `extract_number_tokens("打八折")=∅` → 恒 false → 放行编造折扣。
- **误杀**：授权"九折"（中文），转述"9折"（阿拉伯）→ "9"∉空授权集 → 判授权外 → fail-closed 拦正确转述。
- 根因：中文数字/大写金额既不进白名单也不进被检文本，白名单与被检文本不在同一数字空间。

### KD-03：误杀 → 裁决黑洞（PLAUSIBLE→根因 CONFIRMED）

- `relay_introduces_unauthorized_number` 误杀（KD-01 误杀向量，或转述夹"24小时""第2个问题"等**非授权语义的普通数字**——`extract_number_tokens` 抓一切数字串，无"数量事实 vs 序数/时间"语义区分）→ `outbox_eligible=false` 不发。
- 而 `relay_principal_decision_to_customer`（gateway.rs:768-776）在 gateway 返回后**无条件** `clear_awaiting_principal_state`。
- 结果：领导已裁决（如 approved 9折）因转述夹个"24小时"被误拦 → 客户**永远收不到裁决** + awaiting 也清了 = 裁决黑洞。

### 更深的认知（用户点破）

数字护栏想模仿 grounding 硬闸那种"确定性兜底"，但 grounding 能确定性判定是因为"有没有 verified chunk"是**客观集合运算**；而"转述是否忠于授权"是**语义判断**（LLM 理解"两成=8折"等价、理解"24小时"不是折扣承诺）——这正是同样手法在此必然既漏又误杀的根源。KD-01 与 KD-03 是**同一错误模型的两个症状**。

### 关键亲验：Review Agent 本就具备忠实度判断的全部上下文

- relay 转述走**同一个 gateway**（`relay_principal_decision_to_customer` → `run_user_operation_gateway`），故天然过 decision → 独立 Review Agent → send 全链路。
- `review_decision`（review/mod.rs:286）第 3 参 `inbound` 在 relay 场景 = 合成载荷消息；review user prompt（:441-474）经 `inbound_prompt_content(content, is_synthetic_relay=true)`（prompt_isolation.rs:54）**保留全部内容**（含 `substance=同意8折`/`constraints=…`，只包裹隔离、不剥内部字段），:475 填拟发转述 `reply_text`。
- **即：reviewer 同时看到「领导授权 substance」+「AI 拟发转述」，具备做忠实度语义判断的全部上下文。** reviewer rubric 已有"禁编造价格/承诺 → 抬 factRisk / 降 productAccuracy"通用维度。
- 台账 findings :836 原话已定性：「relay 主防线是 prompt「AI 口吻重组」，此为**代码 backstop**」——数字护栏从来不是主防线，只是（错误的）补充。

## 设计

### 唯一改动：删除 gateway 的数字护栏 fail-closed 用法

`src/agent/gateway.rs:2473-2520` 的 relay 出站守卫块里，`leaks_payload`（载荷泄漏）与 `unauthorized_number`（数字护栏）**耦合在同一 if**。外科式改动，**只摘数字护栏、保留载荷泄漏守卫**：

- 删 `unauthorized_number` 变量（:2487-2490，对 `relay_introduces_unauthorized_number` 的调用）。
- `if leaks_payload || unauthorized_number` → `if leaks_payload`。
- `warn_reason/event_reason` 的三元分支（因只剩 leaks 一种情形）简化为固定的"载荷泄漏"文案。
- 保留 `relay_output_leaks_internal_payload` 的 fail-closed + event + warn 完整不动。
- 注释同步更新：删掉"绝不编造领导授权之外的数量事实"那段（该职责移交 prompt + review）。

### 为什么载荷泄漏守卫保留（威胁模型正确）

`relay_output_leaks_internal_payload`（logic.rs:211）检测 `__PRINCIPAL_RELAY__`/`verdict=`/`substance=`/`constraints=` 这几个**固定内部载荷标记**是否出现在拟发文本——这是**确定性字符串存在性检测**（标记要么在要么不在，非语义判断），威胁模型正确：这些标记绝不该出现在给客户的文本里，出现即说明转述严重异常，fail-closed 拦截正确。保留。

### 忠实度由谁保障（删护栏后的正防线）

- **生成侧**：relay 转述 prompt（prompts.rs:1367-1375）已约束"substance 是转述的唯一事实源""绝不透传内部字段"。LLM 理解上下文，本就能忠实转述、不编造授权外折扣。
- **审查侧**：独立 Review Agent 已看到 substance + 拟发转述（上文亲验），rubric 已有禁编造承诺/factRisk 维度，做语义级忠实度把关（能识别"两成=8折"等价、也能抓真编造）。
- 这是设计本就存在的正防线（prompt + review），删掉错误的字符 backstop 是**回归设计本意**，不是拆掉正防线。

### 为什么不动 prompt / 不加 review 维度（YAGNI）

- prompt 与 review 的既有约束已覆盖 relay 忠实度（substance 唯一事实源 + 禁编造承诺）。额外增补属过度设计——本修复的本质是**移除一个错误的多余闸**，不是新增闸。
- 若实测发现 LLM 确实会编造授权外折扣且 review 未拦（本修复未观测到），再单独立项加强 review relay 维度，届时有数据支撑，不在本次臆测补强。

### KD-03 裁决黑洞：删护栏即根治（残留窗口分析）

relay 转述"未真发出去"的可能来源，主控逐一亲验：
1. **数字护栏 fail-closed**（gateway.rs:2491）——**本次删除**。这是 KD-03 现实中唯一的误杀触发源（数字误杀 / 序数时间数字误杀）。
2. **并发去抖中止**（gateway.rs:2531-2542）——只在 `should_abort_send=Some(guard)` 时触发；relay 调用 `run_user_operation_gateway` 传的是 `None`（gateway.rs:768-774 亲验）→ **去抖对 relay 不生效**，不构成 relay 未发来源。
3. **载荷泄漏 fail-closed**（保留）——确定性字符串检测，正常 LLM 转述绝不命中；命中说明转述严重异常、本就该拦，属**极罕见的正确拦截**。

结论：删除数字护栏后，KD-03 的裁决黑洞在现实中被根治（数字误杀是唯一现实触发源）。载荷泄漏那个理论残留窗口（命中 → 不发 → 无条件清 awaiting）极罕见且是"正确拦截 + 该议题确实不可安全转述"，**不值得为它引入 `sent` 回传的接口改动**（`run_user_operation_gateway` 返回类型 + 4 调用点 + relay 补偿分支），YAGNI。作为**知情记录的接受窄窗口**保留。

## 不改动的（严格限定范围）

- **`holding_reply.rs:26` 对 `relay_introduces_unauthorized_number` 的调用不动**（亲验：语义正确且无黑洞）。holding_reply 用它核对**过渡话术**是否含授权外数字，命中后果是**回落 scene 硬编码兜底文案**（generate_holding_reply :62-65"客户永不被晾死"），**不是 fail-closed 不发**。且 ExpiredAuthorization 场景 prompt 本就要求"绝不编造折扣/金额/百分比"，护栏在此等价于"过渡话术含数字就换兜底"，威胁模型正确。
- **`logic.rs` 的 `relay_introduces_unauthorized_number` / `extract_number_tokens` / `normalize_number_token` 三函数及其单测不删**（holding_reply 仍是合法调用者，删则破坏 holding_reply + 编译）。仅 gateway 不再调 `relay_introduces_unauthorized_number`。
- `relay_output_leaks_internal_payload`（载荷泄漏守卫）不动。
- relay task 生命周期、`clear_awaiting_principal_state`、`run_user_operation_gateway` 返回类型、review prompt、relay 转述 prompt——全不动。
- KD-02（"客户永不知道有领导"词表守卫）不在本次范围（独立 finding）。

## 测试策略

- **gateway 删除点**：现有针对"relay 转述含授权外数字 → 不入队"的集成测/单测（若有）须相应**改为断言相反行为**——含授权外数字的 relay 转述**仍入队发送**（护栏不再拦）。载荷泄漏 → 仍 fail-closed 的断言保留。定位：搜 `unauthorized_number` / `授权外数字` 相关测试。
- **holding_reply 不受影响**：`holding_reply_text_is_safe` 的 ExpiredAuthorization 数字守卫单测（logic.rs 附近）保持绿——证明该调用点未被波及。
- **logic.rs 函数单测**：`relay_introduces_unauthorized_number` 等三函数单测保留（函数仍存活、仍被 holding_reply 用），继续绿。
- baseline `cargo test --lib` ≥ 350 passed / 0 failed 不回退。
- no-human-takeover lint：gateway 删除/改注释的新增行用"转述/裁决/授权/载荷"措辞，无禁词。

## 验证

- `cargo test --lib`（含上述改断言的相关单测；baseline 不回退）。
- 相关集成测（若涉及 relay 出站，`cargo test --test <name> --no-run` 本地编译，执行留 CI Docker）。
- no-human-takeover lint clean。

## 交付

- 单一 src 改动：`src/agent/gateway.rs`（删数字护栏 fail-closed 用法 + 简化 leaks 文案 + 注释同步）。
- 测试：改相关断言（含授权外数字的 relay 仍发）+ 保留载荷泄漏/holding_reply 断言。
- 独立修复 PR（基于最新 main）。台账 KD-01 标 Closed（护栏删除使字符盲区不再有害）、KD-03 标 Closed（删除唯一现实误杀源根治裁决黑洞；载荷泄漏残留窗口作为接受的窄窗口知情记录）。

## 与前几轮设计演进的说明（诚实记录）

本设计经历了从"复杂"到"极简"的收敛：初版曾设想①中文折扣归一化②KD-03 补偿话术③sent 回传接口④review 增补维度（3-4 支柱）。经用户连续点拨"AI 用上下文理解数据本来就容易，不该这么复杂"+ 逐层亲验（reviewer 已看到 substance+转述、去抖对 relay 不生效、holding_reply 调用语义正确），收敛为**单一删除动作**。这符合项目 agent-first 哲学与"转人工红线从词表 panic 换纯 LLM 硬门"（2026-06-20 用户裁定）的一贯方向：语义判断交还 LLM，不用字符匹配 backstop 假装能做语义。
