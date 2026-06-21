# Ask-Human 请示通道配置页（Phase 3）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建一个独立的 `askHumanConfig`「请示通道配置」前端频道，让管理员可视化配置 `AskHumanPolicy`（决策人链 + 4 escalate 开关 + 超时 + 推送频控），消费已有 `PUT .../ask-human-policy` 端点，补后端回显。

**Architecture:** 单页单表单单提交：频道入口 `AskHumanConfigView` 用局部 `useState` 持整个 `AskHumanPolicy` 草稿，mount 时 `GET /operation-domains/user_operations` 读现状回显（补后端序列化 askHumanPolicy），编辑改本地 state，「保存」一次 `PUT` 整体提交。决策人链是受控子组件 `DeciderChainEditor`（复用 ContactPicker 模式）。校验/默认值/抽取逻辑抽 `policyForm.ts` 纯函数（最高价值单测层）。不建 Zustand store（无跨组件共享状态）。

**Tech Stack:** Rust(Axum) 后端 + React 19 + TypeScript + Vite 前端；CSS Modules + tokens.css 变量；vitest（前端单测）+ cargo test（后端）。

## Global Constraints

设计依据：`docs/superpowers/specs/2026-06-21-ask-human-config-page-design.md`（已获批）。每个 task 的要求都隐含包含本节。

- **遵守现有设计系统（用户硬要求）**：CSS 用 `AskHumanConfig.module.css` + `import styles from` 绑定导入 + `className={styles.xxx}`（对齐 8 个主流频道；**绝不**裸副作用导入 `import "./x.module.css"`——那才触发 Rollup tree-shake 整份丢失）。一律走 `frontend/src/components/ui/tokens.css` 变量（文字 `--ink-1`/`--ink-2`/`--ink-3`、面 `--surface-card`/`--hairline`、圆角 `--r-sm`/`--r-md`/`--r-lg`、焦点 `--focus-ring`、状态色 `--color-scheduled`蓝仅关键可点击/`--color-brand`紫仅AI身份），禁裸色值/魔数。4 级层级（频道 header 标题+保存按钮右上 → 单栏 panel，禁嵌套 panel/禁 panel 内 card/禁第三级导航）。字号 page title 28-40px/panel title 18px/body 13px/metadata 10.5-12px。实现前读 `docs/frontend-design-system.md` 全文 + 参照 `features/products-deals/ProductsDeals.module.css`。
- **no-takeover lint**（扫 `frontend/src/` 新增行）：文案一律 AI 内部口径——「安全门拦截 / 产品声明未核验 / AI 策略主动暂缓 / 对话停滞 / 决策人 / 请示通道」，**绝不**出现 `转人工 / 人工接管 / 人工介入 / 接管 / takeover / hand-off`。验证：`bash scripts/check-no-human-takeover.sh`。
- **测试位置铁律**：`frontend/vitest.config.ts` 的 include 锁 `src/__tests__/**/*.test.{ts,tsx}`；前端测试必须落 `src/__tests__/` 镜像路径，否则 CI 静默跳过=假绿。
- **后端补缺向后兼容**：序列化 askHumanPolicy 是加字段（`config.ask_human_policy` 是 `Option<AskHumanPolicy>`，None 时不产出该键），不动 operation_domain_json 其他字段。
- **测试基线不回归**：`cargo test --lib` ≥350/0；四 PBT 累计 ≥33/0。
- **磁盘纪律**：后端编译前 `rm -rf target/debug/incremental` + `CARGO_INCREMENTAL=0`；本地只 `cargo test --lib` + 单 PBT，集成测试 `#[ignore]` 靠 CI。
- **共享工作树**：本分支与并行会话（referral-card / media-asset）交错；`git add` **精确具名**，**绝不** `git add -A`/`.`，排除并行会话产物（`.kiro/specs/universal-test-coverage/*`、`AGENTS.md`、`agent_t*.txt`、`t15_single.txt`、sales-media-asset docs）。
- **domain key**：固定 `user_operations`（P1 scope = 私聊运营）。
- **提交需用户授权**：subagent-driven 流程内逐 task commit 是授权的；push/PR 另需用户拍板。

## 实证基线（写码前已亲核，照此用）

- `AskHumanPolicy`（models.rs:825，`rename_all="camelCase"`）：`deciderChain: DeciderRef[]`、`escalateSafetyGuard`(默认true)、`escalateUnverifiedProduct`(true)、`escalateAiPolicyHold`(默认false)、`escalateStuck`(true)、`dedupeWindowHours?:number`、`dailyPushCap?:number`、`quietHours?:{startHour,endHour,tzOffsetHours}`、`timeoutHours?:number`。
- `DeciderRef`（models.rs:807）：`{ wxid: string, displayName?: string }`。
- `OperationDomainConfig`（models.rs，**无 rename_all** → 落库 snake_case `ask_human_policy`，但 `operation_domain_json` 手写 json! 转 camelCase 输出）。`ask_human_policy: Option<AskHumanPolicy>`（models.rs:917）。
- 读现状：`GET /api/admin/operation-domains/user_operations` → `{ item: {...} }`（domains.rs:86）。`operation_domain_json`（domains.rs:273-295）**当前不含 askHumanPolicy** → Task 1 补。
- 写策略：`PUT /api/admin/operation-domains/:domain/ask-human-policy` body=AskHumanPolicy（domains.rs:206）→ `{ok:true}`；当前版本不存在→404。后端已校验 wxid 非空 + quietHours 0-23。
- `GET /api/contacts?limit=100` → `{ items: Contact[] }`；`Contact`（types/index.ts:46）字段 `id/wxid/nickname?/remark?/alias?`（**无 name**）。显示名取 `nickname || remark || wxid`（ContactPicker products-deals 实证）。
- 前端基建：`api.get<T>(url)`/`api.post`/`api.put`（lib/api.ts，裸 fetch 收全路径，非 2xx 抛 parseApiError）；`useToast()→{success,error,info}(msg)`（components/ui/Toast）；`useConfirm()→async(opts)=>Promise<boolean>`（components/ui/ConfirmDialog）。
- 频道注册（channels.ts）：lazy import 区 + lucide 图标 import + `CHANNELS: ChannelDef[]` 数组项（`{id,group,label,caption,icon,eyebrow,title,subtitle,Component}`）；Shell.tsx `CHANNELS.find` 自动渲染，无第三处接线。

---

### Task 1: 后端补缺 — operation_domain_json 序列化 askHumanPolicy

**Files:**
- Modify: `src/routes/domains.rs:273-295`（`operation_domain_json` 加一行 askHumanPolicy 序列化）
- Test: `tests/ask_human_phase1_e2e.rs`（append 一个 `#[ignore]` 集成测试）+ `src/routes/domains.rs` 内联单测（若该文件已有 tests mod 则 append，否则加）

**Interfaces:**
- Consumes: `OperationDomainConfig.ask_human_policy: Option<AskHumanPolicy>`（models.rs:917）；`AskHumanPolicy` serde camelCase。
- Produces: `GET /operation-domains/:domain` 的 `item` 现在含 `askHumanPolicy`（None 时为 `null` 或缺省），前端 Task 3 `extractPolicy` 消费。

- [ ] **Step 1: 读现状（不改）**

读 `src/routes/domains.rs:273-295` 的 `operation_domain_json`（手写 `json!({...})`，camelCase key，当前最后几个字段是 version/currentVersion/previousVersion/seededBy）。确认它返回 `serde_json::Value`，逐字段手拼。`config.ask_human_policy` 是 `Option<AskHumanPolicy>`，`AskHumanPolicy` 自身 `#[serde(rename_all="camelCase")]`，故 `serde_json::to_value(&config.ask_human_policy)` 会产出 camelCase（None→`null`）。

- [ ] **Step 2: 写失败的集成测试**（append 到 `tests/ask_human_phase1_e2e.rs` 末尾）

```rust
#[tokio::test]
#[ignore]
async fn operation_domain_json_includes_ask_human_policy() {
    let app = common::TestApp::start().await;
    let ws = &app.state.config.default_workspace_id;
    // 给 user_operations 当前版本写一条 ask_human_policy。
    let policy = wechatagent::models::AskHumanPolicy {
        decider_chain: vec![wechatagent::models::DeciderRef {
            wxid: "wxid_boss".into(),
            display_name: Some("老板".into()),
        }],
        escalate_safety_guard: true,
        escalate_unverified_product: true,
        escalate_ai_policy_hold: false,
        escalate_stuck: true,
        dedupe_window_hours: Some(6.0),
        daily_push_cap: Some(3),
        quiet_hours: None,
        timeout_hours: Some(24.0),
    };
    let policy_bson = mongodb::bson::to_bson(&policy).unwrap();
    app.state
        .db
        .operation_domain_configs()
        .update_one(
            mongodb::bson::doc! { "workspace_id": ws, "domain": "user_operations", "current_version": true },
            mongodb::bson::doc! { "$set": { "ask_human_policy": policy_bson } },
            None,
        )
        .await
        .unwrap();
    let resp = wechatagent::routes::domains::get_operation_domain(
        axum::extract::State(app.state.clone()),
        axum::Extension(test_admin(ws)),
        axum::extract::Path("user_operations".to_string()),
    )
    .await
    .unwrap();
    let body: serde_json::Value = resp.0;
    assert_eq!(body["item"]["askHumanPolicy"]["deciderChain"][0]["wxid"], serde_json::json!("wxid_boss"));
    assert_eq!(body["item"]["askHumanPolicy"]["timeoutHours"], serde_json::json!(24.0));
    assert_eq!(body["item"]["askHumanPolicy"]["dailyPushCap"], serde_json::json!(3));
}
```
注：`get_operation_domain` 当前是 `pub(super)`（domains.rs:79）——若测试无法从 crate 外调用，Step 3 顺带把它改成 `pub`（与同模块 `put_ask_human_policy` 已 `pub` 一致；或在测试里走 HTTP。优先改 `pub`，最小改动）。`test_admin` helper 在该测试文件已存在（前序测试用过）。

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test --test ask_human_phase1_e2e operation_domain_json_includes_ask_human_policy -- --ignored --nocapture 2>&1 | tail -20`
Expected: 编译过但断言 FAIL（`body["item"]["askHumanPolicy"]` 是 `null`，因 operation_domain_json 没序列化该字段）。若本地无 Docker 无法跑，则 `cargo test --test ask_human_phase1_e2e --no-run` 确认编译过，断言失败留 CI 验证。

- [ ] **Step 4: 改 operation_domain_json 加序列化**

在 `src/routes/domains.rs` `operation_domain_json` 的 `json!({...})` 里，`"seededBy": config.seeded_by,` 之后加一行：
```rust
        "askHumanPolicy": config.ask_human_policy,
```
（`json!` 宏对 `Option<AskHumanPolicy>` 会用其 Serialize 实现 → camelCase 对象或 `null`。）若 Step 2 需要，把 `pub(super) async fn get_operation_domain` 改为 `pub async fn get_operation_domain`。

- [ ] **Step 5: 跑测试确认通过 + lib 基线**

Run:
```
cargo test --test ask_human_phase1_e2e operation_domain_json_includes_ask_human_policy -- --ignored --nocapture 2>&1 | tail -10
rm -rf target/debug/incremental; CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5
```
Expected: 集成测试 PASS（或本地无 Docker 时 `--no-run` 编译过）；lib ≥350/0 无回归。

- [ ] **Step 6: Commit**

```bash
git add src/routes/domains.rs tests/ask_human_phase1_e2e.rs
git commit -m "feat(ask-human): operation_domain_json 序列化 askHumanPolicy(P3 配置页回显)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 前端 types 补 AskHumanPolicy / DeciderRef / AskHumanQuietHours

**Files:**
- Modify: `frontend/src/types/index.ts`（新增三个 type 导出）

**Interfaces:**
- Produces: `AskHumanPolicy`、`DeciderRef`、`AskHumanQuietHours` TypeScript type（camelCase，对齐后端 serde），供 Task 3 policyForm / Task 4 DeciderChainEditor / Task 5 View import。

- [ ] **Step 1: 读现状（不改）**

读 `frontend/src/types/index.ts`：确认无 `AskHumanPolicy`/`DeciderRef`（仅有不相关的 `QuietHoursMode`，prop 名 `quiet_hours`，**不复用**）。找一个已有 type 块（如 `Contact` :46 或 `DomainProfile` :521）末尾作为插入锚点。

- [ ] **Step 2: 加三个 type（无测试，纯类型声明；Task 3 的 tsc 会校验）**

在 `frontend/src/types/index.ts` 末尾（或紧邻其他 ask-human 相关 type 处）加：
```ts
// 请示通道策略（对齐后端 models.rs AskHumanPolicy，camelCase serde）。P3 配置页 + P2 收件箱共用。
export type DeciderRef = {
  wxid: string;
  displayName?: string;
};

export type AskHumanQuietHours = {
  startHour: number;   // 0-23
  endHour: number;     // 0-23
  tzOffsetHours: number;
};

export type AskHumanPolicy = {
  deciderChain: DeciderRef[];
  escalateSafetyGuard: boolean;
  escalateUnverifiedProduct: boolean;
  escalateAiPolicyHold: boolean;
  escalateStuck: boolean;
  dedupeWindowHours?: number;
  dailyPushCap?: number;
  quietHours?: AskHumanQuietHours;
  timeoutHours?: number;
};
```

- [ ] **Step 3: 类型检查**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -10`
Expected: 0 type errors（纯新增导出，不影响现有）。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/types/index.ts
git commit -m "feat(ask-human): 前端 types 补 AskHumanPolicy/DeciderRef/AskHumanQuietHours

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: policyForm.ts 纯函数 + vitest（最高价值层）

**Files:**
- Create: `frontend/src/features/ask-human-config/policyForm.ts`
- Test: `frontend/src/__tests__/features/ask-human-config/policyForm.test.ts`（import 深度 3 级：`../../../features/ask-human-config/policyForm`）

**Interfaces:**
- Consumes: `AskHumanPolicy`/`DeciderRef`/`AskHumanQuietHours`（Task 2，policyForm.ts 在 `features/ask-human-config/` 下，import `../../types`）。
- Produces: `defaultPolicy(): AskHumanPolicy`、`extractPolicy(domainItem: unknown): AskHumanPolicy`、`validatePolicy(p: AskHumanPolicy): string[]`，供 Task 5 View import。

- [ ] **Step 1: 写失败的测试** — Create `frontend/src/__tests__/features/ask-human-config/policyForm.test.ts`

```ts
import { describe, it, expect } from "vitest";
import { defaultPolicy, extractPolicy, validatePolicy } from "../../../features/ask-human-config/policyForm";

type PolicyLike = Record<string, unknown>;

describe("defaultPolicy", () => {
  it("空链 + 保守默认开关(safety/product/stuck=true, aiPolicyHold=false), 可选项 undefined", () => {
    const p = defaultPolicy();
    expect(p.deciderChain).toEqual([]);
    expect(p.escalateSafetyGuard).toBe(true);
    expect(p.escalateUnverifiedProduct).toBe(true);
    expect(p.escalateAiPolicyHold).toBe(false);
    expect(p.escalateStuck).toBe(true);
    expect(p.timeoutHours).toBeUndefined();
    expect(p.dedupeWindowHours).toBeUndefined();
    expect(p.dailyPushCap).toBeUndefined();
    expect(p.quietHours).toBeUndefined();
  });
});

describe("extractPolicy", () => {
  it("完整 askHumanPolicy 原样抽出", () => {
    const item = { askHumanPolicy: {
      deciderChain: [{ wxid: "w1", displayName: "老板" }],
      escalateSafetyGuard: false, escalateUnverifiedProduct: true,
      escalateAiPolicyHold: true, escalateStuck: false,
      dedupeWindowHours: 6, dailyPushCap: 3,
      quietHours: { startHour: 22, endHour: 7, tzOffsetHours: 8 }, timeoutHours: 24,
    } };
    const p = extractPolicy(item);
    expect(p.deciderChain).toEqual([{ wxid: "w1", displayName: "老板" }]);
    expect(p.escalateSafetyGuard).toBe(false);
    expect(p.quietHours).toEqual({ startHour: 22, endHour: 7, tzOffsetHours: 8 });
    expect(p.timeoutHours).toBe(24);
  });
  it("askHumanPolicy 缺失/null/非对象 → 回落 defaultPolicy", () => {
    expect(extractPolicy({ askHumanPolicy: null })).toEqual(defaultPolicy());
    expect(extractPolicy({})).toEqual(defaultPolicy());
    expect(extractPolicy(null)).toEqual(defaultPolicy());
    expect(extractPolicy("garbage")).toEqual(defaultPolicy());
  });
  it("部分字段缺 → 缺的补默认, 有的保留", () => {
    const p = extractPolicy({ askHumanPolicy: { deciderChain: [{ wxid: "w1" }] } });
    expect(p.deciderChain).toEqual([{ wxid: "w1" }]);
    expect(p.escalateSafetyGuard).toBe(true);
    expect(p.escalateAiPolicyHold).toBe(false);
    expect(p.timeoutHours).toBeUndefined();
  });
});

describe("validatePolicy", () => {
  const ok: PolicyLike = {
    deciderChain: [{ wxid: "w1" }], escalateSafetyGuard: true, escalateUnverifiedProduct: true,
    escalateAiPolicyHold: false, escalateStuck: true,
  };
  it("合法策略 → 空错误数组", () => {
    expect(validatePolicy(ok as never)).toEqual([]);
  });
  it("空决策人链 → 报错", () => {
    expect(validatePolicy({ ...ok, deciderChain: [] } as never)).toContain("至少配置一个决策人");
  });
  it("决策人 wxid 空白 → 报错", () => {
    expect(validatePolicy({ ...ok, deciderChain: [{ wxid: "  " }] } as never).length).toBeGreaterThan(0);
  });
  it("quietHours 小时越界(>23) → 报错", () => {
    expect(validatePolicy({ ...ok, quietHours: { startHour: 24, endHour: 7, tzOffsetHours: 8 } } as never).length).toBeGreaterThan(0);
  });
  it("dedupeWindowHours / timeoutHours 负数 → 报错", () => {
    expect(validatePolicy({ ...ok, dedupeWindowHours: -1 } as never).length).toBeGreaterThan(0);
    expect(validatePolicy({ ...ok, timeoutHours: -5 } as never).length).toBeGreaterThan(0);
  });
  it("dailyPushCap < 1 → 报错", () => {
    expect(validatePolicy({ ...ok, dailyPushCap: 0 } as never).length).toBeGreaterThan(0);
  });
});
```
注：`PolicyLike = Record<string, unknown>` 仅测试内部造畸形输入用，断言时 `as never` 绕类型。生产 `validatePolicy` 形参是 `AskHumanPolicy`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/policyForm.test.ts 2>&1 | tail -15`
Expected: FAIL（policyForm 不存在）。

- [ ] **Step 3: 写 policyForm.ts** — Create `frontend/src/features/ask-human-config/policyForm.ts`

```ts
import type { AskHumanPolicy, DeciderRef, AskHumanQuietHours } from "../../types";

// 空链 + 保守默认开关（与后端 ResolvedAskHumanPolicy 非-all 模式回落一致）。
export function defaultPolicy(): AskHumanPolicy {
  return {
    deciderChain: [],
    escalateSafetyGuard: true,
    escalateUnverifiedProduct: true,
    escalateAiPolicyHold: false,
    escalateStuck: true,
  };
}

function asBool(v: unknown, fallback: boolean): boolean {
  return typeof v === "boolean" ? v : fallback;
}
function asNumOrUndef(v: unknown): number | undefined {
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

// 从 GET /operation-domains/:domain 的 item.askHumanPolicy 抽策略；缺/非对象 → defaultPolicy()。
// 逐字段存在性回落，保证返回结构完整可编辑。
export function extractPolicy(domainItem: unknown): AskHumanPolicy {
  const raw =
    domainItem && typeof domainItem === "object"
      ? (domainItem as Record<string, unknown>).askHumanPolicy
      : null;
  if (!raw || typeof raw !== "object") return defaultPolicy();
  const p = raw as Record<string, unknown>;
  const d = defaultPolicy();
  const chain: DeciderRef[] = Array.isArray(p.deciderChain)
    ? (p.deciderChain as unknown[]).flatMap((it) => {
        if (!it || typeof it !== "object") return [];
        const wxid = (it as Record<string, unknown>).wxid;
        if (typeof wxid !== "string") return [];
        const dn = (it as Record<string, unknown>).displayName;
        return [{ wxid, ...(typeof dn === "string" ? { displayName: dn } : {}) }];
      })
    : [];
  let quietHours: AskHumanQuietHours | undefined;
  const qhRaw = p.quietHours;
  if (qhRaw && typeof qhRaw === "object") {
    const q = qhRaw as Record<string, unknown>;
    if (typeof q.startHour === "number" && typeof q.endHour === "number" && typeof q.tzOffsetHours === "number") {
      quietHours = { startHour: q.startHour, endHour: q.endHour, tzOffsetHours: q.tzOffsetHours };
    }
  }
  const dedupe = asNumOrUndef(p.dedupeWindowHours);
  const cap = asNumOrUndef(p.dailyPushCap);
  const timeout = asNumOrUndef(p.timeoutHours);
  return {
    deciderChain: chain,
    escalateSafetyGuard: asBool(p.escalateSafetyGuard, d.escalateSafetyGuard),
    escalateUnverifiedProduct: asBool(p.escalateUnverifiedProduct, d.escalateUnverifiedProduct),
    escalateAiPolicyHold: asBool(p.escalateAiPolicyHold, d.escalateAiPolicyHold),
    escalateStuck: asBool(p.escalateStuck, d.escalateStuck),
    ...(dedupe !== undefined ? { dedupeWindowHours: dedupe } : {}),
    ...(cap !== undefined ? { dailyPushCap: cap } : {}),
    ...(quietHours ? { quietHours } : {}),
    ...(timeout !== undefined ? { timeoutHours: timeout } : {}),
  };
}

// 校验草稿；返回错误消息数组（空 = 通过）。前端体验校验，后端是权威。
export function validatePolicy(p: AskHumanPolicy): string[] {
  const errs: string[] = [];
  if (!p.deciderChain || p.deciderChain.length === 0) {
    errs.push("至少配置一个决策人");
  }
  for (const d of p.deciderChain ?? []) {
    if (!d.wxid || d.wxid.trim().length === 0) {
      errs.push("决策人 wxid 不能为空");
      break;
    }
  }
  if (p.quietHours) {
    const { startHour, endHour } = p.quietHours;
    if (startHour < 0 || startHour > 23 || endHour < 0 || endHour > 23) {
      errs.push("静默时段小时须 0-23");
    }
  }
  if (p.dedupeWindowHours !== undefined && p.dedupeWindowHours < 0) errs.push("去重窗口不能为负");
  if (p.timeoutHours !== undefined && p.timeoutHours < 0) errs.push("超时小时不能为负");
  if (p.dailyPushCap !== undefined && p.dailyPushCap < 1) errs.push("每日上限至少为 1");
  return errs;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/policyForm.test.ts 2>&1 | tail -10 && npx vitest list 2>&1 | grep policyForm`
Expected: 全 PASS；`vitest list | grep policyForm` 非空（确认被收录，非假绿）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/ask-human-config/policyForm.ts frontend/src/__tests__/features/ask-human-config/policyForm.test.ts
git commit -m "feat(ask-human): P3 policyForm 纯函数(default/extract/validate)+vitest

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: DeciderChainEditor.tsx 受控组件 + 组件冒烟

**Files:**
- Create: `frontend/src/features/ask-human-config/DeciderChainEditor.tsx`
- Create: `frontend/src/features/ask-human-config/AskHumanConfig.module.css`（本 task 起样式文件，Task 5 续填）
- Test: `frontend/src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx`

**Interfaces:**
- Consumes: `DeciderRef`、`Contact`（from `../../types`）；`api.get`（from `../../lib/api`）。
- Produces: `DeciderChainEditor({ chain, onChange }: { chain: DeciderRef[]; onChange: (next: DeciderRef[]) => void }): JSX.Element`，供 Task 5 View 用。

- [ ] **Step 1: 写失败的测试** — Create `frontend/src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx`

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { DeciderChainEditor } from "../../../features/ask-human-config/DeciderChainEditor";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn() } }));

const CONTACTS = {
  items: [
    { id: "1", wxid: "wxid_a", nickname: "阿伟", agentStatus: "managed", tags: [] },
    { id: "2", wxid: "wxid_b", remark: "李总", agentStatus: "normal", tags: [] },
  ],
};

beforeEach(() => {
  (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(CONTACTS);
});

describe("DeciderChainEditor", () => {
  it("从联系人添加 → onChange 收到含 wxid+displayName 的新链", async () => {
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[]} onChange={onChange} />);
    fireEvent.click(screen.getByText(/从联系人添加/));
    await waitFor(() => screen.getByText("阿伟"));
    fireEvent.click(screen.getByText("阿伟"));
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_a", displayName: "阿伟" }]);
  });

  it("已在链中的 wxid 从候选排除", async () => {
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a", displayName: "阿伟" }]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从联系人添加/));
    await waitFor(() => screen.getByText("李总"));
    expect(screen.queryByText("阿伟")).toBeNull();
  });

  it("删除 → onChange 收到去掉该项的链", () => {
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a" }, { wxid: "wxid_b" }]} onChange={onChange} />);
    fireEvent.click(screen.getAllByLabelText("删除")[0]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }]);
  });

  it("上移第二项 → onChange 收到顺序交换的链", () => {
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a" }, { wxid: "wxid_b" }]} onChange={onChange} />);
    fireEvent.click(screen.getAllByLabelText("上移")[1]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }, { wxid: "wxid_a" }]);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx 2>&1 | tail -15`
Expected: FAIL（组件不存在）。

- [ ] **Step 3: 写 DeciderChainEditor.tsx** — Create `frontend/src/features/ask-human-config/DeciderChainEditor.tsx`

```tsx
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { Contact, DeciderRef } from "../../types";
import styles from "./AskHumanConfig.module.css";

function contactLabel(c: Contact): string {
  return c.nickname || c.remark || c.alias || c.wxid;
}

export function DeciderChainEditor({
  chain,
  onChange,
}: {
  chain: DeciderRef[];
  onChange: (next: DeciderRef[]) => void;
}) {
  const [picking, setPicking] = useState(false);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [q, setQ] = useState("");

  useEffect(() => {
    if (!picking) return;
    void (async () => {
      try {
        const res = await api.get<{ items: Contact[] }>("/api/contacts?limit=100");
        setContacts(res.items);
      } catch {
        setContacts([]);
      }
    })();
  }, [picking]);

  const inChain = new Set(chain.map((d) => d.wxid));
  const candidates = contacts
    .filter((c) => !inChain.has(c.wxid))
    .filter((c) => (q.trim() ? contactLabel(c).includes(q) || c.wxid.includes(q) : true));

  function add(c: Contact) {
    onChange([...chain, { wxid: c.wxid, displayName: contactLabel(c) }]);
    setPicking(false);
    setQ("");
  }
  function remove(idx: number) {
    onChange(chain.filter((_, i) => i !== idx));
  }
  function move(idx: number, dir: -1 | 1) {
    const j = idx + dir;
    if (j < 0 || j >= chain.length) return;
    const next = [...chain];
    [next[idx], next[j]] = [next[j], next[idx]];
    onChange(next);
  }

  return (
    <div className={styles.chainEditor}>
      {chain.length === 0 && <div className={styles.chainEmpty}>尚未配置决策人</div>}
      {chain.map((d, idx) => (
        <div key={d.wxid} className={styles.chainRow}>
          <span className={styles.chainName} title={d.wxid}>
            {d.displayName ?? d.wxid}
            <span className={styles.chainWxid}>{d.wxid}</span>
          </span>
          <div className={styles.chainActions}>
            <button type="button" aria-label="上移" disabled={idx === 0} onClick={() => move(idx, -1)}>↑</button>
            <button type="button" aria-label="下移" disabled={idx === chain.length - 1} onClick={() => move(idx, 1)}>↓</button>
            <button type="button" aria-label="删除" onClick={() => remove(idx)}>✕</button>
          </div>
        </div>
      ))}
      <div className={styles.chainHint}>超时未响应时，按此顺序转交链中下一位</div>
      {picking ? (
        <div className={styles.pickerPanel}>
          <input
            className={styles.input}
            placeholder="搜索联系人（昵称/备注/wxid）"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
          <div className={styles.pickerList}>
            {candidates.map((c) => (
              <button key={c.id} type="button" className={styles.pickerItem} onClick={() => add(c)}>
                {contactLabel(c)}
                <span className={styles.chainWxid}>{c.wxid}</span>
              </button>
            ))}
            {candidates.length === 0 && <div className={styles.chainEmpty}>无可选联系人</div>}
          </div>
          <button type="button" className={styles.linkBtn} onClick={() => { setPicking(false); setQ(""); }}>取消</button>
        </div>
      ) : (
        <button type="button" className={styles.linkBtn} onClick={() => setPicking(true)}>+ 从联系人添加</button>
      )}
    </div>
  );
}
```

- [ ] **Step 4: 写 AskHumanConfig.module.css 起步样式**（tokens.css 变量；Task 5 续填表单样式）— Create `frontend/src/features/ask-human-config/AskHumanConfig.module.css`

```css
/* 请示通道配置页样式。一律走 components/ui/tokens.css 变量，禁裸色值/魔数。 */
.chainEditor { display: flex; flex-direction: column; gap: 8px; }
.chainEmpty { font-size: 12px; color: var(--ink-3); padding: 6px 0; }
.chainRow {
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 12px; border: 1px solid var(--hairline); border-radius: var(--r-sm);
  background: var(--surface-card);
}
.chainName { font-size: 13px; color: var(--ink-1); display: flex; align-items: center; gap: 8px; min-width: 0; }
.chainWxid { font-size: 11px; color: var(--ink-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.chainActions { display: flex; gap: 4px; flex-shrink: 0; }
.chainActions button {
  min-width: 28px; height: 28px; border: 1px solid var(--hairline); border-radius: var(--r-sm);
  background: var(--surface-card); color: var(--ink-2); cursor: pointer;
}
.chainActions button:disabled { opacity: .4; cursor: not-allowed; }
.chainHint { font-size: 11px; color: var(--ink-3); }
.linkBtn {
  align-self: flex-start; padding: 6px 12px; border: 1px solid var(--hairline);
  border-radius: var(--r-sm); background: var(--surface-card); color: var(--color-scheduled); cursor: pointer; font-size: 12px;
}
.pickerPanel { display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 1px solid var(--hairline); border-radius: var(--r-md); background: var(--surface-card); }
.pickerList { display: flex; flex-direction: column; gap: 4px; max-height: 220px; overflow-y: auto; }
.pickerItem {
  display: flex; align-items: center; justify-content: space-between; gap: 8px;
  padding: 8px 10px; border: 1px solid var(--hairline); border-radius: var(--r-sm);
  background: var(--surface-card); color: var(--ink-1); cursor: pointer; font-size: 13px; text-align: left;
}
.pickerItem:hover { background: var(--fill-scheduled); }
.input {
  width: 100%; height: var(--control-h, 38px); padding: 0 12px;
  border: 1px solid var(--hairline); border-radius: var(--r-sm); background: var(--surface-card);
  color: var(--ink-1); font-size: 13px;
}
.input:focus { outline: none; box-shadow: var(--focus-ring); }
```

- [ ] **Step 5: 跑测试确认通过 + tsc**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx 2>&1 | tail -10 && npx vitest list 2>&1 | grep DeciderChainEditor && npx tsc --noEmit 2>&1 | tail -8`
Expected: 测试全 PASS；vitest list 收录非空；tsc 0 errors。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/ask-human-config/DeciderChainEditor.tsx frontend/src/features/ask-human-config/AskHumanConfig.module.css frontend/src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx
git commit -m "feat(ask-human): P3 DeciderChainEditor 受控组件(联系人选择器+排序+排除已选)+冒烟

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: AskHumanConfigView + index.tsx（主表单：读现状→编辑→整体保存）

**Files:**
- Create: `frontend/src/features/ask-human-config/index.tsx`（default export `AskHumanConfigFeature` + 内部 `AskHumanConfigView`）
- Modify: `frontend/src/features/ask-human-config/AskHumanConfig.module.css`（续填表单/开关/折叠区/header 样式）

**Interfaces:**
- Consumes: `defaultPolicy`/`extractPolicy`/`validatePolicy`（Task 3）；`DeciderChainEditor`（Task 4）；`AskHumanPolicy`（Task 2）；`api.get`/`api.put`（lib/api）；`useToast`（components/ui/Toast）；`ConfirmProvider`/`ToastProvider`（components/ui）。
- Produces: `default function AskHumanConfigFeature()`（频道入口），供 Task 6 channels.ts lazy import。

- [ ] **Step 1: 读现状（不改）**

读 `frontend/src/features/knowledge/index.tsx:384-394`（Provider 包裹范本：`<ConfirmProvider><ToastProvider>...</ToastProvider></ConfirmProvider>`）。确认 `api.put` 签名：`api.put<T>(url, body)`（lib/api.ts）。本 task 无 store，View 内 `useState` 持 `AskHumanPolicy` 草稿。

- [ ] **Step 2: 写 index.tsx** — Create `frontend/src/features/ask-human-config/index.tsx`

```tsx
import { useCallback, useEffect, useState } from "react";
import { ConfirmProvider } from "../../components/ui/ConfirmDialog";
import { ToastProvider, useToast } from "../../components/ui/Toast";
import { api } from "../../lib/api";
import type { AskHumanPolicy } from "../../types";
import { defaultPolicy, extractPolicy, validatePolicy } from "./policyForm";
import { DeciderChainEditor } from "./DeciderChainEditor";
import styles from "./AskHumanConfig.module.css";

const DOMAIN = "user_operations";

// 仅 4 个 boolean 开关字段的键联合，避免把非-bool 字段的 key 混入（否则 checked/赋值 boolean 会 TS 报错）。
type EscalateKey = "escalateSafetyGuard" | "escalateUnverifiedProduct" | "escalateAiPolicyHold" | "escalateStuck";

const ESCALATE_FIELDS: { key: EscalateKey; label: string; hint: string }[] = [
  { key: "escalateSafetyGuard", label: "安全门拦截时", hint: "命中安全护栏被拦截，请示决策人定夺" },
  { key: "escalateUnverifiedProduct", label: "产品声明未经核验时", hint: "缺可核验知识支撑的产品声明，先请示再答复" },
  { key: "escalateAiPolicyHold", label: "AI 策略主动暂缓时", hint: "AI 依策略主动暂缓，交由决策人裁决" },
  { key: "escalateStuck", label: "对话停滞推不动时", hint: "对话长时间停滞，请示决策人介入推进" },
];

function AskHumanConfigView() {
  const toast = useToast();
  const [draft, setDraft] = useState<AskHumanPolicy>(defaultPolicy());
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const res = await api.get<{ item: unknown }>(`/api/admin/operation-domains/${DOMAIN}`);
      setDraft(extractPolicy(res.item));
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
      setDraft(defaultPolicy());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function save() {
    const errs = validatePolicy(draft);
    if (errs.length > 0) {
      toast.error(errs[0]);
      return;
    }
    setSaving(true);
    try {
      await api.put(`/api/admin/operation-domains/${DOMAIN}/ask-human-policy`, draft);
      toast.success("已保存");
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
      // 保存失败草稿不丢
    } finally {
      setSaving(false);
    }
  }

  // 可选数值字段：空字符串 → 删除该键（undefined）；有值 → number。
  function setNumField(key: "dedupeWindowHours" | "dailyPushCap" | "timeoutHours", raw: string) {
    setDraft((d) => {
      const next = { ...d };
      if (raw.trim() === "") {
        delete next[key];
      } else {
        const n = Number(raw);
        if (Number.isFinite(n)) next[key] = n;
      }
      return next;
    });
  }

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>请示通道配置</h1>
        <button type="button" className={styles.saveBtn} onClick={() => void save()} disabled={saving || loading}>
          {saving ? "保存中…" : "保存"}
        </button>
      </header>

      {loadError && <div className={styles.loadError}>读取现有配置失败（已展示默认值，可编辑保存）：{loadError}</div>}

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>决策人链</h2>
        <DeciderChainEditor chain={draft.deciderChain} onChange={(c) => setDraft((d) => ({ ...d, deciderChain: c }))} />
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>触发请示的情形</h2>
        {ESCALATE_FIELDS.map((f) => (
          <label key={f.key} className={styles.toggleRow}>
            <input
              type="checkbox"
              checked={Boolean(draft[f.key])}
              onChange={(e) => setDraft((d) => ({ ...d, [f.key]: e.target.checked }))}
            />
            <span className={styles.toggleLabel}>{f.label}</span>
            <span className={styles.toggleHint}>{f.hint}</span>
          </label>
        ))}
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>超时转备选</h2>
        <div className={styles.fieldRow}>
          <input
            className={styles.numInput}
            type="number"
            min={0}
            placeholder="不限"
            value={draft.timeoutHours ?? ""}
            onChange={(e) => setNumField("timeoutHours", e.target.value)}
          />
          <span className={styles.fieldUnit}>小时（留空=无限等待）</span>
        </div>
        <div className={styles.chainHint}>主决策人多久没响应就转交链中下一位</div>
      </section>

      <details className={styles.advanced}>
        <summary className={styles.advancedSummary}>高级：推送频控</summary>
        <div className={styles.fieldRow}>
          <span className={styles.fieldLabel}>去重窗口</span>
          <input className={styles.numInput} type="number" min={0} placeholder="不去重"
            value={draft.dedupeWindowHours ?? ""} onChange={(e) => setNumField("dedupeWindowHours", e.target.value)} />
          <span className={styles.fieldUnit}>小时</span>
        </div>
        <div className={styles.fieldRow}>
          <span className={styles.fieldLabel}>每日上限</span>
          <input className={styles.numInput} type="number" min={1} placeholder="不限"
            value={draft.dailyPushCap ?? ""} onChange={(e) => setNumField("dailyPushCap", e.target.value)} />
          <span className={styles.fieldUnit}>条</span>
        </div>
        <div className={styles.fieldRow}>
          <span className={styles.fieldLabel}>静默时段</span>
          <input className={styles.numInputSm} type="number" min={0} max={23} placeholder="起"
            value={draft.quietHours?.startHour ?? ""}
            onChange={(e) => setQuietHour("startHour", e.target.value)} />
          <span className={styles.fieldUnit}>~</span>
          <input className={styles.numInputSm} type="number" min={0} max={23} placeholder="止"
            value={draft.quietHours?.endHour ?? ""}
            onChange={(e) => setQuietHour("endHour", e.target.value)} />
          <span className={styles.fieldUnit}>时区</span>
          <input className={styles.numInputSm} type="number" placeholder="+8"
            value={draft.quietHours?.tzOffsetHours ?? ""}
            onChange={(e) => setQuietHour("tzOffsetHours", e.target.value)} />
        </div>
        <div className={styles.chainHint}>三项留空=全天可推；静默时段三格须同时填</div>
      </details>
    </div>
  );

  // 静默时段三格：任一改动重建 quietHours；三格全空则删除 quietHours。
  function setQuietHour(field: "startHour" | "endHour" | "tzOffsetHours", raw: string) {
    setDraft((d) => {
      const cur = d.quietHours ?? { startHour: NaN, endHour: NaN, tzOffsetHours: NaN };
      const next = { ...cur, [field]: raw.trim() === "" ? NaN : Number(raw) };
      const allEmpty = Number.isNaN(next.startHour) && Number.isNaN(next.endHour) && Number.isNaN(next.tzOffsetHours);
      const copy = { ...d };
      if (allEmpty) {
        delete copy.quietHours;
      } else {
        // 仅当三格都是有效数字才落 quietHours，否则保留编辑中态（用 0 占位避免 NaN 进 body）。
        copy.quietHours = {
          startHour: Number.isNaN(next.startHour) ? 0 : next.startHour,
          endHour: Number.isNaN(next.endHour) ? 0 : next.endHour,
          tzOffsetHours: Number.isNaN(next.tzOffsetHours) ? 0 : next.tzOffsetHours,
        };
      }
      return copy;
    });
  }
}

export default function AskHumanConfigFeature() {
  return (
    <ConfirmProvider>
      <ToastProvider>
        <AskHumanConfigView />
      </ToastProvider>
    </ConfirmProvider>
  );
}
```
注：`setQuietHour` 是 `AskHumanConfigView` 内的函数声明（hoisted，return 后定义合法）。若实现者偏好，可移到 return 之前——行为不变，保持在组件作用域内即可（需访问 `setDraft`）。

- [ ] **Step 3: 续填 AskHumanConfig.module.css**（append 表单/开关/折叠/header 样式，tokens 变量）

在 `frontend/src/features/ask-human-config/AskHumanConfig.module.css` 末尾 append：
```css
.page { padding: var(--page-y, 24px) var(--page-x, 32px); display: flex; flex-direction: column; gap: var(--section-gap, 18px); }
.header { display: flex; align-items: center; justify-content: space-between; }
.title { font-size: 28px; font-weight: 600; color: var(--ink-1); letter-spacing: 0; margin: 0; }
.saveBtn {
  height: var(--control-h, 38px); padding: 0 20px; border: none; border-radius: var(--r-sm);
  background: var(--color-scheduled); color: #fff; font-size: 13px; cursor: pointer;
}
.saveBtn:disabled { opacity: .5; cursor: not-allowed; }
.loadError {
  padding: 10px 14px; border-radius: var(--r-sm); font-size: 12.5px;
  color: var(--color-blocked); background: var(--fill-blocked); border: 1px solid var(--hairline);
}
.section {
  display: flex; flex-direction: column; gap: 10px;
  padding: var(--panel-pad, 18px); border: 1px solid var(--hairline);
  border-radius: var(--r-lg); background: var(--surface-card);
}
.sectionTitle { font-size: 18px; font-weight: 600; color: var(--ink-1); margin: 0; }
.toggleRow { display: grid; grid-template-columns: auto auto 1fr; align-items: center; gap: 10px; cursor: pointer; }
.toggleLabel { font-size: 13px; color: var(--ink-1); }
.toggleHint { font-size: 11.5px; color: var(--ink-3); }
.fieldRow { display: flex; align-items: center; gap: 8px; }
.fieldLabel { font-size: 13px; color: var(--ink-2); min-width: 64px; }
.fieldUnit { font-size: 12px; color: var(--ink-3); }
.numInput {
  width: 120px; height: var(--control-h, 38px); padding: 0 12px;
  border: 1px solid var(--hairline); border-radius: var(--r-sm); background: var(--surface-card);
  color: var(--ink-1); font-size: 13px;
}
.numInput:focus, .numInputSm:focus { outline: none; box-shadow: var(--focus-ring); }
.numInputSm {
  width: 64px; height: var(--control-h, 38px); padding: 0 10px;
  border: 1px solid var(--hairline); border-radius: var(--r-sm); background: var(--surface-card);
  color: var(--ink-1); font-size: 13px;
}
.advanced {
  padding: var(--panel-pad, 18px); border: 1px solid var(--hairline);
  border-radius: var(--r-lg); background: var(--surface-card); display: flex; flex-direction: column; gap: 10px;
}
.advancedSummary { font-size: 13px; color: var(--ink-2); cursor: pointer; user-select: none; }
```

- [ ] **Step 4: 类型检查 + 构建 + lint**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -12 && npm run build 2>&1 | tail -6 && bash scripts/check-no-human-takeover.sh 2>&1 | tail -3`
Expected: 0 type errors；build 成功（含频道 lazy chunk，但本 task 还没注册频道，build 仅校验编译——频道在 Task 6 接线，本 task index.tsx 作为孤立模块也应编译过）；lint clean。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/ask-human-config/index.tsx frontend/src/features/ask-human-config/AskHumanConfig.module.css
git commit -m "feat(ask-human): P3 配置页主表单(读现状回显+4开关+超时+频控折叠+整体保存)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 频道注册 + 收尾验证

**Files:**
- Modify: `frontend/src/types/index.ts`（Channel union 加 `| "askHumanConfig"`）
- Modify: `frontend/src/app/channels.ts`（lazy import + lucide 图标 `SlidersHorizontal` + CHANNELS 数组项）

**Interfaces:**
- Consumes: `AskHumanConfigFeature`（Task 5 default export）。
- Produces: 频道 `askHumanConfig` 在侧栏「运营」组可见、可进入。

- [ ] **Step 1: 加 Channel union**（`frontend/src/types/index.ts`）

把 union 末项 `| "productsDeals";` 改为下面两行（注意 P2 已加过 `askHuman`，确认 union 现末项实际是什么，append 在末尾即可）：
```ts
  | "productsDeals"
  | "askHuman"
  | "askHumanConfig";
```
（实现时按文件真实现状改：找到 union 最后一项，去掉其分号、在末尾追加 `| "askHumanConfig";`。P2 的 `| "askHuman"` 若已存在则保留，不重复加。）

- [ ] **Step 2: 注册频道**（`frontend/src/app/channels.ts`）

lazy import 区（`AskHumanFeature` 那行附近）加：
```ts
const AskHumanConfigFeature = lazy(() => import("../features/ask-human-config"));
```
顶部 lucide 导入块加 `SlidersHorizontal`（加进现有 `from "lucide-react"` 块，按字母位插入，不新开 import）。
`CHANNELS` 数组加一项（group「运营」，放在 askHuman 之后或运营组合适位置）：
```ts
  {
    id: "askHumanConfig",
    group: "运营",
    label: "请示通道配置",
    caption: "Ask-Human Policy",
    icon: SlidersHorizontal,
    eyebrow: "Ask-Human Policy",
    title: "请示通道配置",
    subtitle: "配置决策人链、触发请示的情形、超时转备选与推送频控；保存后即时生效于私聊运营域。",
    Component: AskHumanConfigFeature,
  },
```

- [ ] **Step 3: 类型检查 + 构建 + lint**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -10 && npm run build 2>&1 | tail -6 && bash scripts/check-no-human-takeover.sh 2>&1 | tail -3`
Expected: 0 type errors；build 成功（askHumanConfig lazy chunk 生成）；lint clean。

- [ ] **Step 4: 收尾验证（前端三连 + 后端基线 + lint）**

Run:
```
cd frontend && npx tsc --noEmit && npm run test 2>&1 | tail -6 && npm run build 2>&1 | tail -4
cd .. && rm -rf target/debug/incremental; CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5
bash scripts/check-no-human-takeover.sh 2>&1 | tail -3
```
Expected: 前端 tsc 0 / vitest 全绿（含 P3 新增 policyForm + DeciderChainEditor 测试，且被 vitest 收录）/ build 成功；后端 lib ≥350/0；lint clean。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/types/index.ts frontend/src/app/channels.ts
git commit -m "feat(ask-human): 注册 askHumanConfig 请示通道配置频道(运营组)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

- [ ] **Step 6: UI 真验（环境允许时）**

起后端 `cargo run`（:8080）+ 前端 `cd frontend && npm run dev`，浏览器进 admin：
- 侧栏「运营」组出现「请示通道配置」频道。
- golden path：进频道→读现状回显（首次为默认空链）→从联系人加 2 个决策人→上下移调序→勾改 escalate 开关→填 timeout→展开高级填频控→保存→toast「已保存」→刷新/重进确认持久化（验证 Task 1 后端回显生效）。
- 校验路径：清空决策人链→保存→被挡 + toast「至少配置一个决策人」。
- 视觉对齐：白通道 / tokens 变量 / 字号层级 / 选中 soft blue，与现有频道一致无漂移。
- 无法在浏览器验证的部分明确标注，不假称成功。

## 收尾验证（全部 task 后）

- [ ] **前端三连**：`cd frontend && npx tsc --noEmit && npm run test && npm run build` 全绿。
- [ ] **后端基线**（Task 1 触及）：`rm -rf target/debug/incremental; CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -5` → ≥350/0；四 PBT 累计 ≥33/0。
- [ ] **no-takeover lint**：`bash scripts/check-no-human-takeover.sh` → clean。
- [ ] **集成测试留 CI**：`ask_human_phase1_e2e` 的新测试 `#[ignore]`，CI integration job 跑 `--ignored`。
- [ ] **共享工作树**：精确 `git add` 具名文件，排除并行会话产物。

## Phase 3 完成定义

admin 侧出现「请示通道配置」频道：决策人链（联系人选择器+排序）、4 个 escalate 触发开关、超时转备选平铺主区，推送频控（去重/每日上限/静默时段）收进高级折叠区；读现状回显（后端补 operation_domain_json 序列化 askHumanPolicy）+ 整体 PUT 保存；空链/越界等校验在前端先挡、后端权威；保存失败草稿不丢；遵守现有设计系统（tokens.css 变量 + module.css 绑定导入 + 4 级层级 + 白通道）。**ask-human 三子项目（P1 后端→P2 收件箱→P3 配置页）全部交付。**
