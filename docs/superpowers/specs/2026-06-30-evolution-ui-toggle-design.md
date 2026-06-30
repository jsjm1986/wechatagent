# 演化中心 UI 开关 设计 spec

> 日期：2026-06-30　分支：feat/evolution-ui-toggle（基线 origin/main e5e3952，含 #71）

## 问题

演化中心（自优化器 / `src/evolution/`）当前默认关闭，且"关闭"只能通过改 `.env` 的 `EVOLUTION_ENABLED=false` + 重启进程实现，管理员无法在 UI 里开关。需求：**在管理界面里一键开关演化中心，运行时即时生效、不重启**。

## 核查确认的现状（动手前已全量读代码核实，无猜测）

演化中心的"默认关闭"是**三层门控叠加**：

1. **env 进程级总闸 `EVOLUTION_ENABLED`（默认 false）**：
   - 定义 `src/config.rs:193-194`，读取 `src/config.rs:584`（`parse_bool(env_or("EVOLUTION_ENABLED","false"))`），`.env.example:135`。
   - 门控 worker：`src/evolution/mod.rs:52-55` —— `run_evolutionary_worker` 进函数即 `if !evolution_enabled { return; }`，env=false 时根本不进 tick 循环。spawn 本身在 `src/main.rs:239-241` 无条件，门控在函数内部 early-return。
   - 门控前端：`/api/health` 回吐 `evolutionEnabled`（`src/routes/health.rs:13`）。
2. **mongo workspace 级运行时开关 `evolution_runtime_flags`（已存在）**：
   - 集合 accessor `src/db/mod.rs:317`，模型 `EvolutionRuntimeFlag`（`src/models.rs:1300-1321`，字段 `enabled: bool` / `rollout_percent: u32`(0-100) / `threshold_auto_release_enabled` / `updated_by` / `updated_at`）。
   - GET/PUT 路由 `/api/evolution/runtime-flag`（挂载 `src/routes/mod.rs:1033`，handler `src/routes/evolution.rs:568-636` upsert，按 `admin.current_workspace` 隔离，走 `require_session` 鉴权）。
   - tick 内**已有** mongo flag 灰度门：`src/evolution/mod.rs:104-115` 读 flag → `select_cohorts_filtered` 按 `hash(contact_id)%100 < rollout_percent` 分桶；flag 关或文档不存在 → 全员排除（worker 仍跑空 tick）。
   - 综合判定 `is_evolution_enabled_for`（`src/evolution/runtime_flag.rs:80-101`）：env=false 直接 false（不读 mongo）→ mongo 文档不存在 false → `flag.enabled=false` false → 灰度桶判定。
3. **前端 Tab 内容层**：
   - 频道入口 `frontend/src/app/channels.ts:240-249` **始终可见**（无 `visibleWhen`），不存在"频道隐藏"问题。
   - `frontend/src/features/evolution/index.tsx:11-24` 取 `/api/health` 拿 `evolutionEnabled` 传给 `<EvolutionCenterTab enabled={...}/>`。
   - `EvolutionCenterTab.tsx:243-249`：`enabled=false` 时整个 return 死占位"演化器未启用（EVOLUTION_ENABLED=false）"、跳过数据加载；灰度控件（`:272-294`）埋在 `enabled` 内部，env 关时**根本不可见**。

**关键结论**：mongo flag 当总开关的下半截机制（PUT 即时落库 + tick 内灰度门）**已经存在**；缺的是把 env 那道 early-return 从"硬关"改成"硬上限"，并把前端门控数据源从 env 切到 mongo flag。后端实质代码改动极小。

**主链路安全边界（已核实）**：关演化器只停止产出新的 `threshold_overrides`/prompt 候选；主对话链路通过 `resolve_thresholds`（`src/agent/gateway.rs:174`/`src/agent/runtime.rs:456`）**只读**消费历史覆盖，空集合回落 baseline 零影响。隔离红线 + CI（`scripts/check-evolution-isolation.sh`）保证演化模块不反向调用发送链路。开关它**不影响主对话发送链路本身**。

## 设计决策（已与用户对齐）

| # | 决策 | 取值 |
| --- | --- | --- |
| K1 | env 与 UI 开关关系 | **mongo flag 当真总开关，env 降为生产硬上限**。`EVOLUTION_ENABLED` 语义改为"是否允许 UI 开启"，默认 `true`（允许）；仅显式 `false` 才硬锁定（运维紧急熔断，无需 mongo 写权限）。出厂仍关——因 `evolution_runtime_flags` 默认无文档 = 关。 |
| K2 | env 变量取舍 | **复用 `EVOLUTION_ENABLED`，默认 false→true**，不新增变量。 |
| K3 | 总开关 ↔ 灰度比例 | **开=全量 100**。UI 总开关 on 时写 `enabled=true` + `rollout_percent=100`；灰度比例降为可折叠"高级设置"，不展开即全量。 |
| K4 | UI 形态 | **复用现有 runtime-flag 面板 relabel 为"演化中心总开关"，从 env-gated 占位中移出使其常驻可见**；env 硬锁时显示锁定态、控件 disabled。 |
| K5 | 前端数据源 | **收敛为单一数据源**。Tab 挂载时一次 `GET /api/evolution/runtime-flag`（已同时返回 `envEvolutionEnabled`+`flag`，`evolution.rs:579-583`）自行推导三态；`index.tsx` 不再 gate `/api/health`。health 的 `evolutionEnabled` 字段保留不动（监控探活用）。 |
| K6 | `enabled` prop 去留 | **保留**，不删除。现有 `__tests__/EvolutionCenterTab.test.tsx` 有 7 处 `<EvolutionCenterTab enabled={true}/>` 依赖此 prop，删除会破坏既有测试（违反"测试只增量叠加"纪律）。prop 语义从"env 总闸"重定义为"env 硬上限覆盖"：`enabled=false` 强制锁定态；prop 默认 `true`（不传即"env 允许"，由内部 flag 决定）。`index.tsx` 改为不传 prop（走默认 true），实际 env 锁定态由 `loadFlag` 拿到的 `envEvolutionEnabled` 驱动。 |

## 改动单元

### 后端（改动极小，复用现有 mongo flag 全链路）

**1. `src/config.rs:584`** —— `EVOLUTION_ENABLED` 默认 `"false"` → `"true"`。字段注释（`:193-194`）+ `.env.example:135` 同步：语义改为"是否允许在 UI 开启演化中心；设 false 为运维硬锁定（紧急熔断）"。

**2. `src/evolution/mod.rs:52-55`** —— early-return 的日志/注释措辞从"disabled (EVOLUTION_ENABLED=false); skip spawn"改为"硬锁定"语义（**逻辑不变**：env=false 仍 return 不进循环；env=true 进常驻 tick 循环，tick 内 mongo flag 门已存在，决定是否真选 cohort 干活）。模块头注释（`:8-9`）"由 main.rs 在 EVOLUTION_ENABLED=true 时 spawn"措辞同步为硬上限语义。

**3. GET `/api/evolution/runtime-flag`（`evolution.rs:568-584`）** —— 契约**不变**，已返回 `{workspaceId, envEvolutionEnabled, flag}`，前端直接消费。

**4. PUT `/api/evolution/runtime-flag`（`evolution.rs:587-636`）** —— 契约**不变**，已支持 `enabled`+`rollout_percent`。"开=100"由前端 saveFlag 传值决定，后端不改。

### 前端 `frontend/src/features/evolution/`

**5. `index.tsx`** —— 去掉 `/api/health` 门控（删 `:9-24` 的 fetch+enabled state），直接渲染 `<EvolutionCenterTab/>`（不再传 `enabled` prop）。大页头保留。

**6. `EvolutionCenterTab.tsx`** —— 三态渲染重构（**保留 `enabled` prop**，见 K6）：
- 签名 `{ enabled = true }`（`:155`）**不变**——prop 作为"env 硬上限覆盖"，`enabled=false` 强制锁定态（既有测试 `enabled={true}` 继续有效）。
- 挂载时 `loadFlag()` 已调 `GET /api/evolution/runtime-flag`，扩展其解析：新增 `envAllowed` state（来自 `resp.envEvolutionEnabled`），沿用 `flagEnabled`（`resp.flag?.enabled`）+ `rollout`。
- 计算有效门控：`const locked = !enabled || envAllowed === false`（prop 与 env 任一锁定即锁定）。
- 三态：
  ```
  flag 未回（首屏）   → "加载中…"
  locked             → 锁定态占位"演化中心已被运维硬锁定（EVOLUTION_ENABLED=false），请联系运维"，总开关 toggle disabled 灰显
  !locked:
    ├─ 总开关面板【常驻】：大 toggle「演化中心总开关」绑 flagEnabled，onChange 即调 saveFlag
    ├─ flagEnabled===false → toggle 下方提示"已关闭，打开后开始自动产出实验信封与候选"（非死占位，开关可点）
    ├─ flagEnabled===true  → 加载并展示实验信封/候选/聚合卡（现有逻辑）
    └─ 高级设置（可折叠，默认收起）：灰度比例 0-100 输入（现有 rollout 控件移入）
  ```
- "开=全量"：toggle 打开 saveFlag 写 `{enabled:true, rolloutPercent: 高级值||100}`；关闭写 `{enabled:false}`（rollout 值保留）。
- 数据加载 `load()`（`:224-241`）gate 条件从 `enabled` prop 改为 `!locked && flagEnabled`。

## 数据流（修复后）

```
EvolutionCenterTab 挂载
  └─ GET /api/evolution/runtime-flag → { envEvolutionEnabled, flag:{enabled, rolloutPercent} }
       ├─ envEvolutionEnabled=false → 锁定态（运维硬锁，UI 开不了）
       ├─ envEvolutionEnabled=true & flag.enabled=false → 显示可点的总开关（关态）
       └─ envEvolutionEnabled=true & flag.enabled=true  → 总开关开态 + 加载实验数据
  └─ toggle 打开 → PUT {enabled:true, rolloutPercent:100} → 即时落库 evolution_runtime_flags
       └─ worker 下一 tick（≤6h）读 flag → select_cohorts_filtered 全量选 cohort → 开始产出
```

## 错误处理（沿用现状）

- GET runtime-flag 失败 → 前端 catch 显错、不静默（现有 `loadFlag` catch `:175-177`）。
- worker 读 flag 失败 → 按 disabled 处理跑空 tick（`mod.rs:104-110` 已有）。
- `is_evolution_enabled_for` mongo 抖动 → false + warn（`runtime_flag.rs:91-99` 已有）。
- best-effort：关开关不回滚已 release 的历史 `threshold_overrides`（主链路只读消费、空集合零影响）。

## 测试

- **后端**：
  - `config.rs` 加单测：`EVOLUTION_ENABLED` 缺省解析为 `true`（覆盖默认值变更）。
  - worker early-return（env=false）现有行为不变，`is_evolution_enabled_for` / `bucket_for_contact` 现有 8 个测试（`runtime_flag.rs:103-195`）**不动**。
- **前端** `EvolutionCenterTab` 测试（vitest `--pool=forks`，路径含非 ASCII）：
  - **既有测试不动**（增量叠加纪律）：`__tests__/EvolutionCenterTab.test.tsx` 的 7 处 `enabled={true}` 仍有效（prop 保留，见 K6）；`__tests__/features/evolution/evolution.test.tsx`（测 `EvolutionFeature` 自取 health）需同步——`index.tsx` 不再 fetch health 后，该测试的 health mock 断言要相应更新（这是"既有测试需随接口变更同步"，非删维度）。
  - **新增**：env 锁定态（`envEvolutionEnabled:false` 或 `enabled={false}`）→ 显锁定占位、toggle disabled。
  - **新增**：env 允许 + flag 关（`envEvolutionEnabled:true, flag.enabled:false`）→ 显可点总开关、不加载实验数据。
  - **新增**：flag 开 → 加载实验数据、聚合卡渲染。
  - **新增**：toggle 打开 → PUT 被调且 body 含 `enabled:true` + `rolloutPercent:100`。
  - mock `apiGet("/api/evolution/runtime-flag")` 返回三态 fixture（含 `envEvolutionEnabled`）。
- **双 lint**：`scripts/check-no-human-takeover.sh` 0 violations（"演化中心总开关"/"运维硬锁定"等新文案合规）；新增行禁词扫描 0。
- 回归门：`cargo test --lib` ≥350/0；`RUSTFLAGS="-D warnings" cargo check --tests` EXIT=0；前端 vitest + tsc + build。

## 部署注意（写入 spec 供运维）

`EVOLUTION_ENABLED` 默认 `false→true` 后：**所有现有部署升级后 worker 都会进常驻 tick 循环**（默认 6h 一次，`EVOLUTION_TICK_SECONDS=21600`；flag 关时只写 experiment 信封跑空 tick，成本可忽略）。**生产 117 等若 `.env` 显式写了 `EVOLUTION_ENABLED=false`，升级后会变成"硬锁定、UI 开不了"——需部署时手动删掉那行**，让 env 回落默认 true（允许），再由 UI 总开关控制实际开关。

## 不做（YAGNI 边界）

- 不动 worker 隔离红线、不碰 `threshold_overrides` 主链路消费逻辑。
- 不新建通用 `settings`/`feature_flag` 集合（项目是"每功能各自一表/一字段"的分散模式，YAGNI）。
- 不动灰度哈希分桶算法 / auto-release 子闸 / prompt 自优化通道。
- 不扩展多租户：flag 仍按 `admin.current_workspace` 单 workspace 隔离。
- 不动 `/api/health` 的 `evolutionEnabled` 字段（监控探活仍用，前端只是不再依赖它做门控）。

## 命名红线

新增行（"演化中心总开关"/"运维硬锁定"/"已关闭，打开后开始自动产出"等文案、注释、测试文案）不得含 CI 禁词 `人工接管/人工介入/人工托管/接管/人工/takeover/hand-off`。本设计用词均合规。
