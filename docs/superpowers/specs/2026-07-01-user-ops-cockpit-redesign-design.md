# 用户运营驾驶舱前端重设计（Spec A：信息架构重构 + 能力上前台）

> 日期：2026-07-01
> 类型：前端为主（React + Vite + TypeScript）+ 一个后端小改（operation-health 补 3 字段）
> 范围：**Spec A**。全局 token 三源统一（docs / tokens.css / styles.css 数值不一致）是独立的 **Spec B**，本次不做。
> 前置：基于最新 main（commit 4bdecd9）100% 亲验。前端现状、设计系统规范、后端能力盘点三份调研 + 6 个设计阻塞缺口均已逐条核过（每处结论带 file:line）。

## 0. 背景与问题

「用户运营」频道 smart 模式的右栏驾驶舱（`UserOperationCockpit`，`frontend/src/features/user-ops/legacy.tsx:184-717`）存在两个问题：

1. **太长、不易读**：`cockpit` 这个 tab 纵向堆了 9 个 section（`legacy.tsx:318-440`），其中最重的是一个 **23 字段的运营记忆编辑表单**（`MEMORY_DRAFT_FIELD_GROUPS`，`legacy.tsx:84-136`）被塞进「看判断和风险」的查看型 tab——查看与编辑语义混杂是过长的根因。同一信息（下一步/避免事项/领域信号）在 cockpit 只读卡、运营记忆、profile tab 三处重复出现。
2. **后端能力远超前端表达**：项目做了大量新能力，但驾驶舱没跟上。最典型——「最近复盘」只用 `review.approved ? "通过" : "拦截"`（`legacy.tsx:706`）这种二元判断，而后端每轮产出的 `finalReviewStatus`（10 态精确枚举）、`conversation_mode`（AI 人格态）、记忆事实溯源、贝叶斯走势、标签三层信任等能力，前端要么埋在最底部要么完全没展示。

**核心定位**（贯穿全设计）：这是「全 AI 自治」产品——客户永远只跟 AI 对话，运营在幕后。驾驶舱的核心动作是 **观测**（AI 做了什么判断、对不对、有没有该关注的风险）+ **偶尔干预**（调指令 / 记忆 / 模式），运营不下场跟客户聊。所以重设计的主轴 = **把「观测」与「配置」分离**，并把关键 AI 判断顶到前台。

## 1. 已亲验的关键事实（设计地基，全部带 file:line）

### 1.1 finalReviewStatus 10 态闭集（`src/agent/run_envelope.rs:67-78`）
`approved` / `revision_applied_approved`（已发送，绿）；`held_by_ai_policy`（AI 策略暂缓，橙）；`ai_waiting_for_more_context`（缺信息等待，橙）；`blocked_by_safety_guard` / `blocked_by_required_field` / `blocked_by_budget` / `blocked_unverified_product_claim` / `revision_failed`（拦截，红）；`legacy_mode_unchecked`（灰）。
- **前端已能拿到**：`userOpsStore.ts:389` 已在拉 `/api/decision-reviews?contactId=...`；`DecisionReview.finalReviewStatus` 类型已存在（`types/index.ts:300`）。
- **中文映射资产已存在**：`operations/index.tsx:139` 和 `autonomy/index.tsx:361` 都用 `FINAL_REVIEW_STATUS_LABELS`。驾驶舱直接复用，不新造。
- 禁用值（`held_for_human` 等）写库即阻断（`run_envelope.rs:140-146`），前端永不会收到，无需映射。

### 1.2 OperationHealth 结构（`src/routes/shared.rs:459-514`，端点 `src/routes/contacts.rs:1075-1088`）
返回 `{ scores, items }`。`items` 每项 `{ key, label(中文), score(0-100), tone, detail(中文) }`。**7 个固定 item**：userUnderstanding / relationshipQuality / productFit / rhythmRisk / knowledgeGrounding / hallucinationRisk / pressureRisk。`tone` 三值闭集 `good`/`warn`/`danger`，**后端已按 key 是否 `Risk` 结尾自动反转判定方向**（`shared.rs:491-506`）——前端不算风险方向，直接用 tone 上色。已加载。

### 1.3 conversation_mode 四态 + hold_category 三态（`src/agent/types.rs`）
- `conversation_mode`（:224-232）：`casual_relationship` / `value_exchange` / `consultative` / `boundary_protection`，默认 `casual_relationship`。**可被 DomainProfile 覆盖**（:687-703），前端遇未知值须兜底显原值 + `conversation_mode_reason`。前端已通过 taxonomy 字典翻译渲染（`PlannerViewSection`，`legacy.tsx:2202`）。
- `hold_category`（:1222-1231）三态是 finalReviewStatus 的子集/来源，复用同套颜色。

### 1.4 next_best_action 无 schema（`src/agent/types.rs:134`）
后端是自由 `Document`，不校验内部结构。前端 `nextBestActionLabel`（`operations/index.tsx:61-66` 与 `legacy.tsx:2085-2090` 两份相同）只认 `type`(string)/`score`(number)，渲染 `"{type} / {score}"`，缺则 `-`。→ chip 做成「有 type 就显、无则回落文案」，不强求后端补 schema。

### 1.5 quiet_hours 后端已算但未暴露（`src/agent/quiet_hours.rs`）
`in_quiet_hours` / `is_quiet_now`（:88）/ `next_wake_at`（:99）/ `effective_quiet_hours_enabled`（:128）全是 `pub(crate)` 纯函数，**全项目 routes 层零命中**，无 GET 端点暴露。调用形态（`gateway.rs:945/3039-3052`、`webhooks.rs:596`）：
- `is_quiet_now(runtime.quiet_hours_start, runtime.quiet_hours_end, runtime.quiet_hours_tz_offset_hours)`
- `next_wake_at(runtime.quiet_hours_end, runtime.quiet_hours_tz_offset_hours, &contact.wxid, config.wake_jitter_max_seconds)`
- `effective_quiet_hours_enabled(&contact, &profile, runtime.quiet_hours_enabled)`——`profile` 来自 `load_active_domain_profile(&db, workspace_id)`（`domain_profile.rs:978`）。
- `runtime` 由 `UserRuntimeParameters::from_config(domain_config, state)` 构建（`runtime.rs:126`）。
- **关键**：`get_operation_health`（`contacts.rs:1080-1082`）现在只加载 contact/memory/latest_review，**没有 runtime，也没有 profile**。要补作息字段，端点需额外加载 domain_config（→runtime）+ profile。→ 后端 task 是「中等」不是「trivial」。

### 1.6 请示端点：ask-human 是聚合层，principal-escalations 是专属层
- `GET /api/admin/ask-human/summary`（`ask_human_inbox.rs:452`）→ 8 源 pending 计数，含 `principalEscalation`。**判断条「请示灯」用这个的计数，单请求最省。**
- `GET /api/admin/principal-escalations?status=pending`（`principal_escalations.rs:25`）字段最全（questionForPrincipal/decision/authorizationExpiresAt），裁决收件箱用这个。本次驾驶舱只做「灯 + 计数 + 点击跳转」，不在驾驶舱内做裁决。

### 1.7 承载文件与 CSS
- `legacy.tsx` 2365 行，`UserOperationCockpit` :184-717。`ContactsView`/`UserOpsModeHeader`/`TraditionalOpsTabs`（传统模式）在同文件，本次不动。
- CSS：user-ops 走全局 `styles.css`（`userCockpitGrid`/`smartTabPanel`/`cockpitSection` 等），而语义色**硬编码 hex**（`healthGrid.good/.warn/.danger` = `#b8ddd8`/`#fedf89`/`#fda29b`），违反 tokens.css「禁止硬编码色值」。
- **tree-shake 坑**（有 memory 记录）：绝不把全局字符串 className 的 CSS 命名为 `.module.css` 再副作用导入 → Rollup 摇树删光样式。新组件用 CSS Modules（`import styles from "./x.module.css"` + `className={styles.x}`）才安全。
- 设计系统色纪律：紫（`--color-brand`）=AI 身份，不表达状态；蓝（`--color-scheduled`）=主操作/可点击；绿=running；橙=held；红=blocked。四级层级，不做第三层持久导航。

## 2. 目标架构

右栏驾驶舱三段式：**常驻判断条** + **段控（观测 / 配置）** + **下钻视图**。左栏 `ContactsView` 保留不动。

```
右栏 CockpitPanel
├─ JudgmentBar（常驻）：[人格态][最近轮][下一步][风险灯][作息灯][请示灯]
├─ 段控：[观测] | [配置]
├─ ObserveView（默认，只读卡网格，卡可点下钻）
├─ ConfigureView（编辑动作，从观测剥离）
└─ 下钻视图（右栏内切换 + 返回）：记忆全景 / 会话+决策复盘 / 发送历史
```

## 3. 文件结构（legacy.tsx 拆分）

新建 `frontend/src/features/user-ops/cockpit/`：

| 文件 | 职责 |
| --- | --- |
| `CockpitPanel.tsx` | 右栏容器，替代现 `UserOperationCockpit` 外壳；管 观测/配置/下钻 的视图状态；挂 JudgmentBar |
| `JudgmentBar.tsx` | 常驻判断条（6 chips） |
| `ObserveView.tsx` | 观测模式（6 只读卡网格） |
| `ConfigureView.tsx` | 配置模式（编辑动作聚合） |
| `drilldowns/MemoryDetailView.tsx` | 记忆全景（fact 溯源） |
| `drilldowns/ConversationReviewView.tsx` | 会话流 + 决策复盘（自治协议 9 字段） |
| `drilldowns/SendHistoryView.tsx` | 发送历史（迁移现 `SendHistorySection`） |
| `cockpit.module.css` | scoped 样式，**引用 tokens.css 的 var()，不硬编码色** |

复用现有独立组件：`BayesianTrendChart.tsx` / `PersonalityPanel.tsx` / `TagTrustPanel.tsx`（各自已有 `.module.css`）。`legacy.tsx` 保留 `ContactsView`/`UserOpsModeHeader`/`TraditionalOpsTabs`；`UserOperationCockpit` 及其私有子组件（`MemoryCardSummary`/`ChangePreview`/`PlannerViewSection`/`SmartOpsTabs` 等）迁入 cockpit/ 或按需保留复用。`user-ops/index.tsx` 把 `<UserOperationCockpit .../>` 换成 `<CockpitPanel .../>`（props 收敛，见 §7）。

## 4. JudgmentBar（判断条）

一行常驻 chip，数据源全部已核可达（除作息灯依赖 §6 后端补字段）：

| chip | 数据源 | 表达 / 颜色 |
| --- | --- | --- |
| 人格态 | `contact.lastConversationMode` 经 taxonomy 翻译（`labelFor(taxonomies,"conversation_mode",...)`，同 PlannerView） | 紫底（`--color-brand`=AI 身份）；未知值兜底显原值 |
| 最近轮 | `decisionReviews[0].finalReviewStatus` + 复用 `FINAL_REVIEW_STATUS_LABELS` | 绿已发 / 橙暂缓 / 红拦截 / 灰其它（按 §1.1 分组）；无 review 时显「尚无决策记录」 |
| 下一步 | `decisionReviews[0].nextBestAction` 经 `nextBestActionLabel`（认 type/score） | 有 type 就显，缺则回落「等待用户消息」 |
| 风险灯 | operation-health `items` 中 tone=`danger` 的项 | 有 danger 亮红，点击 → 观测模式健康度卡 |
| 作息灯 | **需后端补**（§6）`inQuietHours` / `nextWakeAt` | 静默中显「客户休息时段留言，将在 X 点后统一回复」；**拿不到字段则不渲染此 chip（优雅降级，不报错）** |
| 请示灯 | `GET /api/admin/ask-human/summary` 的 `principalEscalation` 计数 | `>0` 亮蓝可点 → 跳请示收件箱（现有 ask-human/autonomy 频道入口）；`0` 不显 |

## 5. 观测模式 / 配置模式

### 5.1 ObserveView（只读仪表盘，卡可点下钻）
段控 `[观测] | [配置]` 替代现 6 个平级 `SmartOpsTabs`。观测默认，6 只读卡：

1. **运营健康度** — operation-health 7 项，tone 三色（用 tokens.css 的 `--fill-running/--fill-held/--fill-blocked`，不硬编码），score 0-100 条。
2. **标签信任** — 三层视觉分：人工权威（`manual_tags`）/ AI 确信（`confirmed_tags`，可展开看 evidence 轮次）/ 待审候选（taxonomy_candidate）。基于现 `TagTrustPanel` 增强其三层区分。
3. **AI 判断要点** — 用户理解 / 当前运营状态 / 领域信号（现 cockpit profileGrid 6 格精简为只读要点）。
4. **记忆要点** — `MemoryCardSummary` 缩略，点 → 记忆全景下钻。
5. **人格 + 贝叶斯缩略** — `PersonalityPanel`（OCEAN）+ `BayesianTrendChart` 迷你，点 → 走势下钻。
6. **Planner** — 阶段 / 承诺（现 `PlannerViewSection`）。

### 5.2 ConfigureView（编辑动作，从观测剥离——根治过长）
现塞在「看判断」tab 的编辑内容全部移入此模式：
- 运营记忆 23 字段（`MEMORY_DRAFT_FIELD_GROUPS`，按 4 组折叠：用户理解 / 关系状态 / 产品契合 / 下一步动作）。
- 画像备注 / 自定义指令（`customAgentInstructions`，≤1000）/ 客户类型（relationshipType）/ 最近承诺 / 跟进策略。
- 辅助模式引荐开关（assistOverride）+ 撤销引荐。
- AI 自然语言调整（guide preview / apply）。
- 影子模拟（simulation）。

**内容与现 profile/adjust/memory/simulation tab 一致，只是重新归组到「配置」模式下**，行为、store action、后端调用不变。

## 6. 后端小改（Spec A 唯一后端 task）

`operation_health_json`（`src/routes/shared.rs:459`）响应顶层补 3 字段：`inQuietHours`(bool) / `nextWakeAt`(RFC3339 string | null) / `quietHoursEnabled`(bool)。`get_operation_health`（`contacts.rs:1075`）需额外：
1. 加载 domain_config → `UserRuntimeParameters::from_config(...)` 得 runtime；
2. `load_active_domain_profile(&state.db, &contact.workspace_id)` 得 profile；
3. 调现成纯函数：`effective_quiet_hours_enabled(&contact, &profile, runtime.quiet_hours_enabled)`、`is_quiet_now(start,end,tz_offset)`、`next_wake_at(end,tz_offset,&contact.wxid,config.wake_jitter_max_seconds)`（仅在 quiet 时算 nextWakeAt，否则 null）。

**红线**：只读聚合，**不碰 agent 决策 / gateway / 发送逻辑**，不改 quiet_hours 纯函数本身。参照现有加载 domain_config 的写法（`decision.rs:1069` `load_user_operation_domain_config`）复用，避免新造加载路径。

## 7. 数据流 / store

- `userOpsStore` 已加载 `decisionReviews`（`:389`）/ `operationHealth` / `operatingMemory` / `memoryCandidates`——基本复用。
- 新增：`ask-human/summary` 拉取（判断条请示灯计数），可在 store 加一个 `escalationPendingCount` + loader，或 CockpitPanel 局部 fetch（二选一，实现时定，倾向 store 统一）。
- `CockpitPanel` props 相比现 `UserOperationCockpit`（30+ props）应收敛：编辑类回调归入 ConfigureView 子树，观测类只读数据归入 ObserveView 子树，减少顶层 props 面。

## 8. 测试

- **前端 vitest**（复用现有 fixture，如 `decision_review.fixture.json`）：
  - `JudgmentBar`：finalReviewStatus 各态 → 正确中文 + 颜色分组；无 review → 兜底文案；作息字段缺失 → 不渲染作息 chip 不报错；请示计数 0/>0 显隐。
  - `ObserveView`：健康度 7 项按 tone 上色；标签三层区分渲染；卡点击触发下钻。
  - 段控切换 观测↔配置；下钻进入 + 返回。
  - `ConfigureView`：23 字段编辑回调仍正确触发 store action（回归保护）。
- **后端**：`operation_health_json` 补字段后的单测/集成（`inQuietHours`/`nextWakeAt`/`quietHoursEnabled` 存在且类型正确；非 quiet 时 nextWakeAt=null）。本机无 Docker → 集成测试留 CI，纯函数/序列化单测本地跑。
- **回归**：`cargo test --lib` ≥350/0；前端现有 user-ops 相关测试（`__tests__/features/user-ops/`、`CockpitView.test.tsx` 等）不回归——**注意**这些测试若断言现 `UserOperationCockpit` 的 DOM 结构/tab，会因重构失效，需同步更新（属重构必要连带，非为调绿而改）。

## 9. 不做（YAGNI 边界，明确排除）

- **全局 token 三源统一 = Spec B**，本次只做「user-ops 新组件引用 tokens.css var、不硬编码色」这一局部纪律，不动 docs/tokens.css/styles.css 的全局定义。
- **不碰传统模式**（`TraditionalOpsTabs` 及 playbooks/prompts/settings/audit 四 tab）。
- **不在驾驶舱内做请示裁决**（只做灯 + 跳转，裁决仍在现有 ask-human/autonomy 频道）。
- **不强求 next_best_action 后端补 schema**（chip 容忍缺失）。
- **不做 per-contact 三级覆盖链的完整可视化**（operation_mode_override 仍按现有单开关表达，深度可视化留后续）。
- **不改 knowledge/cockpit/**（那是知识库的驾驶舱，与 user-ops 无关，勿混）。

## 10. 全局约束（每 task 隐含遵守）

- React 19 + Vite + TS；CSS Modules（避免 tree-shake 坑，§1.7）。
- 遵守 `docs/frontend-design-system.md`：四级层级、颜色纪律（紫=AI身份/蓝=主操作/绿橙红=状态）、不做第三层持久导航、面板不套卡片。
- 禁用词 lint（`check-no-human-takeover.sh` 扫 `frontend/src/` 新增行）：不得出现 `人工` / `接管` / `takeover` / `hand-off` 等；AI 暂缓/等待用 AI 内部语义文案（如「AI 策略主动暂缓 / AI 等待更多上下文」）。
- 后端红线：AI 永不自动 verify；本次后端改动只读聚合，不触碰决策/发送。
- 测试基线不回归；新增测试 append，重构导致的现有测试更新如实标注为「重构连带」。

## 11. 执行顺序建议

1. **后端补字段**（§6，独立可测，前端作息灯依赖它）。
2. **拆分骨架**：建 cockpit/ 目录、CockpitPanel 外壳 + 段控，观测/配置视图搬迁（先保证行为等价、结构搬家）。
3. **JudgmentBar**（能力上前台核心）。
4. **ObserveView 增强**（健康度 tone、标签三层、卡下钻入口）。
5. **下钻视图**（记忆全景 fact 溯源、决策复盘自治协议、发送历史）。
6. **测试 + 现有测试更新 + 清理 legacy.tsx 残留**。
