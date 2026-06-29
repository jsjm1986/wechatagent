# 内容资产频道整理：文本资产分档注入 + 清理过期录入项

- 日期：2026-06-29
- 分支：feat/content-assets-tiered-injection（基线 origin/main 84d9578）
- 类型：功能增强 + 过期设计清理（前端 + 后端 + 数据模型）

## 1. 背景与问题

「内容资产」频道（`content_assets` 集合）当前混装两类东西：
- **文件型素材**（image/file/video）：AI 可主动发给客户的二进制物，带完整发送编排（sendable/target_stages/send_trigger_hint/expression_pref/requires_principal_approval）+ 审核闸。这是成熟的销售素材库，**不动**。
- **文本型资产**（kind ∈ text/faq/script/brand_voice/forbidden_expression）：经 `load_context_assets`（decision.rs:1405）注入决策 prompt 的「可引用内容资产」段（decision.rs:866），当话术/口吻/禁语参考。

### 已核实的三个真实问题（全部读代码确认，非猜测）

**问题 A — 文本资产绑死 Full 档，是缺陷不是设计。**
`load_context_assets` 的调用受 `include_business`（decision.rs:332）门控，而 `include_business = matches!(tier, Full)`（decision.rs:316）。即文本资产**只在 Full 档注入**。但 gateway 的渐进式三档（`PROGRESSIVE_TIER_ENABLED` 默认 true）**第一程从 Lean 起步**（gateway.rs:1017-1018），绝大多数轻量对话轮（寒暄、关系经营、简单答复）停在 Lean/Relational，**根本到不了 Full**。结果：一条核心禁语或品牌口吻，在最日常的对话里完全不生效——降档即失效。对比 `doNotDo`/`commitments` 安全约束用的是「任何档恒注入」机制（decision.rs:1268，靠 Lean 档补安全子片），证明跨档注入在架构上完全可行且有现成样板。

**问题 B — 知识库够不到轻量轮。**
知识库（operation_knowledge_chunks）重型、走 progressive-disclosure（catalog→search→open_slice）、**只在 Full 档检索**。它无法覆盖 Lean/Relational 轮的轻量话术/口吻需求。文本资产本应是这块的轻量补充，却同样绑死 Full，等于补位失败。

**问题 C — 「新增资产」表单有过期录入项。**
表单（content-assets/index.tsx:199-264）让运营手填「素材 URL」和「MCP Media ID」两个输入框。`media_id` 本是**系统发送时自动管理的缓存**（见 §2 安全边界），运营不可能手工知道这个值——这是纯粹的过期错误设计。`url` 是文本资产时代的外链字段，文件型素材库用不上。另有 `moment_media`（朋友圈素材）kind 选项，但朋友圈运营域尚未开始做。

## 2. 关键安全边界（实现期红线，已逐点核实）

**`media_id` 字段是文件发送链的命脉，数据模型字段绝不可删**：
- `ensure_media_uploaded`（media_send.rs:91-93）：上传 MCP 前查 `asset.media_id` 缓存，命中且未过 TTL 则复用，避免重复上传
- 发送后**回写** media_id 到 content_assets（media_send.rs:179/235）
- **崩溃防重发不变式**（media_send.rs:236/256）：`asset.media_id == None ⇒ 从未发出 ⇒ 可放行重发`
- 换文件 `clear_media_id`（media_assets.rs:346）强制清缓存防发旧文件

因此本设计对 media_id 的处理是**只删前端录入入口，保留数据模型字段 + 发送链全部逻辑**。create 时 media_id 本就该是 None（系统后填），删录入入口不影响发送链。url 同理（只删录入，保留字段）。

## 3. 设计

### 3.1 数据模型（models.rs `ContentAsset`）

新增字段：
```rust
/// 文本型资产的最低注入档：控制本条资产从哪个档位起注入决策 prompt。
/// lean=任何档恒注入（最常生效）/ relational=关系档起 / full=仅完整档。
/// None（缺失）默认按 full 处理——与改造前「只 Full 注入」逐字等价。
/// 仅对文本型 kind 有意义；文件型素材（走 sendable 发送链）不读此字段。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub min_inject_tier: Option<String>,
```
- 闭集 `{lean, relational, full}`，校验用纯函数白名单（非 DB 约束），非法值落 full（保守）
- 线上**无老数据**（已确认），无迁移负担；默认 full 保证即使有遗漏也等价现状

### 3.2 后端注入逻辑（decision.rs）

新增纯函数（可单测）：
```rust
/// 档位序：Lean(0) < Relational(1) < Full(2)。当前轮档位 >= 资产最低档时注入。
pub(crate) fn asset_visible_at_tier(min_tier: Option<&str>, current: PromptTier) -> bool
```
语义：`current_tier_rank >= min_tier_rank` 才注入。min_tier=None → 视为 full（仅 Full 可见，等价现状）。

改造 `load_context_assets` + 其调用点：
- 调用点（decision.rs:332）**去掉 `if include_business` 门**，改为任何档都调用 `load_context_assets`
- `load_context_assets` 的 Mongo 查询**下推档位条件**：`min_inject_tier` 字段值在「当前档及更低档」集合内（如 current=Lean 只捞 min=lean 的；current=Relational 捞 min∈{lean,relational}；current=Full 捞全部）。用 `$in` 表达当前档可见的 min_tier 取值集合。**关键**：老数据 min_inject_tier 缺失字段，等价 full，仅 Full 档可见——查询需用 `$in` + 对 Full 档额外含「字段不存在」分支（`$or: [min_inject_tier ∈ 可见集, 字段不存在]` 仅 Full 档加后者），保证缺失=full 语义且不漏不多
- prompt 段（decision.rs:866「可引用内容资产」）在任何档都可能有内容，参照 `render_safety_donts_commitments` 跨档样板

**性能**：Lean/Relational 轮新增一次 content_assets 查询。查询走 workspace+account+kind 索引（db/indexes.rs 已有），档位下推后只捞当前档可见的少量行，开销可控。best-effort：DB 故障 → 空串（不阻塞决策，同现有 reaction_hint 路径）。

### 3.3 前端（content-assets/index.tsx + contentStore.ts + types）

**文本录入表单（「新增资产」，保留并重构）**：
- **删除**「素材 URL」输入框（index.tsx:236-243）
- **删除**「MCP Media ID」输入框（index.tsx:244-251）
- **删除** KIND_OPTIONS 里的 `moment_media`（朋友圈素材）选项（index.tsx:17）
- **新增**「最低注入档」下拉：精简档(lean) / 关系档(relational) / 完整档(full)，默认 full。配文案说明：「档位越低，越早注入、越常生效。核心禁语/口吻选精简档（时刻生效）；重型话术/长 FAQ 选完整档（仅深入业务时）。」
- 列表区每条文本资产展示其注入档标签

**contentStore.ts**：
- `assetDraft` 去掉 url/mediaId 键，加 minInjectTier 键（默认 "full"）
- `createAsset` 的 POST body 去掉 url/mediaId，加 minInjectTier
- 其他 action（upload/review/edit/toggle/delete）不碰，零影响

**types/index.ts**：ContentAsset 类型加 `minInjectTier?: string`（url/mediaId 类型字段保留，list 仍返回展示用）

### 3.4 后端 create 端点（assets.rs `create_content_asset`）

- `ContentAssetRequest` 入参**去掉 url、media_id**（不再接受前端传入），**新增 min_inject_tier**（Option，校验闭集，默认 full）
- 构造 ContentAsset 时 url=None、media_id=None（系统语义：文本资产不该带这俩；文件走 upload 端点）、min_inject_tier=payload 值
- list 端点（assets.rs:list_content_assets）输出**保留** url/mediaId 字段（不破坏展示），新增 minInjectTier

## 4. 不做（YAGNI 边界）

- **不删** models.rs 的 url / media_id 字段（media_id 是发送链命脉 §2；url 保留 Option 不写入不破坏结构）
- **不动**文件型素材发送链（send_outbound_media / ensure_media_uploaded / outbox / 防重发）
- **不砍**任何文本 kind（brand_voice/forbidden_expression 保留作轻量补充，正是本设计价值；text/faq/script 同留）
- **不做** script/faq 迁知识库（独立专题，本设计让文本资产成为知识库的正交补充而非替代）
- **不做** moment_media 的后端清理（仅删前端选项；后端无专门逻辑，无需动）

## 5. 测试与闸门

**纯函数单测（锁不变量）**：
- `asset_visible_at_tier`：lean 资产在 Lean/Relational/Full 三档都可见；relational 资产在 Relational+Full 可见、Lean 不可见；full 资产仅 Full 可见
- 默认值：min_tier=None → 仅 Full 可见（与改造前字节等价）
- 档位可见集合纯函数（供查询下推用）：current=Lean→{lean}；Relational→{lean,relational}；Full→全部+字段缺失

**回归闸门**：
- 后端 `cargo test --lib` ≥ 350/0；新增 content_assets 注入相关测试
- 前端全套只增不减；`tsc --noEmit` 0 error；`npm run build` 成功 + CSS module 存活
- no-human-takeover lint：新增行（含前端文案）0 命中禁词
- 命名红线：前端文案用 AI 中性词

## 6. 完成标准

- 文本资产可按条配置最低注入档（lean/relational/full），在对应档位起注入决策 prompt，修复「绑死 Full、降档失效」缺陷，成为知识库（重/Full/按需）的轻量正交补充（轻/任意档/可恒在）
- 「新增资产」表单去掉 URL / MCP Media ID 录入框 + moment_media 选项，加最低注入档下拉
- create 端点不再接受 url/media_id 入参；models 的 url/media_id 字段及文件发送链完整保留
- 全套测试绿 + 基线不退 + 双 lint 0 命中
