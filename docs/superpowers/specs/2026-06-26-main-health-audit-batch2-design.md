# main 健康度审查 batch2 修复设计

> 来源审查:`.git/sdd/main-health-cross-audit-2026-06-26.md`(7 维度 workflow + 独立 verifier + 主线程深核三轮交叉验证净清单)。batch1(SEC-1/EVO-2/KNOW-1/FE-1)已合入 main(PR#41)。本设计覆盖**剩余 6 条 finding**。
> 日期:2026-06-26。每条方案均经**改动点完整业务逻辑深度核实**(读 CommitmentRepr/CommitmentEntry 定义、planner emit 链路、CONC-1 写点门控结构),核实推翻了两处原始方案(见下文偏差说明)。

## 背景与目标

main 健康度净清单 10 条 CONFIRMED,batch1 修了 4 条安全/契约级。本 batch 修剩余 6 条:写侧并发硬化(CONC-1/2/3)+ 动作闸复检(GATE-1)+ 告警口径(KNOW-2)+ 死字段接线(EVO-3)。均为 Low/Minor/Medium,无 Critical/安全级。

## 深度核实推翻的两处原始方案(关键)

**偏差 1 — CONC-2 不用 pipeline 去重**:`CommitmentRepr` 是 `#[serde(untagged)]` enum(models.rs:3954),落库 `commitments` 数组**异构**(`Plain` → 裸字符串;`Structured` → 子文档 `{id,text,dueAt,createdAt}`)。在异构数组上用 aggregation `$filter` 按 text 去重需 `$type`+`$cond` 判类型,表达式复杂脆弱且无法纯函数单测(违项目测试纪律)。且经读 planner:`pick_commitment_emit_target`(planner/mod.rs:555)每 contact **只选最紧迫一条** emit,emit 前 `commitment_recently_emitted`(:610)按 `commitmentId` 去重 → 并发写重复 text **几乎无害**(各有独立 UUID,planner 单选+id 幂等)。真实危害仅"并发丢失最紧迫且后续不复述那条" → 比净清单 Medium 窄。**故 CONC-2 改为简单 `$push`+`$slice:-8` 治丢失,去重留应用层(接受并发重复)**。

**偏差 2 — CONC-1 不套整个 update**:`apply_operating_memory_update` 末尾的 `update_one`(gateway.rs:4192)其 `set_doc` 混两类字段——门控块(`:4177 if !memory_card_has_signal`)内写 memory_card + memory_card_version,门控外恒写 updated_at + operating_memory_update/context_pack(**不 bump version**)。给整个 update 套版本谓词会误拦门控外的 context_pack 写 → lost-race 永久重试/丢写。**故 CONC-1 只把 memory_card 那几个字段拆出单独走 occ_memory_filter,门控外的写保持原样**。

## 全局约束(继承,每个 Task 隐含)

- baseline:`cargo test --lib` ≥ 350 passed/0 failed;4 PBT 文件累计 ≥ 33/0。
- 禁词 lint(no-human-takeover):`git diff` 新增行(src/agent,src/routes,src/evolution,frontend/src,含注释)零命中 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。
- `cargo check --tests -Dwarnings` exit 0(集成 binary 全编译)。
- 既成事实纪律:回复/业务动作成功后的 DB 写失败 → warn 不返 Err(防重发)。
- 精确 git add(不用 -A/.);本地仅 `cargo test --lib` + 单 PBT(磁盘紧),集成套件留 CI。
- 生产 Mongo 8(部署服务器 117),`$push`/`$slice` 等聚合算子可用。

## 逐条设计

### CONC-3 — load_or_create insert 捕获 E11000(最痛,优先)

**file**:`src/agent/memory.rs:798-816`

**问题**:`load_or_create_operating_memory` create 分支裸 `insert_one(&memory).await?`。首次触达时 webhook gateway(在发送**之前**,gateway.rs:880)与后台任务并发 find_one→None→都 insert,输者 E11000 经 `?` 透传成 AppError → **回复客户之前整轮 run 失败**(不受既成事实纪律保护——后者只覆盖发送成功后)。入站消息已在 webhooks.rs 持久化,微信重投命中去重早返回不重生成回复 → 客户可能真丢一条回复。仓内 6 处已有 dup-key 幂等(outbox/tasks/webhooks/behavior_signals/escalation::logic/admin_taxonomies),唯独此处没对齐。

**方案**:insert 返回 `Err` 时,用现成 `crate::agent::escalation::logic::is_duplicate_key_error`(pub(crate))判定;命中 dup-key 则不透传,落到下方**已存在的** find_one 重读分支(:803-815)返回赢家文档。其余错误仍 `?` 透传。

**测试**:`is_duplicate_key_error` 已有调用方测试覆盖判定逻辑。本条新增逻辑是"insert Err 且 dup-key → 走 find_one";因 create 分支整体需要 DB,放 `#[ignore]` 集成测试断言并发双 insert 都成功返回同一文档(留 CI)。lib 侧不强加无法运行的并发测试。

### KNOW-2 — unverified-warning 计数补 status=active(零风险)

**file**:`src/agent/knowledge_router.rs:100-112`(total)、`:119-132`(verified)

**问题**:`maybe_emit_unverified_warning` 两处 `count_documents` 不带 status 过滤,而运行时注入口径 `load_operation_knowledge`(:50 `status="active"`、:70-71 `status="active" AND integrity_status="verified"`)。归档(status≠active)的已核验切片仍计入 verified>0 → :133 提前 return 抑制告警,但这些切片不被注入 → 运营得不到"有切片却全不可注入"告警。纯告警可观测口径偏差,不影响发送/安全。

**方案**:两处 count filter 各补 `"status": "active"`,对齐注入口径。

**测试**:count filter 是 DB 查询,放 `#[ignore]` 集成测试断言"归档已核验切片不计入 verified、触发告警"(留 CI)。

### CONC-2 — commitments $push 治并发丢失

**file**:`src/agent/gateway.rs:3710-3732`

**问题**:commitments 当前从 run 起始快照 clone→push→cap8→整体 `$set`(filter 仅 `_id`)。并发 writer(webhook task vs worker tick)各从陈旧快照 append 后互相覆盖丢累积项。commitments 驱动行为(planner emit follow_up),丢最紧迫且后续不复述的那条 = 漏一次跟进。

**方案**:commitments 从大 `set_doc` 拆出,单独一次 `update_one` 用 `$push: {commitments: {$each: [新entry], $slice: -8}}`(`$slice: -8` 保留最新 8 条,与原 `drain(0..drop)` 丢最旧语义一致)。仅在**应用层 already_present 判定为新 entry 时**才发这次额外 update(避免无谓往返)。去重保留应用层快照判定(`already_present`,与现有逻辑一致)——并发下可能写重复,接受此代价:planner `pick_commitment_emit_target` 单选 + `commitment_recently_emitted` 按 id 幂等,重复项最多占 cap8 槽位不会重复 emit。memory_summary 维持 last-write-wins(纯文本,后续轮次复述自愈,同 bayesian 旁路纪律)。

**接缝**:此改动作用于 contacts 集合的 update(原 :3828 那次大 `$set` 移除 commitments key,新增一次 `$push` update)。与 CONC-1 改的是不同集合(operating_memories),无冲突。

**测试**:`$push`+`$slice` 文档构造是确定性的,加 lib 纯函数测断言构造的 update doc 形态正确($each 含新 entry、$slice==-8);并发行为留 CI。

### CONC-1 — memory_card 写拆出走 OCC

**file**:`src/agent/gateway.rs:4176-4204`

**问题**:`apply_operating_memory_update` 末尾 `update_one`(:4192)filter 仅 (workspace_id, account_id, contact_wxid) 无 version 谓词,绕过 memory.rs 的 occ_memory_filter CAS。门控在 :4177(仅卡片无信号、首次触达时写 memory_card)。

**方案**:把门控块(:4177-4188)内写的 memory_card / memory_card_version / memory_card_updated_at 三字段从共享 `set_doc` 拆出。当门控触发(写 memory_card)时,这三字段单独用 `occ_memory_filter(ws, acct, wxid, memory.memory_card_version)`(现成 pub(crate),memory.rs:632)做一次 `update_one`,判 `modified_count==0` 走 lost-race 分支(跳过,不报错——既成事实)。门控外的 updated_at + operating_memory_update/context_pack 仍走原 (三键 filter) update,**不动**(它们不 bump version,套版本谓词会误拦)。

**测试**:occ_memory_filter 已有测试。本条是"门控触发时用版本 filter 写 memory_card",放 `#[ignore]` 集成测试断言 lost-race 跳过(留 CI)。

### GATE-1 — revision 后复检动作闸

**file**:`src/agent/gateway.rs:1398-1435`(动作闸)、`:1686` 后(second_passed)

**问题**:`enforce_state_action_policy`(全仓仅 :1407 一处调用)+ taxonomy 软闸包在 `:1398 if matches!(finalize_status, Approved)` 块,位于 revision 块(:1590)之前。revision 后 final_decision 整条替换(:1644),operation_state 可能迁入禁止 reply 的态,只重跑 finalize_review_for_send(:1666 安全闸),**不重跑动作闸** → 绕过。核心安全闸有二次复检不受影响,故 Minor;只补动作闸(taxonomy 软闸本就有意非阻断,不补)。

**方案**:把动作闸逻辑(:1398-1435 的 load policy + classify_decision_action + enforce + held 处理)抽成一个可复用单元(局部 async 闭包或私有 async fn,接收 &mut review / &mut final_decision / &mut finalize_status)。原位置(初次 finalize Approved 后)调一次;在二次 finalize 的 `second_passed`(:1686)判定为 Approved 后,对改写后的 `final_decision.operation_state` 再调一次。命中 forbidden 时同样置 held_by_ai_policy + should_reply=false + 落审计事件。

**测试**:抽出的单元若为纯函数(policy 已加载、判定逻辑)可 lib 测;若含 DB(load policy)放 `#[ignore]` 集成测断言"revision 迁入禁态→held"(留 CI)。

### EVO-3 — threshold_auto_release_enabled 轻量接线

**file**:`src/evolution/auto_release.rs:42`、`src/routes/evolution.rs`(UpdateRuntimeFlagRequest:548 / PUT $set:602-608 / runtime_flag_json:690-697)

**问题**:`EvolutionRuntimeFlag.threshold_auto_release_enabled`(models.rs:1220)是 per-workspace 字段但零生产消费。`auto_release_eligible_thresholds`(:42)只读 env `evolution_auto_release_enabled` + 写死 default_workspace_id;PUT 不接受/不写该字段;GET 不序列化。

**范围界定(已与用户确认轻量版)**:evolution worker(run_one_tick)目前全程只跑 default workspace、不遍历——真多租户灰度需改 worker 核心循环,是独立工程,**不在本 batch**。本条只让字段不死、端到端通(针对 default workspace)。

**方案**:
1. `auto_release_eligible_thresholds`(:42):env `evolution_auto_release_enabled` 保留为**全局总闸**(关则整段 return,镜像 `is_evolution_enabled_for` 双闸顺序);总闸开时,再 `load_runtime_flag(state, default_workspace_id)` 读 `flag.threshold_auto_release_enabled`,false 则 return Ok(0)。即"env 总闸 AND per-workspace 子闸"。
2. PUT `UpdateRuntimeFlagRequest`(:548)加 `threshold_auto_release_enabled: Option<bool>` 字段;`$set`(:602)在 Some 时写入(None 不动,保持 upsert 既有值)。
3. GET `runtime_flag_json`(:690)补输出 `thresholdAutoReleaseEnabled` 键。

**测试**:auto_release 双闸顺序加纯函数/集成测(总闸关→0、子闸关→0、双开→进入扫描);PUT/GET 字段往返放路由集成测或 `#[ignore]`(留 CI)。

## Task 划分与执行

7 个 Task,subagent-driven-development 逐任务执行(spec+质量双判 review + fix loop + whole-branch 终审):

1. CONC-3(最痛、改动小、helper 现成)
2. KNOW-2(零风险、两行)
3. CONC-2(commitments $push)
4. CONC-1(memory_card 拆出 OCC)
5. GATE-1(动作闸抽闭包 + 二次复检)
6. EVO-3(轻量接线)
7. 全量验证 + baseline 门

CONC-1(operating_memories)与 CONC-2(contacts)改不同集合无冲突;GATE-1 改 revision 块独立。Task 间接缝小,可顺序独立交付。

## 非目标(本 batch 明确不做)

- evolution worker 多租户遍历(EVO-3 真灰度的上游)——独立工程。
- memory_summary 的并发 OCC(接受 last-write-wins)。
- commitments 应用层去重的原子化(接受并发重复,planner 单选+id 幂等兜底)。
- taxonomy 软闸 revision 后复检(本就有意非阻断)。
