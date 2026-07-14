# evolution 自优化演化器深度审查（第五批）Findings 台账

> 只审不修 · 纯 docs 台账 · 沿用前四批「根因层先派→资源域并行 + 主控逐条亲验」范式。

## 审查范围（4 簇 / 6095 行 14 文件）

| 簇 | 审查对象 | 行数 | 重心 |
| --- | --- | --- | --- |
| **S 演化根因层**（先派等回） | `mod.rs`(386) + `budget.rs`(223) + `runtime_flag.rs`(195) + `cohort.rs`(232) + `envelope.rs`(111) + `error.rs`(28) + `lint.rs`(79) | 1254 | worker 门控 + run_one_tick 9 步编排；EvolutionBudget 硬上限；runtime_flag 灰度门读失败兜底；cohort 分桶；envelope 状态机；隔离红线。产出**演化安全基准**喂簇1-3 |
| **1 候选生成** | `threshold.rs`(467) + `prompt_critic.rs`(606) | 1073 | threshold 纯统计候选正确性；prompt_critic LLM 候选（消 budget + silent skip）；候选落 pending_eval 不直接生效；proposal 幂等 |
| **2 shadow评估** | `replay.rs`(909) + `significance.rs`(996) | 1905 | replay shadow 隔离（不碰真实发送/gateway）；eval 预算；significance 统计显著性判定正确性；坏候选误判 eligible；eval 幂等 |
| **3 放量闭环** | `release.rs`(855) + `auto_release.rs`(519) + `post_release.rs`(489) | 1863 | **自动化越界红线**（release 需 admin 二次确认 / auto_release 唯一自动放量 / 绝不自动回滚 R9.7）；release 应用候选原子性；post_release +24h 窗；rollback 全 admin 手工；release 幂等 |

## 方法论（沿用前四批）

1. **簇S 根因层先派并等回** —— 提炼「演化安全基准」（自动化边界/预算硬上限/灰度门兜底/隔离红线/失败隔离）喂 Task1-3 当审查标尺。
2. **Task1-3 TaskS 完成后并行派** —— 各簇只读 subagent（general-purpose，继承 Opus 省略 model），带 TaskS 基准 + 检查清单 7 问。
3. **主控逐条亲验** —— 每 finding Read/Grep 复核 file:line + 失效链成立性，驳回夸大严重度。
4. **两态**：PLAUSIBLE（读码）/ CONFIRMED（能构造触发序列）。

## 演化安全/幂等/统计检查清单 7 问

1. **幂等**：同一 experiment/proposal 重复 tick / 重复 eval / 重复 release 是否重复副作用？
2. **统计正确性**：threshold 统计口径、significance 显著性判定是否有算错/误判致坏候选晋升？
3. **自动化越界红线**：所有 release 都经 admin 二次确认？auto_release enabled 门控严格？**绝不自动回滚**（R9.7）HOLDS？
4. **预算旁路**：replay/prompt_critic 的 LLM 消耗是否都过 EvolutionBudget？
5. **shadow 隔离**：replay/eval 是否真 shadow（不碰真实发送/生产 chunk/gateway）？
6. **一致性（非原子写）**：envelope 多字段分步 update、proposal 状态流转是否有中间失败留不一致？
7. **无界增长/崩溃恢复/best-effort 吞错**：experiment/proposal 无界堆积无 TTL？post_release 扫描崩溃能否回收？unwrap_or_else 吞错掩盖真错？

## 严重度校准（防夸大，沿用前四批口径 + 自动化越界维度）

- **High**：推荐配置（EVOLUTION_ENABLED 默认 true + 单进程 + admin 二次确认闭环）下**确定性可达**的：生产 prompt/threshold 被错误放量、绕过 admin 二次确认自动 release、**自动回滚**（R9.7 明禁）、演化越界写生产链路（隔离红线破洞）、统计显著性判定错误致坏候选晋升。
- **Medium**：需并发/崩溃时机叠加 / 多副本 / 多租户才触发，或有自愈兜底。
- **Low**：观测/边缘/无界增长无立即后果/就绪债/桩未接线。
- **单进程默认不可达的多副本竞态=水平扩展就绪债；单租户默认不可达的隔离缺陷=多租户就绪债——都不夸成 High**（[[project-multitenant-isolation-debt]] 口径 + 第三批部署拓扑维度）。

## 元家族聚焦

本批预期主线元家族=**「自优化闭环声称的安全不变量（绝不自动回滚/release 需 admin 确认/预算硬上限/shadow 隔离/统计正确）在实现层的旁路/自动化越界」**——前四批「声称不变量实现层有旁路/层间不对称」元家族在自优化闭环侧的延伸。

## 边界排除（防与既往批次重叠）

- `agent::*` 生产链路（前四批已审）——仅在隔离红线对照时引用不重审。
- `routes::evolution` HTTP 授权面（第二批 auth/routes 已审）——本批仅审 worker 侧逻辑。
- `prompt_critic` 调 `generate_agent_json` 的 LLM 入口本身（主链路已审）——仅审 evolution 侧调用契约 + budget 记账。

## 环节汇总（收尾时填）

- 总 findings 数：（TaskE 填）
- 严重度分布：（H/M/L，TaskE 填）
- 元家族归纳：（TaskE 填）
- 后续 P0-P3 路线：（若有 High 优先级高于前四批遗留 Medium，TaskE 填）
- 交叉去重留痕：（TaskE 填）
- 正向 HOLDS（主控亲验）：（TaskE 填）

---

## 字段模板

```
### [X-NN] 一句话标题
- 入口: —（函数/worker）
- 所属簇: S|1|2|3
- 类型: 自动化越界|统计正确性|幂等|一致性(非原子写)|隔离红线|预算旁路|时间窗竞态|无界增长|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 失效链: （谁在什么时机触发什么错误放量/统计错误/红线绕过后果；非失效类填 —）
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

---

## 簇 S findings（演化根因层）

（主控亲验后填入）

## 簇 1 findings（候选生成）

（主控亲验后填入）

## 簇 2 findings（shadow评估）

（主控亲验后填入）

## 簇 3 findings（放量闭环）

（主控亲验后填入）
