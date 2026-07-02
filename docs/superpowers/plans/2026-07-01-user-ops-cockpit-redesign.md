# 用户运营驾驶舱前端重设计 实施计划（Spec A）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把用户运营驾驶舱从"9 段纵向堆叠 + 查看/编辑混杂"重构为"常驻判断条 + 观测/配置段控 + 下钻"三段式，并把 finalReviewStatus / conversation_mode / 健康度 tone / 标签三层 / 记忆溯源等后端已产出但前端未表达的能力顶到前台。

**Architecture:** 前端为主。新建 `frontend/src/features/user-ops/cockpit/` 承载重构后的驾驶舱（CockpitPanel 外壳 + JudgmentBar + ObserveView + ConfigureView + 下钻视图），CSS Modules 引用 tokens.css。一个后端小改：`operation_health_json` 补 3 个 quiet_hours 只读字段（作息灯数据源）。

**Tech Stack:** Rust/Axum（后端）+ React 19 + Vite + TypeScript + CSS Modules（前端）+ vitest。

## Global Constraints

- **核心定位（全 AI 自治）**：客户永远只跟 AI 对话，运营在幕后观测 + 偶尔干预。驾驶舱主轴 = 观测与配置分离。
- **禁用词 lint**（`scripts/check-no-human-takeover.sh` 扫 `frontend/src/` 新增行）：绝不出现单字 `人工` / `接管` / `takeover` / `hand-off`。AI 暂缓/等待用 AI 内部语义（如「AI 策略主动暂缓」「AI 等待更多上下文」）。
- **CSS tree-shake 坑**：全局字符串 className 的 CSS 绝不命名 `.module.css` 副作用导入（Rollup 摇树删光）。新组件用 CSS Modules（`import styles from "./x.module.css"` + `className={styles.x}`）才安全。
- **设计系统**（`docs/frontend-design-system.md`）：紫 `--color-brand`=AI 身份不表达状态；蓝 `--color-scheduled`=主操作/可点击；绿=running/held=橙/blocked=红。四级层级，不做第三层持久导航，面板不套卡片。**新组件引用 tokens.css var()，不硬编码色值。**（全局 token 三源统一是独立 Spec B，本次不做。）
- **finalReviewStatus 10 态**（`src/agent/run_envelope.rs:67-78`，前端已能拿到、`FINAL_REVIEW_STATUS_LABELS` 中文映射已存在于 operations/autonomy 频道）：绿已发 `approved`/`revision_applied_approved`；橙暂缓 `held_by_ai_policy`/`ai_waiting_for_more_context`；红拦截 `blocked_by_safety_guard`/`blocked_by_required_field`/`blocked_by_budget`/`blocked_unverified_product_claim`/`revision_failed`；灰 `legacy_mode_unchecked`。
- **OperationHealth 结构**（`src/routes/shared.rs:459-514`）：`{ scores, items }`，items 每项 `{ key, label, score(0-100), tone(good/warn/danger), detail }`，7 固定 item，tone 后端已按 `Risk` 后缀反转判定，前端不算方向。
- **conversation_mode 四态**（`src/agent/types.rs:224-232`）：casual_relationship / value_exchange / consultative / boundary_protection，可被 DomainProfile 覆盖 → 前端未知值兜底显原值 + reason。
- **测试基线**：`cargo test --lib` ≥350/0；前端现有 user-ops 测试因重构失效的需同步更新（重构必要连带，非调绿）。
- **不碰**：传统模式（TraditionalOpsTabs 四 tab）、knowledge/cockpit/、全局 token 定义、请示裁决逻辑（驾驶舱只做灯+跳转）。

---

### Task 1: 后端 operation-health 补 quiet_hours 只读字段

**Files:**
- Modify: `src/routes/shared.rs`（`operation_health_json` :459-514）
- Modify: `src/routes/contacts.rs`（`get_operation_health` :1075-1088）
- Test: `src/routes/shared.rs` 的 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `operation_health_json` 返回 JSON 顶层新增 `inQuietHours: bool` / `nextWakeAt: string|null`(RFC3339) / `quietHoursEnabled: bool`。前端 JudgmentBar 作息灯消费。

**背景（已亲验，全部带 file:line）**：
- 作息纯函数（`src/agent/quiet_hours.rs`，均 `pub(crate)`）：`is_quiet_now(start,end,tz_offset)` (:88)、`next_wake_at(end,tz_offset,jitter_seed,jitter_max_seconds)` (:99) 返回 `mongodb::bson::DateTime`、`effective_quiet_hours_enabled(&contact,&profile,global_enabled)` (:128)。
- runtime 由 `UserRuntimeParameters::from_config(domain_config, state)` 构建（`src/agent/runtime.rs:126`），字段 `quiet_hours_start`/`quiet_hours_end`/`quiet_hours_tz_offset_hours`/`quiet_hours_enabled`。
- profile 由 `crate::agent::domain_profile::load_active_domain_profile(&db, workspace_id)` 得（`src/agent/domain_profile.rs:978`，async）。
- domain_config 加载复用 `crate::agent::decision::load_user_operation_domain_config`（`src/agent/decision.rs:1069`，先 Read 确认其签名再用）。
- 现 `get_operation_health`（`contacts.rs:1080-1082`）只加载 contact/memory/latest_review，**无 runtime/profile**——本 Task 需在端点内加载 domain_config→runtime + profile，把结果算好后作为参数传给 `operation_health_json`。
- `jitter_seed` 传 `&contact.wxid`，`jitter_max_seconds` 传 `state.config.wake_jitter_max_seconds`（与 gateway.rs:945 一致）。

**红线**：只读聚合，不碰 agent 决策/gateway/发送逻辑，不改 quiet_hours 纯函数本身。

- [ ] **Step 1: 改 `operation_health_json` 签名 + 输出（先让它接收算好的 3 值）**

先 Read `src/routes/shared.rs:459-514` 确认当前签名与 json! 结构。把签名从 `(contact, memory, latest_review)` 扩展为额外接收 `in_quiet_hours: bool, next_wake_at: Option<String>, quiet_hours_enabled: bool`，并在返回的 `json!({...})` 顶层加：
```rust
        "inQuietHours": in_quiet_hours,
        "nextWakeAt": next_wake_at,
        "quietHoursEnabled": quiet_hours_enabled,
```
（保留现有 `scores`/`items` 不变。）

- [ ] **Step 2: 写后端单测（TDD）**

在 `shared.rs` 的 `#[cfg(test)] mod tests` append。参照现有 `decision_review_json_matches_contract_fixture`（:1933 附近）的构造风格构造一个 contact+memory，断言新字段存在且类型正确：
```rust
    #[test]
    fn operation_health_json_carries_quiet_hours_fields() {
        let contact = /* 复用本文件测试已有的 contact 构造 helper；若无则最小构造 */;
        let memory = /* 同上 */;
        let v = operation_health_json(&contact, &memory, None, true, Some("2026-07-02T00:00:00Z".to_string()), true);
        assert_eq!(v["inQuietHours"], serde_json::json!(true));
        assert_eq!(v["nextWakeAt"], serde_json::json!("2026-07-02T00:00:00Z"));
        assert_eq!(v["quietHoursEnabled"], serde_json::json!(true));
        // 非静默时 nextWakeAt 应为 null
        let v2 = operation_health_json(&contact, &memory, None, false, None, true);
        assert_eq!(v2["nextWakeAt"], serde_json::json!(null));
    }
```
> 若本文件测试没有现成 contact/memory 构造 helper，Read 邻近测试看它们怎么造（`shared.rs:1919` 附近有 review 相关构造），用最小字段构造。

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test --lib operation_health_json_carries_quiet_hours 2>&1 | tail -12`
Expected: 编译失败（`operation_health_json` 参数数量不匹配）→ 改签名后应 PASS。若磁盘满先 `rm -rf target/debug/incremental`。

- [ ] **Step 4: 在 `get_operation_health` 端点算 quiet_hours 三值并传入**

`src/routes/contacts.rs:1075-1088`。在 `let latest_review = ...` 之后、`operation_health_json(...)` 之前插入（先 Read `decision::load_user_operation_domain_config` 与 `UserRuntimeParameters::from_config` 的确切签名，按实际调整）：
```rust
    let domain_config = agent::decision::load_user_operation_domain_config(&state, &admin.current_workspace).await;
    let runtime = agent::runtime::UserRuntimeParameters::from_config(domain_config.as_ref(), &state);
    let profile = agent::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id).await;
    let quiet_enabled = agent::quiet_hours::effective_quiet_hours_enabled(&contact, &profile, runtime.quiet_hours_enabled);
    let in_quiet = quiet_enabled
        && agent::quiet_hours::is_quiet_now(runtime.quiet_hours_start, runtime.quiet_hours_end, runtime.quiet_hours_tz_offset_hours);
    let next_wake = if in_quiet {
        Some(agent::quiet_hours::next_wake_at(runtime.quiet_hours_end, runtime.quiet_hours_tz_offset_hours, &contact.wxid, state.config.wake_jitter_max_seconds).try_to_rfc3339_string().unwrap_or_default())
    } else {
        None
    };
    Ok(Json(operation_health_json(&contact, &memory, latest_review.as_ref(), in_quiet, next_wake, quiet_enabled)))
```
> `quiet_hours` / `runtime` / `domain_profile` / `decision` 若非 `pub(crate)` 可见，按最小改动暴露（报告说明）。`try_to_rfc3339_string` 是 bson DateTime 的方法；若不可用，用 `.to_chrono().to_rfc3339()`——Read 确认 bson 版本支持哪个。

- [ ] **Step 5: 编译 + 全量 lib**

Run: `cargo check 2>&1 | tail -8`（EXIT 0）；`cargo test --lib 2>&1 | tail -5`（≥350/0）。
Expected: 编译过，基线不回归。集成测试（端点级）留 CI（本机无 Docker）。

- [ ] **Step 6: 禁用词 lint + Commit**

Run: `bash scripts/check-no-human-takeover.sh HEAD 2>&1 | tail -3`（本 Task 改 src/routes，扫描目录内，须 0 violations——无"人工"字面）。
```bash
git add src/routes/shared.rs src/routes/contacts.rs
git commit -m "feat(user-ops): operation-health 补 inQuietHours/nextWakeAt/quietHoursEnabled 只读字段"
```

---

### Task 2: 脚手架 — cockpit/ 目录 + CockpitPanel 外壳 + 段控（行为等价迁移）

**Files:**
- Create: `frontend/src/features/user-ops/cockpit/CockpitPanel.tsx`
- Create: `frontend/src/features/user-ops/cockpit/cockpit.module.css`
- Modify: `frontend/src/features/user-ops/index.tsx`（把 `<UserOperationCockpit .../>` 换成 `<CockpitPanel .../>`）
- 现 `UserOperationCockpit`（`legacy.tsx:184-717`）暂保留，Task 2 末尾确认无引用后可在 Task 6 删除

**Interfaces:**
- Produces: `CockpitPanel` 接收与现 `UserOperationCockpit` **完全相同的 props**（`legacy.tsx:231-278` 那份 30+ props 类型），内部用 `viewMode` state（`"observe" | "configure"`）+ `drilldown` state（`null | "memory" | "conversation" | "sendHistory"`）管理视图切换。本 Task 只做外壳 + 段控 + 把现有 6 tab 内容临时全塞进来保证不丢功能，后续 Task 再拆。

**背景**：现 `UserOperationCockpit` 用 `activeTab`（6 值）+ `SmartOpsTabs`（`legacy.tsx:919-945`）。本 Task 引入段控替代，但**先保证行为等价**——把现 cockpit/adjust/profile/memory/simulation/conversation 六段内容原样搬进 CockpitPanel，观测段先放 cockpit 只读内容，配置段先放 adjust+profile+memory+simulation，会话进下钻。这一步是"结构搬家 + 段控骨架"，不改数据流。

- [ ] **Step 1: 建 CockpitPanel 外壳 + 段控 + CSS Module**

先 Read `legacy.tsx:184-278`（现 props 定义）完整抄成 CockpitPanel 的 props 类型。建 `CockpitPanel.tsx`：
```tsx
import { useState } from "react";
import styles from "./cockpit.module.css";
// ... 复用 legacy 现有 import 的类型/图标/子组件（从 "../legacy" 或原位置导入）

type ViewMode = "observe" | "configure";
type Drilldown = null | "memory" | "conversation" | "sendHistory";

export function CockpitPanel(props: /* 抄 legacy.tsx:231-278 的 props 类型 */) {
  const { selected } = props;
  const [viewMode, setViewMode] = useState<ViewMode>("observe");
  const [drilldown, setDrilldown] = useState<Drilldown>(null);

  if (!selected) {
    // 抄 legacy.tsx:282-292 的 cockpitEmpty onboardingSteps 空态
  }

  return (
    <section className={styles.cockpitPanel}>
      {/* JudgmentBar 占位：Task 3 填充。先渲染现 panelHead（legacy.tsx:305-314） */}
      {drilldown === null ? (
        <>
          <div className={styles.segmented} role="tablist">
            <button role="tab" aria-selected={viewMode === "observe"} className={viewMode === "observe" ? styles.segActive : styles.seg} onClick={() => setViewMode("observe")}>观测</button>
            <button role="tab" aria-selected={viewMode === "configure"} className={viewMode === "configure" ? styles.segActive : styles.seg} onClick={() => setViewMode("configure")}>配置</button>
          </div>
          {viewMode === "observe" && <ObserveContent {...props} onDrilldown={setDrilldown} />}
          {viewMode === "configure" && <ConfigureContent {...props} />}
        </>
      ) : (
        <DrilldownHost drilldown={drilldown} onBack={() => setDrilldown(null)} {...props} />
      )}
    </section>
  );
}
```
> `ObserveContent` / `ConfigureContent` / `DrilldownHost` 本 Task 内先作为**同文件内的临时函数**，把现有 6 tab 的 JSX 搬进去（见 Step 2）。Task 4/5 再抽成独立文件。

CSS Module（`cockpit.module.css`）——段控用 tokens.css var，不硬编码：
```css
.cockpitPanel { display: grid; gap: 14px; }
.segmented { display: inline-flex; gap: 4px; background: var(--surface-soft); border: 1px solid var(--line); border-radius: var(--r-sm); padding: 3px; width: fit-content; }
.seg, .segActive { border: none; background: transparent; padding: 6px 16px; border-radius: calc(var(--r-sm) - 3px); font-size: 13px; cursor: pointer; color: var(--muted); }
.segActive { background: var(--surface-card, #fff); color: var(--ink); font-weight: 640; box-shadow: 0 1px 2px rgba(0,0,0,0.06); }
```
> 先 Read `frontend/src/components/ui/tokens.css` 确认 `--surface-soft`/`--line`/`--r-sm`/`--muted`/`--ink` 确切存在的变量名，按实际改（缺失就用最接近的已存在 token，别硬编码 hex）。

- [ ] **Step 2: 把现 6 tab 内容搬进 ObserveContent/ConfigureContent/DrilldownHost**

Read `legacy.tsx:318-714` 现 6 个 `activeTab === "..."` 块的完整 JSX。搬迁映射：
- `ObserveContent` ← `cockpit` 块（:318-440）：agentBehaviorGrid + 运营记忆编辑器**移出**（记忆编辑器归 ConfigureContent）+ Agent 当前判断 + 标签可信度 + 人格 + 健康度 + 长期记忆卡片 + Planner + 发送历史。**发送历史改为下钻入口**（点击 `onDrilldown("sendHistory")`）。
- `ConfigureContent` ← `adjust`（:442-488）+ `profile`（:490-639）+ `memory`（:641-671，编辑部分）+ `simulation`（:673-696）+ 运营记忆编辑器（从 cockpit 块 :339-372 移来）。用内部小标题或 `<details>` 分组，别再引入第三层 tab。
- `DrilldownHost` ← `conversation`（:698-714）作为 `"conversation"` 分支；`sendHistory` 分支复用 `SendHistorySection`（:2280）；`memory` 分支 Task 5 做，先占位空 section + 返回按钮。
> 这一步是**verbatim 搬迁现有 JSX + 重接 props**，不新写逻辑。所有 `on*` 回调、`memoryDraft`/`guidePreview` 等数据引用保持不变。搬迁时逐段核对 legacy 源，不凭记忆。

- [ ] **Step 3: index.tsx 切换到 CockpitPanel**

`frontend/src/features/user-ops/index.tsx`：import 从 `./legacy` 的 `UserOperationCockpit` 改为 `./cockpit/CockpitPanel` 的 `CockpitPanel`（:4 import 行 + :291 使用处）。props 传递不变（现 :291-338 那份全传）。`activeTab`/`smartOpsTab`/`onTab`/`setSmartOpsTab` 相关 props CockpitPanel 不再需要，从传递中移除（段控自管 state）——同步 store 若有 `smartOpsTab` 遗留可保留不清（Task 6 清理）。

- [ ] **Step 4: dev server 验证行为等价**

Run: `cd frontend && npm run dev`（后台起），在浏览器打开用户运营频道 smart 模式，选一个 managed 联系人。
Expected（逐项点验）：观测段能看到 AI 判断/健康度/标签/人格/记忆卡/Planner；配置段能编辑运营记忆 23 字段 + 画像 + AI 调整 + 模拟；点发送历史进下钻能返回；会话记录可达。所有编辑保存按钮仍触发原有保存（不报错）。**这是 UI 任务，必须浏览器实测，不能只靠编译过。**

- [ ] **Step 5: 前端类型检查 + 现有测试**

Run: `cd frontend && npx tsc --noEmit 2>&1 | tail -15`（0 error）；`cd frontend && npx vitest run src/__tests__/features/user-ops 2>&1 | tail -20`。
Expected: tsc 过。现有 user-ops 测试若断言旧 `UserOperationCockpit` 结构会失败——本 Task 先记录哪些失败，Task 6 统一更新（或本步就地更新断言到新结构，二选一，倾向本步就地修断言避免积压）。

- [ ] **Step 6: 禁用词 lint + Commit**

Run: `bash scripts/check-no-human-takeover.sh HEAD 2>&1 | tail -3`（0 violations）。
```bash
git add frontend/src/features/user-ops/cockpit/ frontend/src/features/user-ops/index.tsx
git commit -m "refactor(user-ops): 驾驶舱改段控三段式(CockpitPanel+观测/配置),行为等价迁移"
```

---

### Task 3: JudgmentBar 判断条（能力上前台核心）

**Files:**
- Create: `frontend/src/features/user-ops/cockpit/JudgmentBar.tsx`
- Modify: `frontend/src/features/user-ops/cockpit/CockpitPanel.tsx`（挂 JudgmentBar 替换 Step 1 的 panelHead 占位）
- Modify: `frontend/src/features/user-ops/cockpit/cockpit.module.css`（chip 样式）
- Modify: `frontend/src/stores/userOpsStore.ts`（新增 `escalationPendingCount` + loader，判断条请示灯）
- Test: `frontend/src/__tests__/features/user-ops/judgmentBar.test.tsx`（新建）

**Interfaces:**
- Consumes: `decisionReviews`（store 已有，`userOpsStore.ts:389` 拉 `/api/decision-reviews`）、`operationHealth`（store 已有）、Contact（`lastConversationMode`）、新增 `escalationPendingCount`。
- Produces: `<JudgmentBar contact={} latestReview={} health={} escalationCount={} onRiskClick={} />`，6 chips。

**背景**：现驾驶舱「最近复盘」只用 `review.approved ? "通过":"拦截"`（`legacy.tsx:706`）——本 Task 用 finalReviewStatus 精确 10 态映射取代。`FINAL_REVIEW_STATUS_LABELS` 中文映射在 `operations/index.tsx:139` 与 `autonomy/index.tsx:361` 已存在，**复用不新造**（Read 确认其定义位置，import 复用；若不便跨 feature import 则在 cockpit 内定义一份同值的，避免耦合——倾向复用）。

- [ ] **Step 1: 写映射 + chip 数据提取的单测（TDD）**

新建 `judgmentBar.test.tsx`。先定义纯函数 `finalReviewTone(status?: string): "sent"|"held"|"blocked"|"other"` 并测：
```tsx
import { finalReviewTone } from "../../../features/user-ops/cockpit/JudgmentBar";
describe("finalReviewTone", () => {
  it("approved 系 → sent", () => {
    expect(finalReviewTone("approved")).toBe("sent");
    expect(finalReviewTone("revision_applied_approved")).toBe("sent");
  });
  it("暂缓系 → held", () => {
    expect(finalReviewTone("held_by_ai_policy")).toBe("held");
    expect(finalReviewTone("ai_waiting_for_more_context")).toBe("held");
  });
  it("拦截系 → blocked", () => {
    ["blocked_by_safety_guard","blocked_by_required_field","blocked_by_budget","blocked_unverified_product_claim","revision_failed"].forEach(s =>
      expect(finalReviewTone(s)).toBe("blocked"));
  });
  it("legacy/未知/缺失 → other", () => {
    expect(finalReviewTone("legacy_mode_unchecked")).toBe("other");
    expect(finalReviewTone(undefined)).toBe("other");
    expect(finalReviewTone("some_future_value")).toBe("other");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/judgmentBar.test.tsx 2>&1 | tail -15`
Expected: 失败（`finalReviewTone`/JudgmentBar 未定义）。

- [ ] **Step 3: 实现 JudgmentBar**

`JudgmentBar.tsx`。`finalReviewTone` 按上面分组硬编码 10 态（这是闭集，穷举合理，非魔法值）。chip 渲染：
```tsx
export function finalReviewTone(status?: string): "sent" | "held" | "blocked" | "other" {
  switch (status) {
    case "approved":
    case "revision_applied_approved": return "sent";
    case "held_by_ai_policy":
    case "ai_waiting_for_more_context": return "held";
    case "blocked_by_safety_guard":
    case "blocked_by_required_field":
    case "blocked_by_budget":
    case "blocked_unverified_product_claim":
    case "revision_failed": return "blocked";
    default: return "other"; // legacy_mode_unchecked / 未知 / 缺失
  }
}
```
组件：人格态 chip（`labelFor(taxonomies,"conversation_mode",contact.lastConversationMode)` 翻译，紫底 class；无值不显）；最近轮 chip（`FINAL_REVIEW_STATUS_LABELS` 取中文 + `finalReviewTone` 决定 sent/held/blocked/other 颜色 class；无 review 显「尚无决策记录」）；下一步 chip（复用 `nextBestActionLabel(latestReview?.nextBestAction)`，缺回落「等待用户消息」）；风险灯（`health?.items?.some(i => i.tone === "danger")` 亮红，`onClick={onRiskClick}`）；作息灯（`health?.inQuietHours` 时显「客户休息时段留言，将在 {格式化 health.nextWakeAt} 后统一回复」；`health?.inQuietHours` 为 undefined/false 不渲染此 chip——优雅降级）；请示灯（`escalationCount > 0` 亮蓝可点，跳现有 ask-human/autonomy 频道入口）。
> `labelFor` / `nextBestActionLabel` 从现有位置 import（Read 确认路径：`nextBestActionLabel` 在 `legacy.tsx:2085`，`labelFor` 在 profileStore 相关——若 legacy 的不便复用就搬成共享 util）。CSS chip 颜色用 tokens.css：sent=`--fill-running`、held=`--fill-held`、blocked=`--fill-blocked`、人格紫=`--fill-brand`（Read tokens.css 确认这些 fill token 存在）。

- [ ] **Step 4: store 加 escalationPendingCount**

`userOpsStore.ts`：加 state `escalationPendingCount: number`（默认 0）+ loader `loadEscalationCount()` 拉 `GET /api/admin/ask-human/summary`，取 `principalEscalation` 字段。在 `openContact`/挂载时调（或复用现有 hydrate 时机）。Read 现有 loader 写法（如 `loadMessages` :389）照抄风格。

- [ ] **Step 5: CockpitPanel 挂 JudgmentBar + 跑测试 + dev 验证**

CockpitPanel 顶部渲染 `<JudgmentBar contact={selected} latestReview={decisionReviews[0]} health={operationHealth} escalationCount={escalationPendingCount} onRiskClick={() => setViewMode("observe")} />`（风险灯点击切观测段）。
Run: `npx vitest run src/__tests__/features/user-ops/judgmentBar.test.tsx`（PASS）；`npx tsc --noEmit`（0 error）；dev server 浏览器看判断条渲染正确（换不同状态的联系人验证颜色）。

- [ ] **Step 6: 禁用词 lint + Commit**

Run: `bash scripts/check-no-human-takeover.sh HEAD 2>&1 | tail -3`（0 violations——「AI 策略主动暂缓」等文案无"人工"）。
```bash
git add frontend/src/features/user-ops/cockpit/ frontend/src/stores/userOpsStore.ts frontend/src/__tests__/features/user-ops/judgmentBar.test.tsx
git commit -m "feat(user-ops): JudgmentBar 判断条(finalReviewStatus/人格态/风险/作息/请示上前台)"
```

---

### Task 4: ObserveView 增强 + 抽独立文件

**Files:**
- Create: `frontend/src/features/user-ops/cockpit/ObserveView.tsx`（从 CockpitPanel 的 ObserveContent 抽出）
- Modify: `frontend/src/features/user-ops/cockpit/CockpitPanel.tsx`
- Modify: `frontend/src/features/user-ops/TagTrustPanel.tsx`（三层信任视觉区分增强）
- Test: `frontend/src/__tests__/features/user-ops/observeView.test.tsx`（新建）

**Interfaces:**
- Consumes: Contact / OperationHealth / operatingMemory / personalityProfile / bayesian。
- Produces: `<ObserveView {...props} onDrilldown={} />` 6 只读卡。

**背景**：健康度卡直接 map `health.items`，tone 三色用 tokens.css fill（不硬编码现 `#b8ddd8` 等）。标签三层 = manual_tags（人工权威）/ confirmed_tags（AI 确信，可展开 evidence 轮次）/ taxonomy_candidate（待审）。

- [ ] **Step 1: 抽 ObserveView + 健康度 tone 用 token**

把 CockpitPanel 内 ObserveContent 抽成 `ObserveView.tsx`。健康度卡改为：
```tsx
{(health?.items ?? []).map((item) => (
  <div key={item.key} className={styles[`health_${item.tone}`]}>
    <strong>{item.label}</strong><span>{item.score}</span>
    <p>{item.detail}</p>
  </div>
))}
```
cockpit.module.css 加 `.health_good{background:var(--fill-running)} .health_warn{background:var(--fill-held)} .health_danger{background:var(--fill-blocked)}`（Read tokens.css 确认 fill 变量名）。

- [ ] **Step 2: 写 ObserveView 渲染测试（TDD）**

`observeView.test.tsx`：mock 一个 health（含 good/warn/danger 三项），断言渲染出 3 项且带对应 tone class；断言点击"记忆要点"卡触发 `onDrilldown("memory")`。
```tsx
it("健康度按 tone 渲染 + 卡点击下钻", () => {
  const onDrilldown = vi.fn();
  render(<ObserveView health={{items:[{key:"a",label:"理解",score:80,tone:"good",detail:"x"}], inQuietHours:false} as any} selected={fakeContact} onDrilldown={onDrilldown} /* ...其余必填 props mock */ />);
  expect(screen.getByText("理解")).toBeInTheDocument();
  fireEvent.click(screen.getByTestId("observe-memory-card"));
  expect(onDrilldown).toHaveBeenCalledWith("memory");
});
```

- [ ] **Step 3: 跑失败 → 实现 → 跑通**

Run: `npx vitest run src/__tests__/features/user-ops/observeView.test.tsx 2>&1 | tail -15`（先失败）。给记忆卡加 `data-testid="observe-memory-card"` + `onClick={() => onDrilldown("memory")}`，人格/贝叶斯卡加 `onClick={() => onDrilldown("conversation")}`（或独立 bayesian 下钻，本期并入 conversation 复盘视图或单列，按 Task 5 定）。再跑 PASS。

- [ ] **Step 4: TagTrustPanel 三层增强**

先 Read `TagTrustPanel.tsx` 全文（135 行）看现有渲染。增强为三分区：人工权威（manual_tags，蓝/权威视觉）、AI 确信（confirmed_tags，每条可展开看 `evidences` 的 turn/msgId）、待审候选（若 props 有 taxonomy candidate 数据源则显，无则略）。视觉三层用 tokens.css，不硬编码。保持现有 `onSaveManualTags` 回调不变。

- [ ] **Step 5: tsc + dev 验证 + Commit**

Run: `npx tsc --noEmit`（0）；`npx vitest run src/__tests__/features/user-ops`（相关 PASS）；dev server 看观测段健康度三色正确、标签三层可辨、卡点击进下钻。
```bash
git add frontend/src/features/user-ops/
git commit -m "feat(user-ops): ObserveView 抽出+健康度tone用token+标签三层信任区分"
```

---

### Task 5: 下钻视图（记忆溯源 / 决策复盘 / 发送历史）

**Files:**
- Create: `frontend/src/features/user-ops/cockpit/drilldowns/MemoryDetailView.tsx`
- Create: `frontend/src/features/user-ops/cockpit/drilldowns/ConversationReviewView.tsx`
- Create: `frontend/src/features/user-ops/cockpit/drilldowns/SendHistoryView.tsx`（迁移现 `SendHistorySection` :2280）
- Modify: `frontend/src/features/user-ops/cockpit/CockpitPanel.tsx`（DrilldownHost 分发）
- Test: `frontend/src/__tests__/features/user-ops/memoryDetail.test.tsx`（新建）

**Interfaces:**
- Produces: 三个下钻视图，各含 `onBack` 返回按钮。DrilldownHost 按 `drilldown` 值分发。

**背景**：记忆全景要展示 `MemoryFact` 溯源字段（confidence/importance/source_message_ids/deprecated_at+reason）——现 `MemoryCardSummary`（`legacy.tsx:738-792`）把它当纯文本渲染，浪费了结构。决策复盘要展示自治协议 9 字段（why_should_reply/self_critique/user_understanding 等，来自 decisionReviews/run log）——现完全没展示。

- [ ] **Step 1: MemoryDetailView 溯源渲染 + 测试**

先 Read `legacy.tsx:738-914`（MemoryCardSummary + memoryFactList + MemoryFactView 类型）确认 fact 结构。`memoryDetail.test.tsx`：mock 一条带 confidence/importance/deprecatedAt 的 fact，断言这些字段被渲染（而非只显 text）：
```tsx
it("记忆事实展示溯源字段", () => {
  render(<MemoryDetailView memoryCard={{coreFacts:[{text:"客户偏好微信沟通", confidence:8, importance:9, deprecatedAt:null}]} as any} onBack={()=>{}} />);
  expect(screen.getByText(/客户偏好微信沟通/)).toBeInTheDocument();
  expect(screen.getByText(/8/)).toBeInTheDocument(); // confidence
});
```

- [ ] **Step 2: 跑失败 → 实现三下钻 → 跑通**

- `MemoryDetailView`：复用 `memoryFactList`（Read 其位置，import 或搬 util），每条 fact 渲染 text + confidence/importance 徽标 + 弃用则显 deprecatedAt/reason。分区 coreFacts/recentFacts/deprecatedFacts + preferences/objections/commitments/doNotDo（复用现 MemoryCardSummary 的 section 划分 :744-751）。
- `ConversationReviewView`：现 `conversation` 块（`legacy.tsx:698-714`）的 ConversationStream + reviewList 搬来，reviewList 每条 review 增展开显示自治协议字段（`review.userUnderstanding`/`selfCritique`/`whyShouldReply` 等——Read `types/index.ts` 的 DecisionReview 类型确认哪些字段可用，只显存在的）。
- `SendHistoryView`：现 `SendHistorySection`（:2280）verbatim 迁移 + 加 onBack。
Run: `npx vitest run src/__tests__/features/user-ops/memoryDetail.test.tsx`（PASS）。

- [ ] **Step 3: DrilldownHost 分发 + tsc + dev 验证**

CockpitPanel 的 DrilldownHost：`drilldown==="memory"` → MemoryDetailView；`"conversation"` → ConversationReviewView；`"sendHistory"` → SendHistoryView。各传 `onBack={() => setDrilldown(null)}`。
Run: `npx tsc --noEmit`（0）；dev server 验证三下钻进入/返回、记忆溯源字段可见、决策复盘展开看到 AI 内心独白。

- [ ] **Step 4: 禁用词 lint + Commit**

Run: `bash scripts/check-no-human-takeover.sh HEAD 2>&1 | tail -3`（0）。
```bash
git add frontend/src/features/user-ops/cockpit/
git commit -m "feat(user-ops): 下钻视图(记忆溯源/决策复盘自治协议/发送历史)"
```

---

### Task 6: 现有测试更新 + legacy 清理 + 全量回归

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`（删已迁移的 `UserOperationCockpit` 及其私有子组件，保留 ContactsView/UserOpsModeHeader/TraditionalOpsTabs 及仍被复用的）
- Modify: 失效的现有测试（`__tests__/features/user-ops/`、`CockpitView.test.tsx` 等）
- Modify: `frontend/src/stores/userOpsStore.ts`（清 `smartOpsTab`/`setSmartOpsTab` 遗留，若已无引用）

**背景**：`UserOperationCockpit` 内容已全部迁入 cockpit/，本 Task 删死代码 + 修因重构失效的测试断言。删前 grep 确认无其它引用。

- [ ] **Step 1: grep 确认无引用后删除 legacy 死代码**

Run: `grep -rn "UserOperationCockpit\|SmartOpsTabs\|smartOpsTab" frontend/src --include=*.tsx --include=*.ts | grep -v cockpit/ | grep -v __tests__`
逐个确认：`UserOperationCockpit`/`SmartOpsTabs` 应只剩 legacy.tsx 自身定义（已无外部引用）→ 删除其定义。`smartOpsTab` 若 store/index 已无用 → 删。`MemoryCardSummary`/`PlannerViewSection`/`SendHistorySection`/`ChangePreview` 等：若已被 cockpit/ 复用（import from legacy）则保留导出；若已搬进 cockpit/ 则删 legacy 版。**逐个 grep 定夺，不批量删。**

- [ ] **Step 2: 更新失效的现有测试**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops 2>&1 | tail -30`
对每个失败测试：若它断言旧 tab 结构（`activeTab`/SmartOpsTabs），更新为新段控/下钻结构的等价断言（测同一业务意图，非删测试）。`plannerView.test.tsx`/`personalityPanel.test.tsx`/`tagTrustPanel.test.tsx`/`bayesianTrendChart.test.tsx` 若测的是独立子组件应基本不受影响，确认。

- [ ] **Step 3: 全量前端 + 后端回归**

Run:
- `cd frontend && npx tsc --noEmit 2>&1 | tail -10`（0 error）
- `cd frontend && npx vitest run 2>&1 | tail -15`（全绿）
- `cargo test --lib 2>&1 | tail -5`（≥350/0，确认 Task 1 后端不回归）
- `cd frontend && npm run build 2>&1 | tail -10`（构建过——验证无 tree-shake 导致的样式丢失，CSS Module 引用完整）

- [ ] **Step 4: dev server 完整走查 + 禁用词全量**

dev server 完整点验：判断条 6 chip / 观测 6 卡 / 配置全编辑 / 3 下钻进返 / 换 managed 与 normal 联系人 / 空态。
Run: `bash scripts/check-no-human-takeover.sh origin/main HEAD 2>&1 | tail -3`（全分支 0 violations）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/user-ops/ frontend/src/stores/userOpsStore.ts
git commit -m "refactor(user-ops): 清理 legacy 驾驶舱死代码+更新重构失效测试"
```

---

## Self-Review

**1. Spec 覆盖**（逐 § 核）：
- §3 文件结构（cockpit/ 7 文件）→ Task 2（CockpitPanel/cockpit.module.css）+ Task 3（JudgmentBar）+ Task 4（ObserveView）+ Task 5（3 下钻）✅（ConfigureView 在 Task 2 作 ConfigureContent，未单独抽文件——若 reviewer 认为该独立成文件，Task 4 类比抽出即可；spec §3 列了 ConfigureView.tsx，**补：Task 2 Step 2 应把 ConfigureContent 落成 `ConfigureView.tsx` 独立文件**，见下方修正）
- §4 判断条 6 chip → Task 3 ✅
- §5 观测/配置分离 → Task 2 迁移 + Task 4 增强 ✅
- §6 后端 3 字段 → Task 1 ✅
- §7 store（escalationPendingCount）→ Task 3 Step 4 ✅
- §8 测试 → 各 Task 内 vitest + Task 6 回归 ✅
- §9 不做边界 → 计划无 Task 触碰传统模式/knowledge cockpit/全局 token ✅

**修正（Self-Review 发现）**：spec §3 明确列 `ConfigureView.tsx` 为独立文件。Task 2 Step 1/2 把 `ConfigureContent` 作同文件临时函数——**执行时应直接落成 `cockpit/ConfigureView.tsx` 独立文件**（与 ObserveView 对称），Task 4 只抽 ObserveView 是笔误，ConfigureView 在 Task 2 就该独立。实现者按此调整：Task 2 建 CockpitPanel + ObserveView + ConfigureView 三文件骨架（内容搬迁），Task 4 只做 ObserveView 增强。

**2. Placeholder 扫描**：Task 1 backend 有完整验证代码；Task 3 finalReviewTone 完整；migration 步骤用"Read 源 X-Y 行 verbatim 搬迁"而非 TBD——这是对已存在验证代码的搬迁指令，非占位。测试步骤均有具体断言代码。CSS 变量名标注"Read tokens.css 确认"因三源 token 不一致（Spec B 未做），实现期必须现读实际变量——这是红线要求的亲验，非偷懒占位。

**3. 类型一致性**：`finalReviewTone` 返回 `"sent"|"held"|"blocked"|"other"` 全程一致；`ViewMode`/`Drilldown` 类型 Task 2 定义、Task 3/5 消费一致；`CockpitPanel` props = 现 `UserOperationCockpit` props（Task 2 抄 legacy.tsx:231-278）。

## Execution Handoff

计划已存到 `docs/superpowers/plans/2026-07-01-user-ops-cockpit-redesign.md`。
