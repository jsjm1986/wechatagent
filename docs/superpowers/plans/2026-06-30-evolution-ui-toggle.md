# 演化中心 UI 开关 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让管理员在 UI 里一键开关演化中心，运行时即时生效不重启；env `EVOLUTION_ENABLED` 降为生产硬上限，mongo `evolution_runtime_flags.enabled` 升为总开关。

**Architecture:** 后端实质改动极小——`EVOLUTION_ENABLED` 默认值 false→true（语义改"是否允许 UI 开启"）、worker early-return 措辞改"硬锁定"语义（逻辑不变）。总开关全链路复用已存在的 `/api/evolution/runtime-flag` GET/PUT + `evolution_runtime_flags` 集合 + tick 内灰度门。前端把 `EvolutionCenterTab` 从"env 死占位 + 埋藏灰度控件"重构为三态（运维硬锁 / 可点开关关态 / 开态加载数据），数据源收敛到单次 `GET /api/evolution/runtime-flag`（含 `envEvolutionEnabled`）。

**Tech Stack:** Rust 2021 / Axum / MongoDB；前端 React 19 + TS + Vite + Zustand，vitest（`--pool=forks`）。

## Global Constraints

- env `EVOLUTION_ENABLED` 语义＝"是否允许在 UI 开启演化中心"，默认 `true`（允许）；仅显式 `false` 为运维硬锁定。复用原变量，不新增。
- "开=全量"：UI 总开关 on 时 PUT 写 `{enabled:true, rolloutPercent:100}`；off 写 `{enabled:false}`（rollout 值保留）。
- 前端单数据源：`EvolutionCenterTab` 挂载时一次 `GET /api/evolution/runtime-flag` 拿 `{envEvolutionEnabled, flag}` 推三态；`index.tsx` 不再 gate `/api/health`。
- `EvolutionCenterTab` 的 `enabled` prop **保留不删**（7 处既有测试依赖），语义重定义为"env 硬上限覆盖"：`locked = !enabled || envAllowed === false`。
- 既有测试只增量叠加，不删旧维度；`evolution.test.tsx` 因 `index.tsx` 不再取 health 需同步更新（接口变更同步，非删维度）。
- 后端 GET/PUT `/api/evolution/runtime-flag` 契约不变；不动 worker 隔离红线、不碰 `threshold_overrides` 主链路消费、不新建通用 settings 集合、不动灰度哈希分桶/auto-release 子闸/prompt 通道、不动 `/api/health` 字段。
- CI 基线门：`cargo test --lib` ≥350/0；`RUSTFLAGS="-D warnings" cargo check --tests` EXIT=0。
- 命名红线（CI 硬门）：新增行不得含 `人工接管/人工介入/人工托管/接管/人工/takeover/hand-off`。
- 本 worktree 路径含非 ASCII（工作项目）：vitest 用 `--pool=forks`；`cargo test --lib` 前 `touch src/lib.rs` 强制 relink 避共享 target stale 二进制。

---

## File Structure

- `src/config.rs` — `EVOLUTION_ENABLED` 默认值 + config 默认测试（Task 1）。
- `.env.example` — env 注释语义同步（Task 1）。
- `src/evolution/mod.rs` — worker early-return 措辞 + 模块头注释（Task 2，纯注释/日志）。
- `frontend/src/features/evolution/EvolutionCenterTab.tsx` — 三态重构主体（Task 3）。
- `frontend/src/features/evolution/index.tsx` — 去掉 health 门控（Task 3）。
- `frontend/src/__tests__/EvolutionCenterTab.test.tsx` / `frontend/src/__tests__/features/evolution/evolution.test.tsx` — 前端测试（Task 3 + Task 4）。
- 收口回归 + 双 lint（Task 5）。

---

### Task 1: `EVOLUTION_ENABLED` 默认 false→true + config 默认测试

**Files:**
- Modify: `src/config.rs:584`（默认串）、`src/config.rs:193-194`（字段注释）、`src/config.rs:722-743`（测试 mod）
- Modify: `.env.example:135`（env 注释）

**Interfaces:**
- Produces: `config.evolution_enabled` 缺省语义＝true（允许 UI 开启）。Task 2 的 worker early-return 仍读它，语义变"硬上限"。

- [ ] **Step 1: 写失败测试**

在 `src/config.rs` 的 `mod tests`（`:722`）内、`media_config_defaults` 测试之后新增（沿用该文件"断言 `env_or(key,default)` 默认串、不动进程 env"的模式）：

```rust
    /// EVOLUTION_ENABLED 默认串＝"true"（语义：默认允许在 UI 开启演化中心；
    /// 设 false 为运维硬锁定）。直接断言 from_env 用的同款 env_or 默认串解析结果。
    #[test]
    fn evolution_enabled_defaults_to_true() {
        // 未设环境变量时，默认串经 parse_bool 解析为 true。
        assert!(parse_bool(&env_or("EVOLUTION_ENABLED_UNSET_XYZ", "true")));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib evolution_enabled_defaults_to_true 2>&1 | tail -12`
Expected: 编译失败或断言失败——此刻 `:584` 仍是 `"false"`，但本测试断言的是新默认串 `"true"`。注意：本测试用的是**字面量 `"true"`** 不读 `:584`，所以它本身会**通过**（测的是 parse_bool 行为）。**先改测试为断言 `:584` 真实默认**——见修正下方。

> **修正（避免假测试）**：本测试若只断言字面量 `"true"` 则恒真、不锁 `:584`。改为断言生产构造结果。把上面的测试体替换为构造一个未设 env 的 `AppConfig` 并断言其 `evolution_enabled`。但 `from_env` 读全局 env 有并行污染风险（该文件注释明说）。故采用**两段保险**：①保留上面 parse_bool 行为测试；②新增一条直接读源码常量的守卫——在 `:584` 抽出默认串为具名常量再断言。具体见 Step 3。

- [ ] **Step 3: 写最小实现（抽默认串为常量 + 改默认 + 注释）**

3a. 改 `src/config.rs:584`，把内联默认串抽成模块级常量并改为 `"true"`。在 `:584` 处：

```rust
            evolution_enabled: parse_bool(&env_or("EVOLUTION_ENABLED", EVOLUTION_ENABLED_DEFAULT)),
```

在 `src/config.rs` 顶部（其它 `const` / `use` 之后，结构体定义之前合适位置）新增：

```rust
/// EVOLUTION_ENABLED 默认串。语义：是否允许在 UI 开启演化中心（默认允许）；
/// 设 "false" 为运维硬锁定（紧急熔断，无需 mongo 写权限）。
pub(crate) const EVOLUTION_ENABLED_DEFAULT: &str = "true";
```

3b. 改字段注释 `src/config.rs:193-194`：

```rust
    /// 是否允许在 UI 开启演化中心（runtime flag 总开关的硬上限）。默认 true（允许）；
    /// 设 false 为运维硬锁定——worker 不进 tick、UI 总开关锁定。
    pub evolution_enabled: bool,
```

3c. 把 Step 1 的测试体改为锁定常量（替换为）：

```rust
    /// EVOLUTION_ENABLED 默认＝允许（true）。锁定 EVOLUTION_ENABLED_DEFAULT 常量值，
    /// 防止有人把默认改回 "false" 而不更新部署文档（语义：默认允许 UI 开启演化中心）。
    #[test]
    fn evolution_enabled_defaults_to_true() {
        assert_eq!(EVOLUTION_ENABLED_DEFAULT, "true");
        assert!(parse_bool(EVOLUTION_ENABLED_DEFAULT));
    }
```

3d. 改 `.env.example:135`（把原 `EVOLUTION_ENABLED=false` 行及其注释改为）：

```
# 是否允许在 UI 开启演化中心（演化中心总开关的硬上限）。默认 true=允许（实际开关由 UI 控制，
# 出厂仍关——因 evolution_runtime_flags 默认无文档=关）。设 false 为运维硬锁定（紧急熔断、UI 开不了）。
EVOLUTION_ENABLED=true
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo test --lib evolution_enabled_defaults_to_true 2>&1 | tail -8`
Expected: `test result: ok. 1 passed`。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/config.rs .env.example
git commit -m "feat(evolution-toggle): EVOLUTION_ENABLED 默认 false→true(降为生产硬上限)+常量+默认测试

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: worker early-return 措辞改"硬锁定"语义（纯注释/日志，逻辑不变）

**Files:**
- Modify: `src/evolution/mod.rs:8-9`（模块头注释）、`src/evolution/mod.rs:47`（函数 doc）、`src/evolution/mod.rs:52-55`（early-return 日志）

**Interfaces:**
- Consumes: `config.evolution_enabled`（Task 1 语义已变"硬上限"）。
- Produces: 无新接口。逻辑严格不变：env=false 仍 early-return 不进循环；env=true 进常驻 tick 循环（tick 内 mongo flag 门 `:104-115` 已存在）。

- [ ] **Step 1: 改模块头注释（`:8-9`）**

把：

```rust
//! 主循环 [`run_evolutionary_worker`] 由 `main.rs` 在 `EVOLUTION_ENABLED=true`
//! 时 spawn；关闭时 worker 直接 return，不影响主进程。波次落地节奏：
```

改为：

```rust
//! 主循环 [`run_evolutionary_worker`] 由 `main.rs` 无条件 spawn；`EVOLUTION_ENABLED`
//! 是硬上限——为 false（运维硬锁定）时 worker 进函数即 return，不进 tick；为 true
//! 时进常驻 tick 循环，每 tick 内由 mongo runtime flag 决定是否真选 cohort。波次落地节奏：
```

- [ ] **Step 2: 改函数 doc（`:47`）**

把：

```rust
/// 演化器主循环。`EVOLUTION_ENABLED=false` 时立即 return，等价于功能未启用。
```

改为：

```rust
/// 演化器主循环。`EVOLUTION_ENABLED=false`（运维硬锁定）时立即 return；为 true 时
/// 进常驻 tick 循环，实际是否产出由 mongo runtime flag（UI 总开关）每 tick 决定。
```

- [ ] **Step 3: 改 early-return 日志（`:52-55`）**

把：

```rust
    if !state.config.evolution_enabled {
        tracing::info!("evolution worker disabled (EVOLUTION_ENABLED=false); skip spawn");
        return;
    }
```

改为：

```rust
    if !state.config.evolution_enabled {
        tracing::info!("evolution worker hard-locked (EVOLUTION_ENABLED=false); not entering tick loop");
        return;
    }
```

- [ ] **Step 4: 编译确认（纯注释/字符串，应无影响）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && cargo check --lib 2>&1 | tail -3`
Expected: `Finished`，0 error。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add src/evolution/mod.rs
git commit -m "docs(evolution-toggle): worker early-return 措辞改硬锁定语义(逻辑不变)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 前端 `RuntimeFlagResponse` 扩 envEvolutionEnabled + Tab 三态重构 + index 去 health 门控

**Files:**
- Modify: `frontend/src/features/evolution/EvolutionCenterTab.tsx:96-98`（接口）、`:155`（签名保留）、`:162-200`（flag state + loadFlag/saveFlag）、`:224-249`（load gate + 三态渲染）、`:272-306`（面板 relabel + 高级折叠）
- Modify: `frontend/src/features/evolution/index.tsx`（去 health fetch）

**Interfaces:**
- Consumes: `GET /api/evolution/runtime-flag` → `{ workspaceId, envEvolutionEnabled: boolean, flag: { enabled, rolloutPercent } | null }`（后端 `evolution.rs:579-583` 已返回，前端类型本任务补齐）。
- Produces: 三态渲染 + toggle 即时 PUT。

- [ ] **Step 1: 扩 `RuntimeFlagResponse` 接口（`:96-98`）**

把：

```tsx
export interface RuntimeFlagResponse {
  flag: RuntimeFlag | null;
}
```

改为（补 `envEvolutionEnabled`，后端 GET 已返回但前端类型缺）：

```tsx
export interface RuntimeFlagResponse {
  // GET 返回；PUT 响应无此字段，故可选。true=env 允许 UI 开启；false=运维硬锁定。
  envEvolutionEnabled?: boolean;
  flag: RuntimeFlag | null;
}
```

- [ ] **Step 2: 加 envAllowed state + loadFlag 解析它（`:162-180`）**

在 `:162-165` 的 flag state 区新增一行：

```tsx
  const [envAllowed, setEnvAllowed] = useState<boolean | null>(null);
```

把 `loadFlag`（`:167-180`）的 try 体改为（新增读 `envEvolutionEnabled`）：

```tsx
    try {
      const resp = await apiGet<RuntimeFlagResponse>("/api/evolution/runtime-flag");
      setEnvAllowed(resp.envEvolutionEnabled !== false); // 缺省按允许；显式 false 才硬锁
      setFlagEnabled(Boolean(resp.flag?.enabled ?? false));
      setRollout(String(resp.flag?.rolloutPercent ?? 0));
    } catch (e) {
      setFlagMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setFlagBusy(false);
    }
```

- [ ] **Step 3: 挂载即 loadFlag + load gate 改 flagEnabled（`:224-241`）**

把 `load()`（`:224-236`）的 gate 行 `if (!enabled) return;` 改为：

```tsx
  async function load() {
    if (!enabled || envAllowed === false || !flagEnabled) return;
```

把 useEffect（`:238-241`）改为挂载时拉 flag，并在 flagEnabled 变化时加载数据：

```tsx
  useEffect(() => {
    void loadFlag();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, envAllowed, flagEnabled]);
```

- [ ] **Step 4: 三态渲染（替换 `:243-249` 的死占位）**

把：

```tsx
  if (!enabled) {
    return (
      <div className={styles.disabled} data-testid="evolution-disabled">
        演化器未启用（EVOLUTION_ENABLED=false）。启用后此处会展示自动产出的实验信封与候选。
      </div>
    );
  }
```

替换为（locked = prop 或 env 任一锁定；首屏 flag 未回显加载中）：

```tsx
  const locked = !enabled || envAllowed === false;
  if (locked) {
    return (
      <div className={styles.disabled} data-testid="evolution-disabled">
        演化中心已被运维硬锁定（EVOLUTION_ENABLED=false），请联系运维解除后再在此开启。
      </div>
    );
  }
  if (envAllowed === null) {
    return (
      <div className={styles.disabled} data-testid="evolution-flag-loading">
        加载中…
      </div>
    );
  }
```

- [ ] **Step 5: runtime-flag 面板 relabel 为总开关 + saveFlag 开=100（`:182-200` saveFlag + `:272-306` 面板）**

5a. 改 `saveFlag`（`:182-200`）让"开=全量 100"：把 try 体的 pct 计算与 PUT body 改为：

```tsx
    try {
      // 开=全量：enabled 时若高级灰度值为 0 则按 100 全量；关时保留原 rollout 值。
      const advanced = Math.max(0, Math.min(100, Number(rollout) || 0));
      const pct = flagEnabled ? (advanced === 0 ? 100 : advanced) : advanced;
      const resp = await apiPut<RuntimeFlagResponse>("/api/evolution/runtime-flag", {
        enabled: flagEnabled,
        rolloutPercent: pct,
      });
      setFlagEnabled(Boolean(resp.flag?.enabled ?? flagEnabled));
      setRollout(String(resp.flag?.rolloutPercent ?? pct));
      setFlagMsg("演化中心总开关已保存");
    } catch (e) {
      setFlagMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setFlagBusy(false);
    }
```

5b. 改面板文案（`:281` 的 `<span>启用演化灰度</span>`）为总开关语义：

```tsx
            <span>演化中心总开关</span>
```

5c. 把"灰度比例"label（`:283-293`）包进可折叠"高级设置"。在面板 `<div className={styles.flagRow}>` 内，把灰度比例 `<label>` 整块替换为：

```tsx
          <details className={styles.advanced}>
            <summary>高级设置（灰度比例）</summary>
            <label className={styles.flagField}>
              <span>灰度比例（%）</span>
              <input
                type="number"
                min={0}
                max={100}
                value={rollout}
                onChange={(e) => setRollout(e.target.value)}
                disabled={flagBusy}
              />
            </label>
          </details>
```

5d. checkbox onChange（`:276-280`）保持绑 `flagEnabled`，但改为切换后立即保存（总开关直觉）。把 checkbox 的 `onChange` 改为：

```tsx
              onChange={(e) => {
                setFlagEnabled(e.target.checked);
                // 状态更新后保存：用新值直接 PUT，避免读到旧 state。
                void saveFlagWith(e.target.checked);
              }}
```

并在 `saveFlag` 之后新增一个接受显式 enabled 的封装（解决 setState 异步读旧值）：

```tsx
  async function saveFlagWith(nextEnabled: boolean) {
    setFlagBusy(true);
    setFlagMsg("");
    try {
      const advanced = Math.max(0, Math.min(100, Number(rollout) || 0));
      const pct = nextEnabled ? (advanced === 0 ? 100 : advanced) : advanced;
      const resp = await apiPut<RuntimeFlagResponse>("/api/evolution/runtime-flag", {
        enabled: nextEnabled,
        rolloutPercent: pct,
      });
      setFlagEnabled(Boolean(resp.flag?.enabled ?? nextEnabled));
      setRollout(String(resp.flag?.rolloutPercent ?? pct));
      setFlagMsg("演化中心总开关已保存");
    } catch (e) {
      setFlagMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setFlagBusy(false);
    }
  }
```

> 注：`saveFlag()`（无参，供"保存灰度"按钮用）保留不动，读当前 `flagEnabled` state；`saveFlagWith(next)` 供 toggle 即时保存用。两者并存。

- [ ] **Step 6: `index.tsx` 去 health 门控**

把 `frontend/src/features/evolution/index.tsx` 整个组件体（`:8-46`）替换为（删 health fetch + enabled state，直接渲染 Tab，不传 `enabled` prop → 走默认 true，env 锁定态由 Tab 内 loadFlag 的 envAllowed 驱动）：

```tsx
export default function EvolutionFeature() {
  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.panelHead}>
          <div className={styles.panelHeadL}>
            <span className={styles.eyebrow}>Self Evolution</span>
            <span className={styles.title}>实验信封 · 候选 · Shadow 评测</span>
          </div>
          <div className={styles.headIcon}>
            <ShieldCheck size={18} />
          </div>
        </div>
        <EvolutionCenterTab />
      </section>
    </div>
  );
}
```

删掉文件顶部不再使用的 `useEffect, useState` import（`:1`）——改为 `import { ShieldCheck } from "lucide-react";` + `import { EvolutionCenterTab } from "./EvolutionCenterTab";` + `import styles from "./EvolutionCenterTab.module.css";`（去掉 react hooks import）。

- [ ] **Step 7: 编译确认（tsc）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution/frontend" && npx tsc --noEmit 2>&1 | grep "error TS" | head; echo "TSC_DONE"`
Expected: `TSC_DONE` 前无 `error TS`。

- [ ] **Step 8: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add frontend/src/features/evolution/EvolutionCenterTab.tsx frontend/src/features/evolution/index.tsx
git commit -m "feat(evolution-toggle-fe): Tab 三态重构(运维硬锁/总开关关态/开态)+收敛单数据源+开=全量

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 前端测试（既有同步 + 新增三态）

**Files:**
- Modify: `frontend/src/__tests__/features/evolution/evolution.test.tsx`（health mock 同步）
- Modify: `frontend/src/__tests__/EvolutionCenterTab.test.tsx`（新增三态测试）

**Interfaces:**
- Consumes: `EvolutionCenterTab`（Task 3 三态）、`EvolutionFeature`（Task 3 去 health）。

- [ ] **Step 1: 同步 `evolution.test.tsx`（index 不再取 health）**

`EvolutionFeature` 现在直接渲染 `EvolutionCenterTab`，后者挂载即 `GET /api/evolution/runtime-flag`。把 `evolution.test.tsx` 两条用例的 fetch mock 从"按 /api/health 分支"改为"按 runtime-flag + experiments 分支"。整文件 `:17-52` 两个 `it` 替换为：

```tsx
  it("演化中心开启时渲染聚合卡与候选列表区", async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/evolution/runtime-flag")) {
        return {
          ok: true,
          json: async () => ({ envEvolutionEnabled: true, flag: { enabled: true, rolloutPercent: 100 } }),
        } as Response;
      }
      // /api/evolution/experiments
      return { ok: true, json: async () => ({ items: [] }) } as Response;
    }) as typeof fetch;

    render(<EvolutionFeature />);

    expect(screen.getByText("实验信封 · 候选 · Shadow 评测")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("evolution-center")).toBeInTheDocument();
    });
    expect(screen.getByTestId("agg-experiments")).toBeInTheDocument();
    expect(screen.getByTestId("agg-significance")).toBeInTheDocument();
    expect(screen.getByTestId("proposal-list-empty")).toBeInTheDocument();
  });

  it("env 硬锁定时渲染锁定占位", async () => {
    globalThis.fetch = vi.fn(async () =>
      ({ ok: true, json: async () => ({ envEvolutionEnabled: false, flag: null }) }) as Response
    ) as typeof fetch;

    render(<EvolutionFeature />);

    await waitFor(() => {
      expect(screen.getByTestId("evolution-disabled")).toBeInTheDocument();
    });
  });
```

- [ ] **Step 2: 跑既有+同步测试确认通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution/frontend" && npx vitest run src/__tests__/features/evolution/evolution.test.tsx --pool=forks 2>&1 | tail -8`
Expected: 2 passed。

- [ ] **Step 3: `EvolutionCenterTab.test.tsx` 新增三态测试**

既有测试全部用 `<EvolutionCenterTab enabled={true} />` 且 mock `fetch`（`vi.stubGlobal("fetch", fetchMock)`）。既有用例第一个 fetch 是 experiments——但 Task 3 后 Tab 挂载**先**打 runtime-flag。**既有用例需在 experiments mock 前补一个 runtime-flag mock**，否则首个 `mockResolvedValueOnce` 会被 loadFlag 吃掉导致错位。

3a. 在 `describe` 顶部新增一个 helper（`:123` after-each 之后）：

```tsx
  function mockRuntimeFlag(over?: { envEvolutionEnabled?: boolean; enabled?: boolean; rolloutPercent?: number }) {
    return {
      ok: true,
      json: async () => ({
        envEvolutionEnabled: over?.envEvolutionEnabled ?? true,
        flag: { enabled: over?.enabled ?? true, rolloutPercent: over?.rolloutPercent ?? 100 },
      }),
    };
  }
```

3b. 既有每个 `it` 里，在第一个 `fetchMock.mockResolvedValueOnce({... experiments ...})` **之前**插入一行 `fetchMock.mockResolvedValueOnce(mockRuntimeFlag());`（runtime-flag 先于 experiments）。共 7 处 `render(<EvolutionCenterTab enabled={true} />)` 对应的 mock 序列各补一行。

> 实现者注意：逐个用例核对 mock 调用顺序——loadFlag 在 useEffect 挂载即触发，是**第一个** fetch；load(experiments) 是**第二个**。既有用例若依赖"第一个 mock=experiments"，全部要在前面插 runtime-flag mock。以 vitest 跑挂的报错定位（mock 错位会让 experiments 解析成 flag 形状 → 渲染异常）。

3c. 新增三态测试（在 `describe` 末尾 `}` 前）：

```tsx
  it("env 硬锁定（envEvolutionEnabled=false）渲染锁定占位", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ envEvolutionEnabled: false, enabled: false }));
    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => {
      expect(screen.getByTestId("evolution-disabled")).toHaveTextContent("运维硬锁定");
    });
  });

  it("env 允许但 flag 关时显示可点总开关、不加载实验数据", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ envEvolutionEnabled: true, enabled: false, rolloutPercent: 0 }));
    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => {
      expect(screen.getByTestId("runtime-flag-panel")).toBeInTheDocument();
    });
    // flag 关 → 不应加载实验聚合卡
    expect(screen.queryByTestId("agg-experiments")).toBeNull();
    // 总开关 checkbox 可点（未 disabled）
    const toggle = screen.getByText("演化中心总开关").closest("label")?.querySelector("input");
    expect(toggle).not.toBeNull();
    expect(toggle).not.toBeDisabled();
  });

  it("打开总开关 PUT 写 enabled:true + rolloutPercent:100", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ envEvolutionEnabled: true, enabled: false, rolloutPercent: 0 }));
    // PUT 响应
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ ok: true, flag: { enabled: true, rolloutPercent: 100 } }),
    });
    // 打开后会触发 load(experiments)
    fetchMock.mockResolvedValueOnce({ ok: true, json: async () => ({ items: [] }) });

    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => screen.getByText("演化中心总开关"));
    const toggle = screen.getByText("演化中心总开关").closest("label")!.querySelector("input")!;
    fireEvent.click(toggle);

    await waitFor(() => {
      const putCall = fetchMock.mock.calls.find(
        (c) => String(c[0]).includes("/api/evolution/runtime-flag") && c[1]?.method === "PUT",
      );
      expect(putCall).toBeTruthy();
      const body = JSON.parse((putCall![1] as RequestInit).body as string);
      expect(body.enabled).toBe(true);
      expect(body.rolloutPercent).toBe(100);
    });
  });
```

> 实现者注意：PUT body 断言依赖 `apiPut` 用 `fetch(url, {method:"PUT", body: JSON.stringify(...)})`。若 `apiPut` 封装形态不同（如 body 不是 JSON string），以实际 `apiClient` 实现调整断言取值方式——先读 `frontend/src/` 里 `apiPut` 定义确认。

- [ ] **Step 4: 跑全部前端测试确认通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution/frontend" && npx vitest run src/__tests__/EvolutionCenterTab.test.tsx src/__tests__/features/evolution/evolution.test.tsx --pool=forks 2>&1 | tail -12`
Expected: 全部 passed（既有 + 新增三态 + 同步用例）。

- [ ] **Step 5: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution"
git add frontend/src/__tests__/EvolutionCenterTab.test.tsx frontend/src/__tests__/features/evolution/evolution.test.tsx
git commit -m "test(evolution-toggle-fe): 既有用例补 runtime-flag mock+新增三态/开=100 测试

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 收口回归 + 双 lint

**Files:** 无代码改动，仅验证。

- [ ] **Step 1: 全量 lib 测试（touch 强制 relink）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && touch src/lib.rs && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥350 passed / 0 failed。

- [ ] **Step 2: dead-code 门**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && RUSTFLAGS="-D warnings" cargo check --tests 2>&1 | tail -3`
Expected: `Finished`，EXIT=0。

- [ ] **Step 3: 前端 build**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution/frontend" && npm run build 2>&1 | tail -5`
Expected: `built in` 成功。

- [ ] **Step 4: 双 lint**

Run:
```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/prompt-evolution" && bash scripts/check-no-human-takeover.sh 2>&1 | tail -3 && echo "HITS=$(git diff origin/main...HEAD -- 'src/**' 'frontend/**' '.env.example' | grep -E '^\+' | grep -cE '人工接管|人工介入|人工托管|takeover|hand[ -]?off|接管|人工')"
```
Expected: `0 violations`；`HITS=0`。

- [ ] **Step 5: 无代码改动，本 Task 不提交**（验证-only）

---

## Self-Review

**1. Spec coverage:**
- K1/K2 env 降硬上限、复用 EVOLUTION_ENABLED 默认 true → Task 1 ✅
- worker 常驻循环语义（逻辑不变只改措辞）→ Task 2 ✅
- K3 开=全量 100 → Task 3 Step 5（saveFlag/saveFlagWith pct=100）+ Task 4 PUT 断言 ✅
- K4 面板 relabel 总开关 + 常驻可见 → Task 3 Step 5（relabel）+ Step 4（三态移出死占位）✅
- K5 单数据源（Tab 拉 runtime-flag 含 envEvolutionEnabled，index 去 health）→ Task 3 Step 1-3/6 ✅
- K6 enabled prop 保留 + locked 复合判定 → Task 3 Step 4 ✅
- 测试既有同步 + 新增三态 → Task 4 ✅
- 部署注意（生产 .env 显式 false 会硬锁）→ spec 已记，无需代码任务 ✅
- 回归 + 双 lint → Task 5 ✅

**2. Placeholder scan:** 无 TBD/TODO；每个 code step 有完整代码。两处"实现者注意"（Task 4 Step 3b mock 错位定位、Step 3c apiPut body 形态）是**对既有代码不确定点的核对指引**，非占位——指明要先读 `apiPut` 实现/逐用例核 mock 顺序，是真实可执行的核对动作。

**3. Type consistency:**
- `RuntimeFlagResponse { envEvolutionEnabled?: boolean; flag: RuntimeFlag|null }`（Task 3 Step 1 定义）→ Task 3 Step 2 loadFlag 读 `resp.envEvolutionEnabled`、Task 4 mock 返回它 ✅
- `EVOLUTION_ENABLED_DEFAULT: &str`（Task 1 Step 3a 定义）→ Task 1 Step 3c 测试断言 ✅
- `saveFlagWith(nextEnabled: boolean)`（Task 3 Step 5d 定义）→ Task 3 Step 5d toggle onChange 调用 ✅
- `locked = !enabled || envAllowed === false`（Task 3 Step 4）→ Task 3 Step 3 load gate 同条件 ✅
- testid `evolution-disabled`（既有，三态锁定态复用）/`runtime-flag-panel`（既有）/`evolution-flag-loading`（新增）一致 ✅

**4. 风险点（实现者注意）：**
- **最高危＝Task 4 Step 3b**：Tab 挂载后 fetch 顺序变成 runtime-flag 先、experiments 后，既有 7 用例的 `mockResolvedValueOnce` 序列必须在最前补一个 runtime-flag mock，否则错位。vitest 跑挂的报错（experiments 解析成 flag 形状）是定位信号。
- Task 3 Step 5d：toggle 即时保存用 `saveFlagWith(next)` 传显式值，规避 React setState 异步读旧值。
- Task 3 Step 6：删 `index.tsx` 的 `useEffect/useState` import 后须确认无残留引用（tsc 会报 unused）。
