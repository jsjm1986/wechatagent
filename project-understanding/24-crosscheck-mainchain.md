# 主链路交叉验证（核证日期 2026-08-13）

> 对象：`project-understanding/` 主链路五份记录——01（gateway）、03（webhooks/tasks）、04（decision/review/guards）、05（outbox/escalation/send）、06（memory/reaction/runtime）。
> 方法：①七组跨记录接口断言逐项对照，矛盾处**亲读源码裁决**（禁止用一份记录裁决另一份）；②每份记录随机抽 15 个 file:line 锚点（覆盖不同章节）亲读核对；③抽验各记录"覆盖自证"区段是否虚报。本文所有 file:line 均为当日（2026-08-13）工作区实测，全部经 Read/Grep 当场确认。
>
> 前置核对：五份记录声称的文件行数与 `wc -l` 实测——04（15 文件 23814 行）、05（13 文件 11672 行）、06（16 文件 15597 行）逐文件全部一致；01（gateway.rs 9152）一致；03 声称 webhooks.rs 3089 / tasks.rs 1972，实测 **3088 / 1971**（各差 1；经尾部锚点核验函数级行号无系统性偏移，详见 §2.2）。

---

## 1. 接口一致性审计表

**总判定：7 组接口中 6 组一致，1 组含一处实质矛盾（G，已裁决：01 对、05 错）。**

### A. webhook 物化任务的 kind/水位协议（01↔03）——**一致**

| 断言点 | 01 说 | 03 说 | 判定与源码证据 |
|---|---|---|---|
| 任务 kind | gateway 按 `task.kind == crate::webhooks::DURABLE_INBOUND_REPLY_KIND` 分流到 durable 处理（gateway.rs:190） | `DURABLE_INBOUND_REPLY_KIND = "inbound_reply"`（webhooks.rs:26） | **一致**。亲证 webhooks.rs:26 值为 `"inbound_reply"`，gateway.rs:190-192 引用同一常量分流 |
| content 协议 | task.content.trim() 必须是合法 ObjectId——"task 快照的 content 存的就是持久化的入站消息 `_id`"（gateway.rs:237-239） | 任务只存消息 `_id` 的 hex：`content: message_id.to_hex()`（webhooks.rs:120） | **一致**。两侧亲证：写侧 webhooks.rs:120，读侧 gateway.rs:237-239 `ObjectId::parse_str(task.content.trim())` |
| 水位字段 | `load_recent_messages` 合并 durable 任务的 `covered_through_*`（**不含边界**）或回落 `obligation_started_*`（**含边界**）之后的全部 inbound（gateway.rs:7405-7421） | 行上有 `latest_inbound_*`（上界）、`obligation_started_*`（下界，webhooks.rs:140-141）、`covered_through_*`（交付水位，`advance_covered_watermark` 单调推进，webhooks.rs:374-411） | **一致互补**。亲证 gateway.rs:7415-7416 covered 用 `inclusive=false`（$gt）、7417-7421 obligation 用 `inclusive=true`（$gte），`uncovered_inbound_watermark_filter`（7352-7364）按参数生成 `$gt/$gte`；03 的三组水位字段写点全部命中 |
| fence 协议 | "后来的入站会刷新同一 task 行、清旧 claim token"（gateway.rs:221-225 注释） | refresh 分支 `$unset` claim_token/outbox_decision_id 等使旧代 fence（webhooks.rs:177-204）；filter 不含 status 条件、终态可复活 | **一致**。亲证 refresh 分支（webhooks.rs:178-204）$unset 含 claim_token/outbox_decision_id，filter 仅 `_id + manual_reply_run_id $exists:false + newer` |
| 结算协议 | dispatcher finalize 按冻结水位调 `settle_ai_reply_obligation`（01 §2 未详述，05 补全） | 先无条件 advance 再精确 latest-CAS（latest_inbound 恰等 + 无 manual 占有）→ sent/`agent_reply_delivered`（webhooks.rs:417-453） | **一致**。亲证 webhooks.rs:423（advance）+ 429-451（精确 CAS filter） |

### B. gateway 调 decision/review 的参数与返回契约（01↔04）——**一致**

| 断言点 | 01 说 | 04 说 | 判定与源码证据 |
|---|---|---|---|
| decide 契约 | rewrite 调 `decide_reply_with_promote(Some(&review.rewrite_instruction), PromptTier::Full)`；Lean 首程与知识路由并行 | 签名 17 参数，返回 `(AgentDecision, Vec<String> promote_risks)`（decision.rs:589-608） | **一致**。亲证签名恰 17 参数（state…run_snapshot），返回 `AppResult<(AgentDecision, Vec<String>)>`；gateway 调用点 grep：2542（Lean 并行）/2576（Full 串行）/2730/2824（升档重生成）/3089（rewrite）/3363（revision），与两侧描述吻合 |
| review+ClaimGate 并行 | `review_and_evaluate_claim_gate`（gateway.rs:598-661）内 `tokio::join!`；**4 个调用点 848/3002/3128/3398** | 判定树引用 gateway.rs:626/645（即函数体内 `review_decision`/`evaluate_independent_claim_gate` 两条调用语句） | **一致（引用粒度不同）**。grep 亲证：调用点恰好 4 个（848/3002/3128/3398）；626/645 恰是函数体内两个 join 分支起始行。两记录相互兼容且均与源码一致 |
| review 错误语义 | review 的错误 `?` 传播，claim_gate 直接返回评估对象（非 Result） | `evaluate_independent_claim_gate` 不改 review、不产生授权（mod.rs:1452-1499） | **一致**。亲证 gateway.rs:658 `Ok((review?, claim_gate))` |
| 是否评审 | budget 超额→local_decision_review；`should_run_review`→并行评审；否则 local | `should_run_review` **恒等于** `decision.should_reply`（mod.rs:3277-3285） | **一致**。亲证 3281-3284 函数体只返回 `decision.should_reply`（planner/runtime 参数带 `_` 前缀不消费） |
| rewrite 准入 | `should_run_targeted_rewrite` 只接 hallucination/grounding 硬闸；软闸失败（needs_revision=true）留给 finalize 后 revision | `should_run_targeted_rewrite = should_reply && !should_hold && !review_passed && !needs_revision`（mod.rs:3266-3275） | **一致（表述层次不同）**。亲证公式逐字命中；因 `route_dual_gate` 仅软闸失败置 needs_revision（gates.rs:237-277），01 的语义表述与 04 的公式等价 |
| review_mode | `effective_review_mode` + `apply_confidence_override`（<阈值→full） | force_full ∥ distrust ∥ planner.high ∥ knowledge_required → full；confidence（缺省 10）< 阈值（默认 4）→ full；planner light → light；否则 full（mod.rs:3236-3259） | **一致**。亲证 3242-3258 逐分支命中 |

### C. finalize 终态 → outbox 入队条件（04↔05 与 01）——**一致**

| 断言点 | 04 说 | 01 说 | 05 说 | 判定与源码证据 |
|---|---|---|---|---|
| 终态枚举 | GatewayStatusFinal 六变体：Approved / BlockedByRequiredField / BlockedByBudget / BlockedUnverifiedProductClaim / BlockedBySafetyGuard / Held(String)（gates.rs:460-477）；`gateway_status_str`（481-492）与 `final_review_status_str`（495-500）一一对应 | 事实卡引用 gates.rs:479-501 同一映射 | —（05 不直接消费该枚举） | **一致**。亲证六变体与两映射函数逐字命中；`revision_applied_approved` 由 gateway 改写、映射函数不参与（gates.rs:496-498 注释亲证） |
| 入队门 | Approved 才允许进 revision / outbox enqueue（gates.rs:456-458 注释） | `outbox_eligible = should_reply && reply_text 非空 && final_status ∈ {approved, revision_applied_approved}`（gateway.rs:4180-4182） | enqueue 是唯一业务入队入口（outbox.rs:221） | **一致**。亲证 gateway.rs:4179-4182 逐字命中；注释 4175-4176 同口径 |
| 分段幂等键 | — | 多段（total>1）时每段 `{segment_idempotency_base}#seg{idx}`，单段不加后缀（gateway.rs:4306-4313）；EnqueueRequest{decision_id=Some(review_id), max_attempts:3, source_kind=trigger.kind()} | `run_sequence_for` 从 source_event_id 的 `#segN` 后缀解析 N（≥0，解析失败→0）（outbox.rs:199-203）；名片 20000/媒体 10000 | **一致咬合**。亲证 4306-4313 `if total > 1` 条件、4314-4326 请求字段（4321 `trigger.kind()`）；outbox.rs:188-204 解析逻辑兼容单段无后缀（→0） |
| 幂等键构造 | — | IdempotentSkip 时 adopt / existing_outbox_covers_decision 判可交付集 | 非 synthetic 文本 key = `{source_event_id}:{contact_wxid}:{content_hash}`（**不含 run_id**，outbox.rs:279-283）→ v2 scoped（285-286）；媒体/名片一律 synthetic | **一致**。亲证 outbox.rs:262-286 分支与 key 形态；跨 run 共享 key 正是 01 描述的 IdempotentSkip 处理场景 |

### D. claim/fencing 协议（03↔05）——**一致**

| 断言点 | 03 说 | 05 说 | 判定与源码证据 |
|---|---|---|---|
| 授权提交序 | `authorize_task_outbox_if_owned`（tasks.rs:468-535）：数 outbox 行（0→拒）→ update_many 打 marker（matched≠total→拒）→ task CAS→outbox_enqueued（授权线性化点）→ notify 立即 + 1050ms 二次唤醒；"标记本身不是授权：dispatcher 还要求本 task ∈ outbox_enqueued\|sent"（475-479 注释） | dispatcher 侧 `classify_task_send_authorization`（dispatcher:422-452）：token/decision 失配→Stale；marker 与 token 不一致→Stale；running→Building；outbox_enqueued\|sent 且有 marker→Authorized、无 marker→Building | **一致（双向互证）**。两侧亲证逐行命中；03 描述的"marker≠授权"不变量与 05 的分类器要求（status+marker 双条件）严丝合缝 |
| Building 处理 | tasks.rs:533 `notify_outbox_work_after(1050ms)` 接住 1s defer | `defer_until_task_authorized`（dispatcher:563-602）：无损退回 pending + next_retry_at=now+1s（574），不加 attempt/reclaim | **一致**。亲证 dispatcher:574 恰为 +1_000ms、tasks.rs:533 恰为 1_050ms |
| Stale 处理 | 新入站 refresh 清 token → 旧 claim 全链拒绝 | `enforce_task_send_authorization`（604-626）：Stale → `cancel_entry("stale_task_claim: {reason}")` | **一致**。亲证 615-623 |
| claim 结构 | TaskClaim{token 不可伪造 + generation 单调}；claim CAS 注入 status ∈ {pending,retry,failed}、After、$inc attempt+generation、$unset 三项（tasks.rs:186-244） | 正确性只依赖 `_id+status+token(+generation)` CAS | **一致**。亲证 tasks.rs:190-232 全部命中（含 i64/i32 兼容缺省 1） |
| marker 修复 | —（03 未涉及） | 行缺 marker 且 task 已提交且 token/decision 匹配 → CAS 继承 marker（dispatcher:518-551） | **不冲突**（05 单侧补充，亲证成立） |

### E. reaction claim 与 gateway 并行 barrier（01↔06）——**一致**

| 断言点 | 01 说 | 06 说 | 判定与源码证据 |
|---|---|---|---|
| 并行模式 | `reaction_gateway_parallel_enabled` 为真 → `tokio::spawn` 跑 `record_user_reaction_with_outcome` 包成 `ParallelReactionTask::running`；为假 → 先串行跑完（gateway.rs:295-342） | `record_user_reaction_with_outcome` 返回 `ReactionOutcome{claimed, outcome_status, stop_requested}`（reaction.rs:30-35, 47-79） | **一致**。亲证 gateway.rs:299-342 两分支与 reaction.rs:30-35/47-51 契约互相咬合 |
| barrier 位置 | 非文本提前汇合 `barrier_stage="before_non_text_outbox"`（gateway.rs:2439-2457）；文本主汇合 `"after_first_reply"`（2617-2634），在任何 escalation/review/mutation/Outbox 之前 | —（06 不涉 gateway 内部时序） | **一致**。grep 亲证两个 stage 字符串恰在 2452/2629 |
| stop 语义 | `abort_on_reaction_stop`：stop → cancel task + `user_reaction_stop_requested` 事件 + run log + 终止 run；**分析失败仅 warn 不视为 stop**（gateway.rs:1672-1674） | 确定性 stop 在加载 prompt/config/claim **之前**短路（reaction.rs:52-58）；DB 故障仍返回 stop_requested=true（274-277） | **一致**。亲证 gateway.rs:1653-1726（1675-1681 仅 `outcome.stop_requested` 触发）与 reaction.rs:56-58/251-254（100 年 cooldown）/274-277 |
| reaction claim | reaction 拥有独立预算/task-local scope（gateway.rs:295-298 注释） | claim 原子锁：find_one_and_update sort created_at:-1，filter `status=sent + outcome_status ∈ [null,pending]`，$set analyzing+token、$inc generation；拿不到→default 不调 LLM（reaction.rs:368-400）；专属 RunBudget（67-78） | **一致**。两侧亲证命中 |

### F. 投影 snapshot/activate 时机（01↔06）——**一致**

| 断言点 | 01 说 | 06 说 | 判定与源码证据 |
|---|---|---|---|
| persist 时机 | decision_review 落库 + SR-034 claim 绑定之后、outbox 入队之前（§3.1 #9，gateway.rs:4056）；失败仅 warn + mark_preparation_failed，投递继续 | Gateway 在发送授权路径调用（post_decision.rs:140-214）；"客户投递永不等投影" | **一致**。grep 亲证 persist 调用点唯一在 gateway.rs:4056；post_decision.rs:153-171 读 baseline_profile_revision、191-207 置 prepared |
| activate 时机 | 两点：should_reply=false（no_reply 也激活，gateway.rs:4119）；文本授权 CAS 成功后（4709，用 `?`） | "Activate only after no-reply settlement or durable text-send authorization"（post_decision.rs:585 注释）；prepared→pending（586-601） | **一致**。grep 亲证 activate 调用点恰 2 个：4119/4709；post_decision.rs:585 注释逐字命中 |
| discard 路径 | stale claim（4052）/空文本（4112）/superseded（4274）/skipped_duplicate（4448）/partial failure（4517）/未入队（4568）/authorize CAS 失败（4639） | prepared→discarded + $unset payload（603-617），仅匹配 prepared | **一致**。grep 亲证 7 个 discard 调用点与 01 各步骤一一对应 |
| 冗余语义 | —（01 未涉及） | 疑点 4：`runnable_filter` 本身接受 prepared（review.status 已入三态时不经 activate 也 runnable）；discard 只匹配 prepared 有竞态窗 | **06 疑点亲证属实**：post_decision.rs:619-641 filter 确含 `post_decision_status ∈ [prepared,pending,retry]` |

### G. precheck 豁免矩阵与 escalation relay 豁免（01↔05）——**一处矛盾（已裁决）+ 其余一致**

| 断言点 | 01 说 | 05 说 | 判定与源码证据 |
|---|---|---|---|
| relay 判定 | `is_relay = escalation::is_principal_relay_trigger(trigger)`（gateway.rs:5259）；伪造哨兵不豁免（测试 8151-8175） | 唯一判据 `ConversationMessage.is_synthetic_relay` 来源标记，绝不按 content 前缀（H10，logic.rs:199-204） | **一致**。两侧亲证：logic.rs:199-204 `matches!(Inbound(m) if m.is_synthetic_relay)`；测试 8152-8175 伪造哨兵断言存在 |
| 豁免范围 | relay 豁免 `!is_relay` 块内全部频控门：cooldown/policy/rate_limited/daily_limit/expired/quiet_hours（gateway.rs:5268-5341）；not_managed 恒查（5251-5253）；context_changed 在块外但 relay 非 FollowUp 天然不进（5342-5357） | "豁免频控 precheck（rate_limited/cooldown/daily_limit）"（§3.3 + logic.rs:197-198 注释转述） | **一致**。亲证块边界 5268 `if !is_relay {` … 5341 `}`，context_changed 判定在 5342（块外）；01 的豁免矩阵精确，05 为概述级不冲突 |
| **relay 出站守卫** | **只有一道**：`relay_output_leaks_internal_payload`（检哨兵/verdict=/substance=/constraints=）；"**数字白名单 backstop 因威胁模型错误已删除 KD-01/03**"（gateway.rs:4184-4192 注释） | §3.3："relay……出站前有 relay_output_leaks_internal_payload+**数字白名单双守卫**，仍走 outbox" | **矛盾 → 裁决：01 对，05 错**。证据（全部亲验）：① gateway.rs:4188-4192 注释明确"不再用字符级数字白名单 backstop……已删除（KD-01/03）"；② relay 出站路径 gateway.rs:4193-4215 只调 `relay_output_leaks_internal_payload`，无数字白名单调用；③ grep 全库：`relay_introduces_unauthorized_number` 唯一生产调用点是 **holding_reply.rs:60**（安抚话术三关守卫之一），不在 relay 转述路径。**误导源**：logic.rs:255-258 的函数 doc 注释本身过时（仍称"网关据此不发该转述"），05 被其带偏；05 自己在 §2.8（holding_reply 三关含该函数）与 §2.7（函数本体）的描述均正确，仅 §3.3 的跨文件接线归属错误 |
| 精确对齐（旁证） | relay 泄漏守卫命中 → `outbox_eligible=false + delivery_block_status=Some("blocked_by_safety_guard")`（gateway.rs:4197-4198） | 泄漏守卫检 4 标记（logic.rs:214-219） | **一致**。亲证两侧 |

---

## 2. 锚点抽验结果

每份记录抽 15 个锚点（覆盖总地图/逐函数/跨机制/事实卡/疑点各章节），逐一亲读源码核对。**总计 75 项，通过 74 项，失配 1 项，通过率 98.7%。**

### 2.1 记录 01（gateway）——15/15 通过

| # | 锚点 | 记录断言（摘要） | 结果 |
|---|---|---|---|
| 1 | gateway.rs:100-109 | 时间承诺弱启发 8 词（明天/后天/下周/下个月/稍后/晚点/回头/马上） | ✓ 逐词命中 |
| 2 | gateway.rs:113-123 | existing_outbox_covers_decision 状态集 {pending,in_flight,sent,delivery_unknown}（:121） | ✓ |
| 3 | gateway.rs:125-143 | commitments `$push+$each+$slice:-8`（:139） | ✓ |
| 4 | gateway.rs:145-151 | handle_managed_message 直委托 run_user_operation_gateway(Inbound, None, None) | ✓ |
| 5 | gateway.rs:184-192 | 分派：principal_decision_relay→escalation；DURABLE_INBOUND_REPLY_KIND→durable | ✓ |
| 6 | gateway.rs:234-239/269-293 | durable：claim 必须有；content=消息 ObjectId；100ms 监视器（:274）；monitor.abort()（:357） | ✓ |
| 7 | gateway.rs:598-661 + 调用点 | tokio::join! 并行；调用点恰 4 个 848/3002/3128/3398 | ✓ grep 精确 |
| 8 | gateway.rs:1653-1726 | abort_on_reaction_stop：stop→cancel+事件+run log；分析失败仅 warn（1672-1674） | ✓ |
| 9 | gateway.rs:2452/2629 | barrier_stage = before_non_text_outbox / after_first_reply | ✓ |
| 10 | gateway.rs:2496 / 3390 | should_refresh_context 恒 false；revision 30s timeout | ✓ |
| 11 | gateway.rs:3915-3924 + 6166 | 疑点 1：注释仍以 apply_agent_updates 措辞书写；该函数主路径已无内联调用 | ✓ 裁决成立（grep：定义 6166，调用仅 post_decision.rs:935/1109） |
| 12 | gateway.rs:4180-4216 | outbox_eligible 三条件；relay 泄漏守卫 fail-closed（4198 blocked_by_safety_guard） | ✓ |
| 13 | gateway.rs:4306-4326 | `#seg{idx}` 仅 total>1；decision_id=Some(review_id)/max_attempts:3；source_kind=trigger.kind()（4321） | ✓ |
| 14 | gateway.rs:5245-5366 | precheck 九门全序（各门行号 5251/5259/5263-5267/5269-5273/5283-5288/5292-5296/5302-5308/5319-5340/5342-5357） | ✓ 全部精确 |
| 15 | gateway.rs:5550-5605 / 7352-7462 | ACK 豁免清单 8 项；`#ack-placeholder` 键 + decision_id=None；水位 filter 含/不含边界；context 窗 clamp(24,80)（7460） | ✓ |

### 2.2 记录 03（webhooks/tasks）——14/15 通过，1 失配

| # | 锚点 | 记录断言（摘要） | 结果 |
|---|---|---|---|
| 1 | webhooks.rs:26-31 | kind/active_key/handoff 四值常量组 | ✓ |
| 2 | webhooks.rs:39-53 | SHA256(ws\0acct\0wxid\0kind\0) 前 12 字节确定性 ObjectId | ✓ |
| 3 | webhooks.rs:55-74 | mark_inbound_handoff filter `$in [pending,deferred]`（:65） | ✓ |
| 4 | webhooks.rs:81-94 | run_at = created_at+debounce_window、schedule_reason="debouncing" | ✓ |
| 5 | webhooks.rs:112-141 | AgentTask{content=hex(:120), review_required=true(:123), max_attempts=3(:125)} + 5 个附加字段（136-141） | ✓ |
| 6 | webhooks.rs:143-204 | newer 谓词 + manual/refresh 双分支；**"$unset 清 12 个字段"** | **✗ 失配**：refresh 分支 $unset 实际 **11 个**字段（196-201：expires_at/source_decision_id/next_retry_at/cancel_reason/error/claimed_at/claim_token/outbox_decision_id/prepared_commit_kind/prepared_commit/manual_reply_run_id）。其余断言（filter 不含 status、水位单调）全部正确 |
| 7 | webhooks.rs:210-222 | 无条件 mark materialized + 读回行内 run_at | ✓ |
| 8 | webhooks.rs:374-453 | advance 单调 CAS；settle_ai 两步（先 advance 后精确 latest-CAS + manual 排除） | ✓ |
| 9 | webhooks.rs:1878-1913 | parse_inbound_msg_type 无键默认 "text"；classify 11 类归一 + unknown | ✓（text/image/voice/video/namecard/emoji/location/appmsg/voip/statussync/system 恰 11 类） |
| 10 | webhooks.rs:2845-2884 | 验签：6 步校验序、"<ts>."+raw body、恰等边界通过（>skew 才拒）、A-04 注释 | ✓（尾部锚点精确 → 总行数差 1 无系统性偏移） |
| 11 | tasks.rs:136-160 | marker prepare/commit/send-terminal 三 filter 形状 | ✓ |
| 12 | tasks.rs:186-244 | claim CAS：status 注入 {pending,retry,failed}、ReturnDocument::After、$inc attempt+generation、$unset 三项、i64/i32 兼容缺省 1 | ✓ |
| 13 | tasks.rs:468-535 | authorize：count=0 拒 → marker matched==total → task CAS → notify 立即+1050ms | ✓ |
| 14 | tasks.rs:549-559 | inbound reply worker 固定 250ms + "恢复永不排在画像/campaign 后"注释 | ✓ |
| 15 | tasks.rs:1183-1222 | provider 恒 300s + `$inc attempt_count:-1`；退避 base=min(60·2^(n-1),900)±20%（capped clamp(1,6)）；心跳 (timeout/2).clamp(5,60) | ✓ |

另：03 声称总行数 webhooks.rs 3089 / tasks.rs 1972，实测 3088 / 1971（各多报 1 行）。头/中/尾锚点均精确命中，判定为"共 X 行"计数笔误，不影响任何 file:line 引用。

### 2.3 记录 04（decision/review/guards）——15/15 通过

| # | 锚点 | 记录断言（摘要） | 结果 |
|---|---|---|---|
| 1 | decision.rs:589-608 | decide_reply_with_promote 17 参数、返回 (AgentDecision, Vec<String>) | ✓ 参数逐个数恰 17 |
| 2 | decision.rs:620-625 | include_relational = tier∈{Relational,Full}；include_business = tier==Full | ✓ |
| 3 | decision.rs:1315-1325 | generate_agent_json prompt key `"user.reply.fast.task"`（:1321） | ✓ |
| 4 | decision.rs:1338-1345 | H9 profile.conversation_modes 覆盖；**validate_reply_critical**（非 validate_and_promote） | ✓（04 正确记录了代码事实；注意 decision.rs:1326-1332 源码注释仍写 validate_and_promote，04 未被带偏） |
| 5 | review/mod.rs:3236-3259 | effective_review_mode 判定链 + confidence 缺省 10 < 阈值 → full | ✓ |
| 6 | review/mod.rs:3266-3275 | should_run_targeted_rewrite 四条件公式 | ✓ |
| 7 | review/mod.rs:3277-3285 | should_run_review 恒等于 should_reply | ✓ |
| 8 | gates.rs:20-47 | review_passed 六项 AND（26/27/28/34-36/39-41/45-46） | ✓ 全部行号精确 |
| 9 | gates.rs:64-81 | reviewer 事实面恰 13 键 | ✓ 逐键命中 |
| 10 | gates.rs:460-500 | GatewayStatusFinal 六变体 + gateway_status_str（481-492）/final_review_status_str（495-500） | ✓ |
| 11 | gates.rs:548-558 | principal 豁免：granted==true，缺任一层 false（fail-closed） | ✓ |
| 12 | gates.rs:1163-1196 | decide_revision：NotEligible×3 / Skip×2（reason+event 字面量）/ Proceed | ✓ 字面量逐字命中 |
| 13 | guards.rs:187-254 + 260-266 | check_state_transition 六分支（None fail-open→空 states fail-closed→unknown_target 拒→allowFromAny→initial→allowedFrom）；action 闭集 5 值 | ✓ |
| 14 | guards.rs:517-533 | is_verified trim 后精确等小写 "verified" + valid_to；claim_requires camel/snake 双键 | ✓ |
| 15 | types.rs:1507-1526 + taxonomy.rs:393-441 | hold_category 闭集 3 + 禁用集 5；check_value scope 序 [account,"global"] + 四分支优先级 | ✓ |

### 2.4 记录 05（outbox/escalation/send）——15/15 通过（另有 §3.3 一处跨文件接线断言错误，见 §1.G）

| # | 锚点 | 记录断言（摘要） | 结果 |
|---|---|---|---|
| 1 | outbox.rs:141-165 | EnqueueRequest 11 字段（decision_id Option / media 与 referral 互斥注释 :162） | ✓ |
| 2 | outbox.rs:169-185 | priority：媒体/名片一律 20；manual 100/inbound 90/escalation 80/follow_up 60/incident 40/其它 50 | ✓ 逐值命中 |
| 3 | outbox.rs:188-204 | run_sequence：名片 20000/媒体 10000/#segN 解析（≥0，失败→0） | ✓ |
| 4 | outbox.rs:224-240/256 | 入参校验（content 必填按类型）+ day_bucket = now/86400000 | ✓ |
| 5 | outbox.rs:262-286 | 媒体/名片一律 synthetic（硬伤③方案甲注释 257-261）；非 synthetic key `{event}:{wxid}:{hash}` 不含 run_id（279-283）；v2 scoped（285-286） | ✓ |
| 6 | outbox.rs:288-315 | synthetic warning 事件；max_attempts `<=0→3, min(10)` | ✓ |
| 7 | outbox.rs:599-607 | backoff `(2^clamp(attempt,0,10))×5s` ±20% | ✓ |
| 8 | outbox.rs:610-628 | outcome_signals_stop 双词；可取消集仅 {pending,in_flight}、脏值不可取消 | ✓ |
| 9 | outbox.rs:640-670 | 二次门四条件顺序（not_managed_at_send/contact_cooldown_active/user_stop_requested_after_decision/outbox_stale_30min）+ B-03 注释 | ✓ 字面量逐字命中 |
| 10 | outbox_dispatcher.rs:154-195 | 九常量：150s/15s/2(cfg test)/30min/20/16/60s/5/worker_id=host:pid:uuid | ✓ 全部精确 |
| 11 | outbox_dispatcher.rs:412-452 | classify 三层判定（token/decision→marker→status）；"task sent 只表示文本段送达"注释（441-443） | ✓ |
| 12 | outbox_dispatcher.rs:454-559 | review 状态三分（enqueuing→building/enqueued\|sent→非 building/其它→Stale）；marker 修复（518-551）；building 降级（552-558） | ✓ |
| 13 | outbox_dispatcher.rs:563-626 | defer +1s 无损（574）不加 attempt；enforce 三分派（Stale→cancel "stale_task_claim: …"） | ✓ |
| 14 | escalation/logic.rs:199-219 | is_principal_relay_trigger 唯一判据 is_synthetic_relay（H10）；泄漏守卫 4 标记 | ✓ |
| 15 | holding_reply.rs:45-71 + quiet_hours.rs:20/28-67 | 三关守卫（非空/禁词 lint/数字白名单，"None=授权集为空而非关闭校验" :57-59）；DEFERRED kind、[start,end) 含头不含尾、start==end 永不静默、FNV-1a jitter | ✓ |

### 2.5 记录 06（memory/reaction/runtime）——15/15 通过

| # | 锚点 | 记录断言（摘要） | 结果 |
|---|---|---|---|
| 1 | memory.rs:438-447 | core 统一排序 + split_off(6) 容量淘汰 + eviction 注记 coreFactRank=7+offset | ✓ |
| 2 | memory.rs:589-617 | 权威分：operator_manual=100 / confirmed_tag=20 / LLM 基础 1+msg_ids 8+evidence 4+run_id 2（最高 15） | ✓ |
| 3 | memory.rs:701-714 | 非原子三判据：\n≥2 ∨ 句界（。！？;；）≥2 ∨ >80 chars | ✓ |
| 4 | memory.rs:2833-2841 | decide_candidate_status：write_score≥6 ∨ max_importance≥8 → pending，否则 ignored_low_score | ✓ |
| 5 | memory.rs:2843-2864 | 触发全集：requested ∨ score≥6 ∨ pending≥4 ∨ 最老≥6h（常量 2843/2844） | ✓ |
| 6 | reaction.rs:30-35 | ReactionOutcome{claimed, outcome_status, stop_requested} | ✓ |
| 7 | reaction.rs:52-58/67-78 | 确定性 stop 在加载 prompt/config/claim 之前短路；reaction 专属 RunBudget scope | ✓ |
| 8 | reaction.rs:247-277 | 100 年 cooldown（100×365×24×60×60×1000）+ operation_policy.explicitStopRequested[At] + DB 失败仍返回 stop_requested=true | ✓ |
| 9 | reaction.rs:368-400 | claim：filter status=sent + outcome∈[null,pending]、sort created_at:-1、$set analyzing+token、$inc generation、拿不到→default | ✓ |
| 10 | sufficiency.rs:103-120 | KB-01：仅 forced_full ∨ escalated_to_full 记 used_knowledge_ids；非 Full 一律清空（含 carry_through 自报值） | ✓ |
| 11 | post_decision.rs:140-214 | persist：读 profile_revision 作 baseline（153-171）；$set prepared+attempts=0（191-207）；matched≠1 → Err | ✓ |
| 12 | post_decision.rs:585-617 | activate 注释"only after no-reply settlement or durable text-send authorization"；prepared→pending / prepared→discarded+$unset payload | ✓ |
| 13 | post_decision.rs:619-641 | runnable_filter：status∈[outbox_enqueued,sent,no_reply] ∧（prepared/pending/retry 且 due ∨ processing 且锁过期）——疑点 4 亲证属实 | ✓ |
| 14 | run_envelope.rs:34-79 | source_kind 6 常量 + lifecycle 7 态 + finalReviewStatus 闭集恰 10 值 | ✓ 逐值命中 |
| 15 | budget.rs:137-162 + multimodal.rs:49-55 | try_reserve_llm_call 原子占位（calls≥effective 即拒）；effective=base+bonus；fetch_inbound_media 打桩恒 Ok(None) | ✓ |

---

## 3. 需回写修正清单

按优先级排序（回写由主会话执行，本文件不改动其它记录）：

1. **【实质错误】05 §3.3「escalation 与 gateway/tasks 的接口」relay 条目**
   - 现文："relay……出站前有 `relay_output_leaks_internal_payload`+数字白名单双守卫，仍走 outbox"。
   - 正确表述："relay 转述出站前有 `relay_output_leaks_internal_payload` **单一**代码守卫（gateway.rs:4193-4215）；字符级数字白名单 backstop 已因威胁模型错误从 relay 转述路径删除（KD-01/03，gateway.rs:4188-4192 注释）。`relay_introduces_unauthorized_number`（logic.rs:259-269）现存唯一生产消费点是 holding reply 安抚话术守卫（holding_reply.rs:60）。"
   - 建议同时在 05 §5 增补一条偏差记录：logic.rs:222 与 :255-258 的函数 doc 注释本身已过时（仍称"用于 relay 转述的数字白名单核验""网关据此不发该转述"），是本次误写的根因。
2. **【计数小错】03 §2.1 `materialize_durable_inbound_task_at` refresh 分支**
   - 现文："`$unset` 清 12 个字段"。
   - 正确表述："`$unset` 清 **11** 个字段（webhooks.rs:196-201：expires_at / source_decision_id / next_retry_at / cancel_reason / error / claimed_at / claim_token / outbox_decision_id / prepared_commit_kind / prepared_commit / manual_reply_run_id）"。
3. **【行数笔误】03 头部与 §1/§6**
   - 现文：webhooks.rs "共 3089 行"、tasks.rs "共 1972 行"（§6 亦引 "1–3089 / 1–1972"）。
   - 实测（wc -l，2026-08-13）：**3088 / 1971**。全部函数级锚点（含尾部 2845-2884 / 1719-1971 区段）无偏移，仅总数多报 1，改数字即可。

以下为**不需回写**的记录级观察（供参考，不构成错误）：
- 01 引用 "types.rs:1512-1515"（HOLD_CATEGORY_VALUES），数组含结尾 `];` 实为 1512-1516，属边界半行差异，语义无误。
- 04 §6 自述 "gateway.rs 仅 grep 调用点行号（…:2999/:3025…）"中 2999/3025 为块级近似（精确语句在 3002/3026 附近），04 已自declare 未通读 gateway，判定树正文引用的 626/645/3040/3172/3194/3313/3426/3435 全部精确。
- decision.rs:1326-1332 源码注释写 `validate_and_promote` 而代码调 `validate_reply_critical`——04 记录的是代码事实（正确side），该源码注释漂移可作为将来代码清理线索。

---

## 4. 综合可信度评估

| 记录 | 一句话评估 |
|---|---|
| 01（gateway） | **极高**：15/15 锚点精确到行，接口断言全部胜诉（含"数字白名单已删除"这种反直觉细节），自报疑点 1（注释过时）经 grep 裁决成立，可直接作为改动依据。 |
| 03（webhooks/tasks） | **高**：协议级断言（水位/claim/授权链/验签/退避）全部精确，仅 1 处字段计数（12→11）与总行数 ±1 的笔误；修正后可作改动依据。 |
| 04（decision/review/guards） | **极高**：15/15 精确，含"validate_reply_critical 而非注释所说 validate_and_promote"这类注释-代码分辨力，闭集/阈值/判定式逐字可靠。 |
| 05（outbox/escalation/send） | **高**：锚点与常量全部精确（含 9 个 dispatcher 常量逐值命中），但 §3.3 有一处被过时源码注释带偏的跨文件接线错误（relay"双守卫"），已裁决并列回写清单；引用其 relay 出站守卫结论前须以修正版为准。 |
| 06（memory/reaction/runtime） | **极高**：15/15 精确，疑点 4（runnable_filter 冗余语义）亲证属实，投影/反应契约与 01 完全咬合，可直接作为改动依据。 |

---

## 5. 覆盖自证

### 5.1 覆盖自证抽验（各记录声称读过的区段是否虚报）

抽验策略：优先抽文件**尾部测试区段**（最易虚报），核对记录中对该区段的具体描述是否与源码内容对应。5/5 通过，无虚报迹象：

| 记录 | 声称读过的区段 | 抽验点 | 结果 |
|---|---|---|---|
| 01 | 段 12/13（gateway.rs 7701-9152 tests） | 8151-8175 确为 `forged_sentinel_trigger_is_not_relay_exempt`（伪造哨兵不豁免，与记录引用一致）；8296-8305 确有 `MAX_BAYESIAN_SLOTS` 截断断言（`观察[5]="维度5"` 蕴含 =6，与记录"测试显示=6，gateway.rs:8304"一致） | ✓ |
| 03 | webhooks.rs 2400-2799（debounce_tests） | 2692-2742 确为 `concurrent_register_same_key_spawns_exactly_once`，N=16 线程 + spawn 恰一次 + generation==N 断言，与记录"并发 16 线程断言（2692-2742）"一致 | ✓ |
| 04 | types.rs 2001-2946 段 | 2529-2546 确为 `deferred_projection_conversion_cannot_authorize_delivery`（should_reply=false/reply_text 空/assets 空…），与记录"不可能授权发送，测试 :2529-2546"一致 | ✓ |
| 05 | outbox_dispatcher.rs 2858-3771 段 | 3645-3664 确为 `send_timeout_covers_worst_case_mcp_calls_and_stays_below_lease`（150≥60×2 且 <180），与记录"timeout-lease 时序不变量（:3645-3664）"一致 | ✓ |
| 06 | memory.rs 4400-5056 段 | 4616-4621 确为 `next_version_saturates_at_i32_max`（饱和不翻负、OCC 永不命中），与记录"测试 4616-4621"一致 | ✓ |

### 5.2 本次交叉验证亲读的源码证据清单

行数核对：`wc -l` 全部 47 个相关源文件（与五份记录声称逐一对照）。

Read 亲读区段（全部为定点核证，非通读）：
- `src/agent/gateway.rs`：96-151 / 174-363 / 598-677 / 1581-1740 / 3915-3932 / 4175-4339 / 5245-5369 / 5544-5605 / 7352-7462 / 8148-8179 / 8296-8313
- `src/webhooks.rs`：20-229 / 374-458 / 1876-1915 / 2692-2743 / 2845-2889 / 3085-3088（EOF）
- `src/tasks.rs`：136-245 / 466-535 / 549-562 / 1179-1224 / 1968-1971（EOF）
- `src/agent/decision.rs`：589-648 / 1313-1347
- `src/agent/review/mod.rs`：3236-3290
- `src/agent/review/gates.rs`：20-81 / 454-560 / 1157-1201
- `src/agent/guards.rs`：187-266 / 508-537
- `src/agent/types.rs`：1503-1532 / 2525-2550
- `src/agent/outbox.rs`：141-330 / 594-670
- `src/agent/outbox_dispatcher.rs`：139-198 / 401-630 / 3643-3667
- `src/agent/escalation/logic.rs`：193-272
- `src/agent/escalation/holding_reply.rs`：43-72
- `src/agent/quiet_hours.rs`：16-70
- `src/agent/memory.rs`：434-449 / 589-617 / 696-715 / 2833-2867 / 4612-4625
- `src/agent/reaction.rs`：28-82 / 242-281 / 366-403
- `src/agent/sufficiency.rs`：96-122
- `src/agent/post_decision.rs`：140-219 / 580-644
- `src/agent/run_envelope.rs`：34-83
- `src/agent/budget.rs`：137-166
- `src/agent/multimodal.rs`：41-60
- `src/agent/taxonomy.rs`：393-442

Grep 亲验（裁决关键）：
- `review_and_evaluate_claim_gate|apply_independent_claim_gate|ensure_independent_claim_gate|finalize_review_for_send|decide_revision|should_run_targeted_rewrite|decide_reply_with_promote|persist_finalize_pending_events`（gateway.rs 全部调用点行号）
- `relay_introduces_unauthorized_number`（全库——裁决 §1.G 矛盾的决定性证据：唯一生产调用点 holding_reply.rs:60）
- `apply_agent_updates\(|should_refresh_context = false|tokio::time::timeout\(`（src/agent 全部——裁决 01 疑点 1：apply_agent_updates 定义 gateway.rs:6166，调用仅 post_decision.rs:935/1109）
- `barrier_stage: "|persist_projection_snapshot|activate_projection|discard_projection`（gateway.rs——接口 E/F 的调用点全景：barrier 2452/2629；persist 4056；activate 4119/4709；discard 4052/4112/4274/4448/4517/4568/4639）

局限声明：本次为**定点交叉核证**而非五份记录的全量重读；未抽验到的锚点（每份记录数百个）不在本文担保范围内，但抽样横跨各章节且含反直觉断言与文件尾部区段，抽样通过率可外推为记录整体质量的合理估计。接口审计只覆盖任务书指定的七组接口；五份记录与 02（models/db）、07-09（knowledge）等其它记录的接口未在本次范围内。
