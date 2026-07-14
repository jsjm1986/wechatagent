# agent 旁挂能力深度逻辑审查 findings 台账（第一批）

> 接续 2026-07-11 主链路深度逻辑审查（53 findings，P0-P3 + F-01 全闭环）之后的新一轮。本批范围 = agent 旁挂能力子系统。**只审不修**——先出完整台账，再按 P0-P3 分批修（各走独立 brainstorming→SDD→PR）。
>
> 设计：`docs/superpowers/specs/2026-07-14-agent-capabilities-audit-design.md`
> 计划：`docs/superpowers/plans/2026-07-14-agent-capabilities-audit.md`

## 审查范围（4 簇 / ~10.6k 行）

- **簇A 记忆固化**：`src/agent/memory.rs`(3291) + `src/agent/consolidation_window.rs`(77)
- **簇B 标签体系**：`src/agent/taxonomy.rs`(1036) + `src/agent/decision_taxonomy.rs`(427) + `src/agent/tag_evidence.rs`(101) + `src/agent/bayesian_slots.rs`(202)
- **簇C 通用化底座**：`src/agent/domain_profile.rs`(2454) + `src/agent/domain.rs`(107) + `src/agent/domain_signals.rs`(456) + `src/agent/dimension_registry.rs`(449)
- **簇D 节流准入**：`src/agent/simulation.rs`(265) + `src/agent/pacing.rs`(51) + `src/agent/quiet_hours.rs`(357) + `src/agent/entitlements.rs`(1311)

## 方法论

- 4 个只读审查 subagent 分簇并行审（继承 Opus）+ 主控逐条亲验 file:line（复核属实性 + 因果链，驳回夸大）。
- 两态：**PLAUSIBLE**（纯读码推断）/ **CONFIRMED**（可构造推荐配置下真实触发）。
- 元家族：设计声称的不变量/闭环/口径，实现层有旁路 / 缺口 / 非原子窗口 / 新旧不对称。

## 严重度校准（防夸大）

- **High**：推荐配置下**确定性发生**的核心交互失效 / 红线破坏。
- **Medium**：需多条件叠加、或依赖 DB/LLM 瞬时故障注入才触发。
- **Low**：观测项 / 边缘 / 就绪债 / 死代码 / 文档-代码漂移。

## Finding 字段模板

```
### [X-NN] 一句话标题
- 入口频道: —
- 所属簇: A|B|C|D
- 类型: 幂等|竞态|错误处理|一致性|逻辑正确性|红线|文档-代码漂移|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

---

## 环节汇总（收尾时填）

- 总 findings 数：（待填）
- 严重度分布：H / M / L（待填）
- 元家族归纳：（待填）
- 后续 P0-P3 修复路线建议：（待填）

---

## 簇A 记忆固化 findings

（主控亲验后填入）

## 簇B 标签体系 findings

（主控亲验后填入）

## 簇C 通用化底座 findings

（主控亲验后填入）

## 簇D 节流准入 findings

（主控亲验后填入）
