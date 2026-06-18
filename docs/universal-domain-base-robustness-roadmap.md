# 通用化底座强壮性收口路线图

> **日期**：2026-06-18
> **基线**：HEAD = `cff6e88`（B 阶段后端轴已落）
> **来源**：合并两份独立审查 —— ① `docs/universal-domain-base-extensibility-audit.md`（4 扩展轴，后端 only，逐条 file:line 亲核 + 诚实证伪）；② 本会话四维架构审查（抽象承重力 / 数据演进 / 扩展契约 / 并发性能故障）。
> **方法**：两份发现交叉对齐、去重、按严重度归并。采信 audit 的逐条亲核降级（本项目审查 agent 历史常臆造误报，audit 已证伪多个初判 high）。
> **性质**：这是**路线图（评估 + 排序）**，不是单主题实施计划。用户选定主攻主题后，该主题再走 brainstorming → spec → writing-plans 完整流程。

---

## 一句话结论

**"AI 怎么思考"是真骨架（运行时引擎层通用化扎实、回落契约干净），扛得住换行业；没收口的是三类东西：扩展动作分散无统一注册点、Contact 画像数据裸奔无校验、数字分身只做了后端一半。工程上单副本中小规模稳，多账号/多副本/大规模有明确前置项。**

两份审查在"引擎层是真通用"上完全一致；audit 的逐条亲核把若干初判 high 证伪/降级（C4-a 迁移兼容回落、C4-b 闸结构豁免、objection_type 不进决策），这些采信 audit。

---

## 已实证的"真做扎实"（不必再碰）

- 状态机引擎泛化（`guards.rs:144` 只读 initial/allowFromAny/allowedFrom，不认具体状态名）
- 五闸阈值 profile 派生 + grounding 硬闸条件化（`review/gates.rs`）
- 负反应极性、记忆维度、人格/方法论/经营公式 override 真消费（None 回落 / DEFAULT 字节等价护栏处处守）
- DomainProfile 22 字段几乎全真驱动（仅 `domain_schema_id` 死字段）
- 引导层闭环（AI 生成 profile→审→激活，draft 不自动生效红线守得住）
- serde 向后兼容是贯彻的纪律、迁移幂等、索引以 workspace_id 打头、webhook 落库即 ack 不阻塞微信、worker 被 supervisor 重启、LLM 故障优雅降级

---

## 五个主题（合并去重后，按价值/依赖排序）

### 主题 1 — 扩展点收拢：维度 registry + profile 字段中央接线（两份都点名，最高频痛点）
**问题**：系统"加东西"时没有单一模式，散弹改多处、易漏步 drift。
- **C3（audit high）**：加一个 DomainProfile 字段要在 5+ 文件各走各的渲染点；`apply_active_profile`（runtime.rs:261）只接了 3 个 runtime 标量，prompt/planner/catalog 类字段各自散落。
- **B1（audit high）**：加一个维度字典扩展点散落 ≥4 处、无 registry；"新维度要不要进 profile_dimensions / 要不要 typed 字段"的判断散在各 migration 注释里。
- **B4（audit medium）**：LLM 维度走白名单、admin 直写绕白名单——两条写入路径语义不统一、无单一声明。
- **driver 无抽象（本会话审查）**：七种驱动力是 `OperationMode` 七个具名字段非 `Vec<Driver>`/trait，加第八个散弹改 6+ 处 + 复制 ~200 行样板，daily_cap 口径已在 scan_calendar vs scan_silent 间漂移。
- **五闸维度写死（本会话审查）**：阈值可配但闸门数量写死，加"第六道闸"要改 ReviewScores 结构。

**价值**：最高——**解锁后续所有扩展**（加维度/加字段/加驱动力都变容易，且消除 drift 源）。
**风险**：低-中（纯重构向，不动行为，但触及多个中心文件，要严守 DEFAULT 字节等价）。
**工作量**：中-大（registry 收拢 + 接线点重构 + driver 框架可分阶段）。
**依赖**：无前置，是其它主题的地基。

### 主题 2 — 数据完整性：Contact 画像写入接 DomainSchema 校验（本会话审查新增，audit 未覆盖此角度）
**问题**：
- **Contact schema 校验裸奔（本会话审查，高）**：`enforce_domain_attributes`（domain_schemas.rs:544）只在知识 chunk 写路径调用，**Contact 画像写入完全绕过**。customer_stage/value_tier/relationship_type 可写 enum 越界值、required 永不强制、key 拼写漂移静默失败（到处 `get_str().ok()` 软读，零编译保护）。
- **objection_type 假字典（audit B2，medium，零风险快赢）**：声明字典、实现裸 string，`build_intent_trajectory_entry` 不过 normalize。影响面是轨迹数据卫生（不进五闸/状态机）。
- **key 字面量散落（本会话审查）**：高频 key 裸字符串散落各处，仅 `AWAITING_PRINCIPAL_DECISION_ATTR` 一个抽了常量。

**价值**：高——未来加维度的安全网，不补会越加越脏。
**风险**：中（碰写入路径，要保不误拒合法历史值）。
**工作量**：中。
**依赖**：与主题 1 的"维度 registry"天然协同（registry 正好是 schema 校验的声明源），建议主题 1 之后或合并设计。

### 主题 3 — 数字分身闭环：relationship_type 接 LLM 识别 + 前端显形（两份都点名）
**问题**：
- **D3（audit medium）**：relationship_type 只有 admin 手动直写，未进 profile_dimensions、未接 LLM 自动识别——"谁是客户/同行/朋友"靠运营逐个手标，是人工瓶颈。
- **前端 #2（路线图原 B 下半，未做）**：运营态 UI 不读 active profile、销售频道无条件显形、value_tier/purchase_lifecycle/churn_reason 前端零显形、stage 直接显原值无翻译。

**价值**：高（兑现"AI 化身自动托管"产品愿景 + 让非销售场景真正可用）。
**风险**：中（D3 接 LLM 通道需反过拟合多 seed 验证；前端纯展示低风险）。
**工作量**：大（跨后端 LLM 通道 + 前端 4 新建点）。
**依赖**：D3 进 profile_dimensions 与主题 1 的 registry 协同；前端复用 admin 端点零后端改动。

### 主题 4 — 规模/多租户前置（本会话审查新增，audit 声明范围外）
**问题**：
- **planner 只扫 default_account（本会话审查，功能黑洞）**：多 workspace/account 的 contact 根本不被主动触达——这是功能正确性缺口，非纯性能。
- **contacts 缺索引全表扫 + N+1（中等规模暴露）**：planner 每 tick 全账号扫，10 万 contact 显著变慢。
- **进程内状态 + planner 无 claim（多副本暴露）**：去抖/限流是进程内状态，上多副本即破功（同 contact 并发回复、限流翻倍、planner 双 emit）。
- **连接池默认 10 + LLM 无熔断（现在就有隐患）**：长时 LLM 故障在途 pipeline 堆积抢连接。
- **DomainProfile 缓存无 single-flight + invalidate 未接线**：publish 后最多 30s 才生效。

**价值**：取决于规模目标——单账号中小规模不紧迫；要多账号/多副本/上量则是硬前置。
**风险**：中-高（碰核心数据访问 + 部署拓扑）。
**工作量**：大（索引 + planner 多账号化 + 分布式 claim + 熔断分多个专项）。
**依赖**：planner 多账号化是多账号产品的硬前置；其余按规模触发。

### 主题 5 — 零风险快赢 + 需专项
- **C1/C2（audit low）**：删 `domain_schema_id` 死字段、修 `domain_profile.rs` 模块头过时注释。零风险，顺手清。
- **A1'（audit，需专项）**：整条私聊链路硬绑 `domain="user_operations"` 字面量。串行换配置场景不紧迫；要并行运营多套行业才需参数化。
- **models.rs 5301 行单文件（本会话审查，技术债）**：可按子域拆分，正确性无虞，拆分时机已到。
- **迁移破坏性单向 + 无事务（audit 未覆盖，本会话审查 low）**：前滚安全，降级旧代码丢数据。

---

## 推荐推进顺序

> **落地进度（2026-06-18）**：主题 1+2 的**核心**已合并为一个 spec 落码——见 `docs/superpowers/specs/2026-06-18-dimension-registry-and-validation-design.md` + 计划 `docs/superpowers/plans/2026-06-18-dimension-registry-and-validation.md`。已交付：维度元数据单一真相源 `src/agent/dimension_registry.rs`（收敛 B1 散落 typed 列表）；Contact 三写入路径接 `validate_dimension_value`（admin reject / LLM drop+审计 / objection_type 归一，补 B2 假字典脱节）；新增 `WriteIntent` 正交轴（admin 写一律 reject）。lib 1308/0、四 PBT 36/0、lint 0。
> **主题 1/2 仍未做的子项**（本 spec 按 YAGNI 排除，留后续）：C3 profile 字段中央接线点（apply_active_profile 只覆盖 runtime 标量）、driver 框架抽象、五闸数量可配；主题 2 的 key 字面量全面常量化（仅维度 kind 经 registry 收敛，其它高频 key 仍散落）。主题 3（数字分身前端 + D3 LLM 识别）/ 主题 4（规模）未动。

| 顺序 | 主题 | 价值 | 风险 | 工作量 | 理由 |
| --- | --- | --- | --- | --- | --- |
| **第一** | 主题 1 扩展点收拢 | 最高 | 低-中 | 中-大 | 地基，解锁后续所有扩展，消除 drift 源 |
| **第二** | 主题 2 数据完整性 | 高 | 中 | 中 | 与主题 1 registry 协同（registry=schema 声明源），趁热接上 |
| **第三** | 主题 3 数字分身闭环 | 高 | 中 | 大 | 产品愿景兑现，依赖主题 1 的 registry/接线点 |
| **按需** | 主题 4 规模前置 | 规模相关 | 中-高 | 大 | 要多账号/多副本/上量才做，planner 多账号化是硬前置 |
| **穿插** | 主题 5 快赢 | 低 | 零 | 小 | 任何主题间隙顺手清（死字段/注释/objection_type） |

**我的建议**：先做**主题 1（扩展点收拢）+ 主题 2（数据完整性）合并为一个 spec**——它们天然协同（维度 registry 同时是 schema 校验的声明源），一起做能一次性把"加维度"这件事从"散弹改 4+ 处 + 无校验"变成"在 registry 声明一处 + 自动校验"。这是价值最高、解锁最多、风险可控的一块。主题 3/4 在它之后按产品节奏推。

---

## 验证基线（任何主题落地都须守）

- `cargo test --lib` ≥350 / 0；四 PBT 累计 ≥33 / 0
- `scripts/check-no-human-takeover.sh` clean
- DomainProfile 结构改动须 serde 向后兼容 + DEFAULT 销售域字节等价测试
- 改 prompt/rubric 守过拟合红线（只沉淀可复现抽象，不点对点修补）
- 重构向改动用 subagent 多维交叉验证后再提交（byte-equiv / 回落安全 / 红线）

## 约束

- 这是路线图，**不实施**。用户选定主题后该主题走 brainstorming → spec → writing-plans。
- 提交/推送需用户显式批准；commit 精确 add 排除并行会话产物。
- 子代理用 model:opus；回复中文。
