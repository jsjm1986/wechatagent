# Ask-Human 请示通道配置页（Phase 3）设计

> 状态：设计已逐段获批（2026-06-21 brainstorming）。本 spec 详设 **Phase 3（请示通道配置页）**，ask-human 三子项目的最后一步，可直接转 writing-plans。

## 背景与目标

ask-human 统一频道分三个子项目顺序交付：**P1 后端地基（已完成）→ P2 收件箱前端（已完成，feat/ask-human-phase1 已推送）→ P3 配置页（本 spec）**。

P1 已交付决策请示通道的运行时（`resolve_ask_human_policy` 纯函数 + `push_allowed`/`next_decider_on_timeout` 等门控）和写入端点 `PUT /api/admin/operation-domains/:domain/ask-human-policy`。但**管理员目前无任何 UI 配置这条通道**——决策人链、触发情形、骚扰频控、超时全靠直接写库或迁移种子。P3 补这个配置界面。

P3 目标：一个独立的 `askHumanConfig`「请示通道配置」前端频道，让管理员可视化配置 `AskHumanPolicy` 的全部字段：决策人链（联系人选择器 + 可排序）、4 个 escalate 触发开关、超时转备选、推送频控（去重窗口/每日上限/静默时段），每项配引导文案。读现有配置回显 + 整体保存。

## 用户拍板的四个核心决策（2026-06-21 brainstorming）

1. **独立新频道**（非并入 P2 收件箱，非并入 user-ops settings）：职责单一，侧栏「运营」组新增「请示通道配置」。
2. **决策人链 = 联系人选择器 + 可排序**：从 `GET /api/contacts` 选人加链，链内上下移调序（主→备选）、可删，displayName 从联系人带入，不手填 wxid。
3. **补后端序列化 askHumanPolicy**：`operation_domain_json`（domains.rs:290）当前不序列化 `ask_human_policy`，前端读不到现状回显 → 补一行序列化（与 P2 后端补缺同模式，向后兼容）。
4. **核心平铺 + 频控折叠**：决策人链 + 4 escalate 开关 + timeoutHours 平铺主区；骚扰频控三项（dedupeWindowHours/dailyPushCap/quietHours）收进默认折叠的「高级：推送频控」区。

## 消费的后端契约（已实证，file:line 亲核）

- **读现状**：`GET /api/admin/operation-domains/user_operations` → `{ item: operation_domain_json(config) }`（domains.rs:79/86）。当前 `operation_domain_json`（domains.rs:273-295）**未序列化 `ask_human_policy`** → **本 spec 后端补缺**。
- **写策略**：`PUT /api/admin/operation-domains/:domain/ask-human-policy`，body = `AskHumanPolicy` 整体 JSON（domains.rs:206）。后端已校验：decider_chain 各 wxid 非空、quietHours startHour/endHour ≤ 23。成功 → `{ ok: true }`；当前版本不存在 → 404。
- **联系人**：`GET /api/contacts?limit=100` → `{ items: Contact[] }`（mod.rs:299，前端 products-deals ContactPicker:356 实证用法）。
- **domain key**：固定 `user_operations`（P1 scope = 私聊运营，prompts.rs:517 实证）。

### 前端类型缺口（实证：P3 须先补）

`AskHumanPolicy` / `DeciderRef` / `AskHumanQuietHours` **当前前端 `types/index.ts` 不存在**（仅有不相关的 `QuietHoursMode`）。P3 第一步须在 `frontend/src/types/index.ts` 新增这三个 type（camelCase，对齐后端 serde），policyForm/DeciderChainEditor 才能 import。`Contact` 类型（types/index.ts:46 实证）字段为 `wxid` / `nickname?` / `remark?` / `alias?`（**无 `name` 字段**）→ 决策人链 displayName 取 `nickname ?? remark ?? alias ?? wxid`。

### `AskHumanPolicy` 形状（models.rs:825，camelCase serde）

```
AskHumanPolicy {
  deciderChain: DeciderRef[]            // 有序，主→备选；[] = 未启用请示通道
  escalateSafetyGuard: bool             // 默认 true（default_true）
  escalateUnverifiedProduct: bool       // 默认 true
  escalateAiPolicyHold: bool            // 默认 false
  escalateStuck: bool                   // 默认 true
  dedupeWindowHours?: f64               // 省略 = 不去重
  dailyPushCap?: u32                    // 省略 = 不限
  quietHours?: { startHour: u8, endHour: u8, tzOffsetHours: i8 }  // 省略 = 全天可推
  timeoutHours?: f64                    // 省略 = 无限等待
}
DeciderRef { wxid: string, displayName?: string }  // displayName 省略可
```

`ResolvedAskHumanPolicy`（policy.rs:8）的回落语义（policy 为 None 时）：链空、safety/product/stuck=true、aiPolicyHold=（high_risk_escalation_mode=="all"）、其余 None。`defaultPolicy()` 取「无 high_risk all 模式」的保守默认：safety/product/stuck=true、aiPolicyHold=false、链空、可选项全 undefined。

## 架构与分层

```
新频道 features/ask-human-config/
  ├── index.tsx              频道入口 default export：ConfirmProvider/ToastProvider 包裹
  │                          + AskHumanConfigView（读现状→编辑本地草稿→整体保存）
  ├── DeciderChainEditor.tsx 决策人链受控子组件：ContactPicker 选人加链 + 上下移/删
  ├── policyForm.ts          纯函数：defaultPolicy / extractPolicy / validatePolicy（最高价值测试层）
  └── AskHumanConfig.css     plain CSS（非 .module.css，规避 Rollup tree-shake 坑）

复用（不新建）：
  ├── lib/api.ts             api.get<{item}>(读 domain) / api.put(存 policy)
  ├── components/ui/{Toast,ConfirmDialog}  useToast / useConfirm（频道外壳包 Provider）
  └── GET /api/contacts?limit=100          决策人链选人来源（仿 products-deals ContactPicker:343）

后端补缺（1 处）：
  └── operation_domain_json(domains.rs:290) 补 "askHumanPolicy": config.ask_human_policy 序列化 → GET 回显

注册（3 处，同 P2 机制）：
  ① types/index.ts：Channel union 加 | "askHumanConfig"
  ② app/channels.ts：lazy import + lucide 图标（如 SlidersHorizontal）+ CHANNELS 数组项（group「运营」）
  ③ 无第三处接线：Shell.tsx CHANNELS.find 自动渲染
```

### 状态管理选型：局部 useState（非 Zustand）

配置页本质是「读一份配置 → 编辑 → 整体存」的单页单表单单提交，无跨组件/跨页共享状态需求。`AskHumanConfigView` 用 `useState` 持整个 `AskHumanPolicy` 草稿即可，**不建 store**（Zustand 在此是过度设计——P2 inboxStore 有 per-source 降级语义才值得用 store，此处没有）。决策人链作受控子组件，草稿仍在父 View。

## 组件与接口

### `policyForm.ts`（纯函数，无 IO，单测主场）

```ts
import type { AskHumanPolicy } from "../../types";

// 空链 + 保守默认开关（safety/product/stuck=true, aiPolicyHold=false），可选项全 undefined。
export function defaultPolicy(): AskHumanPolicy;

// 从 GET /operation-domains/:domain 的 item.askHumanPolicy 抽策略；缺失/非对象 → defaultPolicy()。
// 对每个字段做存在性回落（部分字段缺也补默认），保证返回结构完整可编辑。
export function extractPolicy(domainItem: unknown): AskHumanPolicy;

// 校验草稿；返回错误消息数组（空 = 通过）。规则：
//  - deciderChain 为空 → "至少配置一个决策人"
//  - 任一 decider wxid 空白 → "决策人 wxid 不能为空"（联系人选择器正常不会触发，防御）
//  - quietHours 存在且 startHour>23 或 endHour>23 → "静默时段小时须 0-23"
//  - dedupeWindowHours / timeoutHours 存在且 < 0 → "...不能为负"
//  - dailyPushCap 存在且 < 1 → "每日上限至少为 1"
export function validatePolicy(p: AskHumanPolicy): string[];
```

### `DeciderChainEditor.tsx`（受控）

```ts
import type { DeciderRef } from "../../types";

function DeciderChainEditor({ chain, onChange }: {
  chain: DeciderRef[];
  onChange: (next: DeciderRef[]) => void;  // 加/删/上下移都回调整个新数组（受控，父持真相）
}): JSX.Element;
```
- 渲染有序链：每行 `displayName(wxid)` + 上移 ↑ / 下移 ↓ / 删 ✕。首行禁上移、末行禁下移。
- 底部「+ 从联系人添加」：内嵌 ContactPicker（`GET /api/contacts?limit=100`），选中 contact → append `{ wxid: contact.wxid, displayName: contact.nickname ?? contact.remark ?? contact.alias ?? contact.wxid }`（Contact 无 `name` 字段，types/index.ts:46 实证），**已在链中的 wxid 从候选排除**。
- 引导小字：「超时未响应时，按此顺序转交链中下一位」。

### `AskHumanConfigView`（index.tsx 内）

- mount：`GET /api/admin/operation-domains/user_operations` → `extractPolicy(res.item)` → `setDraft`。失败 → 顶部红 banner + 草稿置 `defaultPolicy()`（可编辑可存，不卡死）。
- 编辑：决策人链 / 4 开关 / timeoutHours / 折叠区频控，全改本地 `draft`，不实时提交。
- 保存：`validatePolicy(draft)` 先跑 → 有错 toast.error 首条 + 阻止；通过 → `PUT .../ask-human-policy` body=draft → 成功 toast.success「已保存」+ 重 GET 回显；失败 toast.error（后端消息）+ **草稿不丢**。

### `index.tsx` default export

```tsx
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

## UI 布局（线性单栏，企业白通道）

```
请示通道配置                                    [保存]
[GET 失败时红 banner]

① 决策人链
   [✕ 张三(wxid1)] [↑禁][↓]
   [✕ 李四(wxid2)] [↑][↓禁]
   [+ 从联系人添加 ▾]   ← ContactPicker，排除已在链中
   超时未响应时，按此顺序转交链中下一位

② 触发请示的情形（4 toggle，每行引导小字）
   ☑ 安全门拦截时        escalateSafetyGuard
   ☑ 产品声明未经核验时   escalateUnverifiedProduct
   ☐ AI 策略主动暂缓时    escalateAiPolicyHold
   ☑ 对话停滞推不动时     escalateStuck

③ 超时转备选            timeoutHours（数字 + 「不限」勾选；勾选=undefined）
   主决策人多久没响应就转链中下一位（小时）

▸ 高级：推送频控（默认折叠）
   · 去重窗口 dedupeWindowHours（小时，空=不去重）
   · 每日上限 dailyPushCap（条，空=不限）
   · 静默时段 quietHours：startHour ~ endHour（0-23）+ tzOffsetHours（空=全天可推）
```

## 数据流、错误处理与降级

- **空配置不报错**：首次未配过（item.askHumanPolicy 缺）→ extractPolicy 回落 defaultPolicy → 展示空链表单引导填写。
- **保存失败草稿不丢**：PUT 失败仅 toast.error，draft state 保留（用户编辑内容不因一次请求失败丢失，仿 P2 降级精神）。
- **前端校验是体验优化，后端是权威**：validatePolicy 先挡（wxid 非空 / quietHours 0-23 / 数值合法），但后端 put_ask_human_policy 本就有 wxid 非空 + quietHours 0-23 校验，不依赖前端。
- **可选项的「空 = 关闭」语义**：timeoutHours/dedupeWindowHours/dailyPushCap/quietHours 留空 → 提交时该字段 undefined（不带进 body），后端按 None 处理（无限等待 / 不去重 / 不限 / 全天可推）。

## 测试策略

- **纯函数单测（vitest，最高价值层）**：`policyForm.ts` 三函数 —— defaultPolicy 默认值正确；extractPolicy 完整/部分缺/全缺三种输入回落正确；validatePolicy 各错误分支（空链 / wxid 空 / quietHours 越界 / 负数 / cap<1）+ 合法直通。落 `src/__tests__/features/ask-human-config/policyForm.test.ts`。
- **组件冒烟**：DeciderChainEditor 增（选 contact append）/删/上移/下移/首末行按钮禁用 + 已在链中 wxid 排除（mock api.get contacts）。落 `src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx`。
- **后端补缺**：operation_domain_json 序列化 askHumanPolicy 走 Rust 单测（构造带 policy 的 config → json 含 askHumanPolicy.deciderChain）+ 并入 P1 `ask_human_phase1_e2e` 同源集成测试（`#[ignore]`，CI 跑）。
- **测试位置铁律**：vitest.config include 锁 `src/__tests__/**`，测试必须落该目录镜像路径，否则 CI 静默跳过=假绿（P2 Task3/10 踩过）。
- **UI 真验**：起 dev server 浏览器走 golden path（进频道→读现状回显→加决策人→改开关→存→重进确认持久化）+ 校验路径（空链存被挡）+ 后端补缺回显。无法在浏览器验证的部分明确标注，不假称成功。

## 交付边界

- ✅ 含：前端 types 补 `AskHumanPolicy`/`DeciderRef`/`AskHumanQuietHours` 三 type + 新 `askHumanConfig` 频道（index + DeciderChainEditor + policyForm + CSS）+ 频道注册 3 处 + 后端 1 处补缺（operation_domain_json 序列化 askHumanPolicy）+ 纯函数/组件单测 + 后端 Rust/集成测试。
- ❌ 不含：多 domain 配置（P1 scope 仅 user_operations，group/moment 是独立运营域，不在本轮）；ask-human-policy 端点本身的改动（已存在，仅消费）；P2 收件箱的任何改动（已交付）；referral 名片 / assist_mode 配置（独立子项目）。

## 红线与约束

- **no-takeover lint**（扫 `frontend/src/` 新增行）：开关/引导文案一律 AI 内部口径——「安全门拦截 / 产品声明未核验 / AI 策略主动暂缓 / 对话停滞 / 决策人 / 请示通道」，**绝不**出现 `转人工 / 人工接管 / 人工介入 / 接管 / takeover / hand-off`。
- **CSS plain 不 tree-shake**：用 `AskHumanConfig.css` + `import "./AskHumanConfig.css"`，不命名 `.module.css` 做副作用全局导入。
- **后端补缺向后兼容**：序列化 askHumanPolicy 是加字段（`config.ask_human_policy` 本就是 `Option`，None 时序列化为 null 或 skip，前端 extractPolicy 回落 default），不动既有 operation_domain_json 其他字段。
- **测试基线不回归**：后端补缺的 Rust 单测进 `cargo test --lib`（≥350/0）；四 PBT 累计 ≥33/0。
- **磁盘纪律**：后端编译前 `rm -rf target/debug/incremental` + `CARGO_INCREMENTAL=0`；本地只 `cargo test --lib` + 单 PBT，集成测试 `#[ignore]` 靠 CI。
- **共享工作树**：本分支与并行会话（referral-card / media-asset）交错；前端 `git add` 精确具名，排除并行会话产物。

## 命令

- 前端：`cd frontend && npm run dev`（vite，代理 /api → :8080）；`npm run build`；`npm run test`（vitest）。
- 后端补缺：`cargo check --lib`；`CARGO_INCREMENTAL=0 cargo test --lib`；`cargo test --test ask_human_phase1_e2e --no-run`（编译，集成测试 CI 跑）。
