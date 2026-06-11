# 通用化知识库：LLM 对话驱动的行业/产品自适应 — 设计文档

**日期**：2026-06-11
**状态**：设计待评审（DRAFT，零代码改动）
**作者**：运营 agent 侧
**目标读者**：产品负责人 + 知识库后端维护者

---

## 0. 一句话目标

系统出厂时对行业**零假设**。任意行业、任意产品的运营，通过**与 LLM 对话 + 上传行业文档**，由 AI 生成本行业的「画像维度 + 知识字段表 + prompt 片段」候选，经人工审核后落成**稳定、可版本化、可回滚**的配置；运行时消费这份配置，而非消费写死的销售域字段。

非目标（明确排除）：
- ❌ 让运行时决策 agent 每轮自由发明维度（会摧毁画像稳定性，见 §2.1）。
- ❌ 预设若干行业模板（医疗/教培/SaaS）。我们要的是「对话适配任意行业」，不是「内置 N 个行业」。
- ❌ 召回算法升级（向量/BM25/语义检索）——是另一条正交的天花板，本设计不含（见 §7 风险）。

---

## 1. 现状：意图 vs 现实（证据级）

经三个 Opus 子代理 + 主线亲自核实，知识库子系统按「通用运营知识库」设计，**知识容器层已通用，画像决策层仍锁在销售域**。

### 1.1 已经通用的部分 ✅

| 层 | 现状 | 证据 |
| --- | --- | --- |
| chunk 主表 | 30+ 跨行业稳定字段 + 单一 `domain_attributes` 可变子文档 | `models.rs:854-944` |
| 9 类 wiki_type | 认知类型（非销售类型），跨行业举例 | `knowledge-wiki.md §3` |
| 去销售域 CI 闸 | `check-no-sales-domain.sh` 主动禁止销售域命名爬回 src/ | 脚本存在 |
| 写入三层保护 | 锁定字段/数组union/70%阈值，PBT 覆盖 | `chunk_revisions.rs` + `page_merge.rs` |
| AI 永不自动 verify | 6 重冗余兜死，无漏网 | `chunk_revisions.rs:207-210` 等 |
| grounding 主硬闸 | verified-ID 精确交集，行业无关、不可被幻觉绕过 | `review/gates.rs:563-606` |
| 召回排序 | 字符 n-gram 覆盖率，语言无关 | `knowledge_agent.rs:1670-1681` |
| taxonomy check_value | 按 `kind` 字符串泛化 + scope 分层(account/global) | `taxonomy.rs:196-239` |
| 多租户 workspace_id | 贯穿所有集合 + ACL 白名单防 IDOR | `auth/middleware.rs`、`routes/auth.rs:144-151` |
| 配置版本灰度 | system_taxonomies 等三表 publish/rollout/rollback | `agent-policy.md` E5-T1 |

### 1.2 仍锁在销售域的部分 ❌（本设计的攻坚对象）

| # | 硬编码点 | 位置 | 影响 |
| --- | --- | --- | --- |
| H1 | `AgentDecision.customer_stage / intent_level` 是 typed 字段 | `agent/types.rs:99-100` | 换行业维度必须改 Rust 源码 |
| H2 | `TAGGED_FIELDS` const 表硬绑 getter/setter | `agent/decision_taxonomy.rs:38-49` | 维度集合写死，不可配 |
| H3 | prompt 点名销售维度语义 | `prompts.rs:749,771,886-887,936,1595` | "客户阶段=陌生/关注/评估/决策/成交" 写死 |
| H4 | grounding 兜底探针词表中文销售词 | `guards.rs:331-345` | 换行业绝对化承诺静默漏判 |
| H5 | completeness 审计五维 coverage | `catalog.rs:553-605` | capability/pricing/effectClaims 强绑 B2B 销售 |

### 1.3 顺带发现的非通用化缺陷（独立问题，本设计标注但不混入）

- **D1**：`DomainSchema` 运行时零消费——CRUD/版本/校验都建了，但没有任何运行时代码读 active schema。文档（`domain_schemas.rs:5-7`、`knowledge-wiki.md §9`）宣称"写入侧按 active schema 校验"是**未落地**。
- **D2**：verify/reject/auto_verify/PUT/chat 五类写入**绕过** `apply_chunk_revision`，verify 这个关键状态转移**不写 chunk_revisions 历史、不更新 provenance**——审计链在"升级为 verified"处断裂。
- **D3**：关系图谱 BFS 遍历忽略 `relation_kind`，contradicts 与 references 无差别扩散；superseded_by 不做版本 redirect。

这三个是已有功能的实现缺陷，**与通用化正交**，列入「后续清理」不进本设计主线（除非 §6 分期里顺手）。

---

## 2. 核心设计决策

### 2.1 关键分野：两种「LLM 协作」

你提出"引入 LLM 协作适配行业"，有两种做法，**必须选正解、排除陷阱**：

| | 陷阱：运行时自由发挥 | 正解：对话生成稳定配置 |
| --- | --- | --- |
| 做法 | 决策 agent 每轮自由理解行业、自由决定追踪哪些维度 | 运营对话 + 文档 → AI 生成候选配置 → 人审 → 落稳定配置 → 运行时消费 |
| 稳定性 | ❌ 维度每轮漂移，planner 停滞计时器/状态机/数据分析全废 | ✅ 配置稳定可版本化，运行时仍走现有 guard |
| 与现有红线 | ❌ 违反"画像更新须保守""禁止过拟合" | ✅ 兼容，candidate→审核流原样保留 |
| 可回滚 | ❌ 无 | ✅ 复用 publish/rollout/rollback |

**本设计采用正解。** LLM 的角色是「配置的生成者/建议者」（离线、一次性、人审把关），不是「运行时维度的发明者」。

### 2.2 三层架构

```
┌─ ① 引导层（新建·你的核心想法）────────────────────────────┐
│  运营对话："我做口腔种植牙私域获客" + 上传行业文档/产品手册   │
│  → AI 阅读 + 多轮澄清对话，生成候选：                         │
│      • 画像维度（就诊阶段/治疗意向/顾虑类型 + 取值 + 中文别名）│
│      • chunk 自定义字段表（适应症/禁忌/价格区间…）            │
│      • 该行业的 prompt 片段（如何理解这些维度）               │
│      • grounding 承诺词表（本行业的绝对化承诺词：根治/保过…）  │
│      • completeness coverage 维度（本行业"答全了吗"的标准）    │
│  → 全部走【已有的】candidate → admin 审核 → publish 流程       │
└───────────────────────────────────────────────────────────┘
                          ↓ 人工审核确认
┌─ ② 配置层（大半已存在，少量扩展）──────────────────────────┐
│  system_taxonomies   ← 画像维度字典（kind 已泛化 + scope 分层）│
│  DomainSchema        ← chunk 自定义字段表（CRUD/校验已存在）   │
│  prompt_templates    ← 行业 prompt 片段（locale 机制已存在）   │
│  domain_profile(新)  ← 声明"本行业有哪些画像维度参与决策"      │
│  版本灰度：publish / rollout / rollback（已存在，全部复用）    │
└───────────────────────────────────────────────────────────┘
                          ↓ 运行时按 active 配置加载
┌─ ③ 运行时消费层（解耦写死维度·Phase E 推迟的部分）─────────┐
│  AgentDecision.customer_stage/intent_level（写死）            │
│         → domain_signals: Document（泛化容器，已有先例）      │
│  TAGGED_FIELDS const 表                                       │
│         → 从 active domain_profile 动态加载维度列表           │
│  check_value(kind,...) 原样复用（已泛化）                     │
│  prompt 维度语义段 → 从 active 配置注入（替代 H3 写死文案）    │
│  grounding 词表 / completeness 维度 → 从 active 配置读（H4/H5）│
└───────────────────────────────────────────────────────────┘
```

### 2.3 为什么不合并 DomainSchema 和 system_taxonomies

你最初问过这个。答案：**不合并，让引导层同时驱动它俩**。两者职责不同：

- `system_taxonomies` 治理「**agent 给客户打什么画像标签**」（需 alias 归一/候选发现/canonical id/进程缓存热路径）。
- `DomainSchema` 治理「**知识 chunk 有哪些内容字段**」（静态字段校验，面向写入）。

合并会把两套不同生命周期的东西揉在一起。正确做法是新增一个轻量的 `domain_profile`（§3.1）作为「行业总装配单」，引用这两者 + prompt 片段，形成单一 active 入口。

---

## 3. 数据模型变更

### 3.1 新增 `domain_profiles` 集合（行业总装配单）

```rust
/// 一个行业/产品的完整画像配置装配单。每 workspace 同时 1 条 is_active=true。
/// 由引导层 AI 生成候选 → admin 审核 → publish。运行时按 active 加载。
pub struct DomainProfile {
    pub id: Option<ObjectId>,
    pub profile_id: String,          // slug，如 "dental-implant-private"
    pub workspace_id: String,
    pub display_name: String,        // "口腔种植牙私域获客"
    pub description: String,         // AI 生成的行业画像说明（人可读）

    /// 参与决策的画像维度声明（替代 H2 的 TAGGED_FIELDS const 表）。
    /// 每个维度的取值字典仍存 system_taxonomies（按 kind 关联）。
    pub profile_dimensions: Vec<ProfileDimension>,

    /// 关联的 chunk 字段表（引用 DomainSchema.schema_id）。
    pub domain_schema_id: Option<String>,

    /// 行业 prompt 片段（替代 H3 写死文案）。
    pub prompt_fragment: Option<String>,

    /// 本行业绝对化承诺词表（替代 H4 写死中文销售词）。
    pub commitment_markers: CommitmentMarkers,

    /// completeness 审计维度（替代 H5 写死五维）。
    pub coverage_dimensions: Vec<CoverageDimension>,

    // 版本灰度四字段（与 system_taxonomies 等三表对齐，复用 E5-T1 机制）
    pub version: i32,
    pub current_version: bool,
    pub previous_version: Option<i32>,
    pub seeded_by: Option<String>,   // generated_by_ai / manual / default

    pub is_active: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

pub struct ProfileDimension {
    pub kind: String,            // snake_case，对应 system_taxonomies.kind，如 "visit_stage"
    pub display_name: String,    // "就诊阶段"
    pub participates_in_decision: bool,  // 是否进 TAGGED_FIELDS 校验
    pub description: String,     // 注入 prompt 的语义说明
}

pub struct CommitmentMarkers {
    pub product_effect: Vec<String>,  // 本行业 ProductEffect 词（"根治率"/"保过"）
    pub tone_only: Vec<String>,       // 本行业 ToneOnly 词
}

pub struct CoverageDimension {
    pub key: String,             // "indication" / "contraindication"
    pub display_name: String,    // "适应症" / "禁忌人群"
    pub required: bool,
}
```

**R11 向后兼容**：旧库无 `domain_profiles` 时，运行时 fallback 到一个内置的 `DEFAULT_PROFILE`（等价于当前 customer_stage/intent_level + 现有词表/五维），保证零配置启动行为与今天**完全一致**。这是关键安全网——不配置照样跑，配置了才换行业。

### 3.2 `AgentDecision` 解耦（H1/H2）

```rust
// 现状（写死）：
pub customer_stage: Option<String>,
pub intent_level: Option<String>,

// 改为（泛化，已有 profile_attributes: Document 先例）：
pub domain_signals: Document,   // { "visit_stage": "consult", "treatment_intent": "high" }
```

**兼容策略**：保留 `customer_stage`/`intent_level` 作为 `#[serde(default)]` 可选字段，反序列化时若 LLM 仍输出它们，由一个 normalize 步骤迁移进 `domain_signals`（DEFAULT_PROFILE 下两者等价）。gateway 写 contact.domain_attributes 时遍历 `domain_signals` 而非读固定字段。

### 3.3 `decision_taxonomy::TAGGED_FIELDS` 动态化（H2）

```rust
// 现状：const 表 + 函数指针 getter/setter
// 改为：从 active DomainProfile.profile_dimensions 过滤 participates_in_decision=true，
//       对 decision.domain_signals 的对应 key 做 check_value（逻辑本身已泛化）。
```

`check_value(kind, raw, scope, cache)` **完全不动**——它早已按 kind 泛化（`taxonomy.rs:196`）。只把「遍历哪些 kind」从 const 表换成「读 active profile」。

---

## 4. 引导层（①）详细设计

### 4.1 交互流程

```
运营进入「行业配置向导」（控制台新增入口）
  1. 对话开场："你的业务是什么行业、卖什么产品/服务、客户是谁？"
  2. 运营答 + 上传行业文档（产品手册/SOP/话术集，走已有 import 管道）
  3. AI 阅读文档 + 多轮澄清（"你的客户从接触到成交一般经历哪几个阶段？"）
  4. AI 产出候选配置草案（结构化 JSON，对应 DomainProfile 各字段）
  5. 运营在 UI 逐项审核/编辑（维度名、取值、别名、词表、coverage）
  6. 确认 → 走已有 candidate→publish 流，落 domain_profiles + system_taxonomies + DomainSchema
  7. activate → 运行时下一轮决策即用新 profile
```

### 4.2 LLM 产物形态：patch-only / 结构化输出

复用现有 `generate_agent_json`（`agent/mod.rs` 唯一 LLM JSON 入口）。AI 只返结构化候选 JSON，**不直接写库**——和 chunk 的 patch-only 协议同理，人审后由后端落库。

### 4.3 红线继承

- AI 生成的所有配置 = **候选**，必须人工审核才生效（继承"AI 永不自动 verify"精神）。
- 文档导入产出的 chunk 仍 `draft + needs_review`（红线不变）。
- 候选不阻塞任何运行时路径（继承 taxonomy candidate "SHALL NOT 阻塞" 约束）。

---

## 5. 分工边界

| 改动 | 归属 | 说明 |
| --- | --- | --- |
| `domain_profiles` 集合 + 模型 + CRUD + 版本灰度 | 知识库后端 | 新集合，沿用 E5-T1 版本机制 |
| `DomainSchema` 运行时接线（修 D1） | 知识库后端 | 让 active schema 真正校验 chunk 写入 |
| 引导层 AI 生成配置（prompt + 结构化输出 + 审核 UI 后端） | 知识库后端 + 运营 agent 协作 | 跨界，需对齐 |
| `AgentDecision` 解耦 domain_signals（H1） | **运营 agent（我）** | 核心路径 |
| `TAGGED_FIELDS` 动态化（H2） | **运营 agent（我）** | 核心路径 |
| prompt 维度语义动态注入（H3） | **运营 agent（我）** | 核心路径 |
| grounding 词表配置化（H4） | **运营 agent（我）** | guards.rs |
| completeness 维度配置化（H5） | 知识库后端 | catalog.rs |
| 引导层前端向导 UI | **前端（我）** | 控制台新增 |

**需你裁决**：这个工程横跨运营 agent + 知识库后端。建议由我统一推、知识库侧改动列清单供他人 review，还是严格按上表分头做。

---

## 6. 分期落地（每期可独立交付、可回滚）

### Phase 0：安全网先行（低风险，纯增量）
- 新增 `domain_profiles` 集合 + `DomainProfile` 模型 + `DEFAULT_PROFILE` 内置常量。
- 运行时**仍走写死路径**，只是并行加载 active profile（无则 DEFAULT）。
- 验证：零行为变化，基线测试全绿。

### Phase 1：运行时消费层解耦（中风险，核心）
- `AgentDecision` 加 `domain_signals: Document`，normalize 兼容旧字段。
- `TAGGED_FIELDS` 改读 active profile.profile_dimensions。
- prompt 维度语义段从 profile 注入（DEFAULT_PROFILE 文案 = 当前文案，逐字对齐防过拟合）。
- 验证：DEFAULT_PROFILE 下所有现有 PBT/real-LLM 套件**逐条等价**，这是反过拟合的硬护栏。

### Phase 2：grounding / completeness 配置化（中风险）
- H4 词表、H5 五维从 profile 读，DEFAULT_PROFILE 值 = 当前硬编码值。
- 修 D1：DomainSchema 运行时接线（顺带，因引导层要它生效）。

### Phase 3：引导层（你的核心想法，价值兑现）
- AI 对话 + 文档 → 生成候选配置 → 审核 UI → publish。
- 前端向导。
- 端到端验证：用一个**非销售**行业（如选定的目标行业）跑通"对话→生成→审核→激活→AI 按新维度决策"。

### Phase 4（可选）：清理 D2/D3 审计与图谱缺陷。

---

## 7. 风险与护栏

| 风险 | 等级 | 护栏 |
| --- | --- | --- |
| 改 decision/gateway/prompt 核心路径破坏现有行为 | 高 | Phase 0 安全网 + DEFAULT_PROFILE 逐字等价 + 现有基线全绿才进下一期 |
| 对单一目标行业过拟合（违反反过拟合红线） | 高 | DEFAULT_PROFILE = 当前销售域配置，新行业只是「另一份 profile」，不改通用逻辑；引导层 prompt 不得写死任何具体行业词 |
| 召回算法天花板（字符匹配无语义） | 中 | **本设计不解决**，独立列出。术语专业化行业召回质量受限是已知上限 |
| 多租户隔离靠手写过滤、无框架强制 | 中 | 本设计不引入新跨租户查询；新集合沿用 workspace_id 模式 + 复用 ACL |
| LLM 生成配置质量参差 | 中 | 全部候选人审；审核 UI 逐项可改；版本可回滚 |
| 配置膨胀/维度爆炸 | 低 | 沿用 DomainSchema 的 fields≤64 类约束思路，profile_dimensions 设上限 |

---

## 8. 待你拍板的开放问题

1. **推进方式**：已定 = 先出本设计文档对齐 ✅。审完是否进 Phase 0？
2. **分工**：我统一推（知识库侧列清单 review）vs 严格分头？（§5）
3. **目标行业**：已定 = 纯通用、不绑定具体行业 ✅。但 Phase 3 端到端验证需要**一个真实行业样例**跑通，你指定一个？（仅用于验证，不写进代码）
4. **DEFAULT_PROFILE 去留**：长期是否保留销售域 DEFAULT_PROFILE 作为兜底，还是出厂即"空 profile + 强制引导"？（影响新部署首次体验）

---

## 9. 附：本设计不碰的红线（继承）

- AI 永不自动 verify（新知识/配置候选都需人审）。
- 画像更新须保守、禁止过拟合、系统性思维找最优杠杆。
- 新增测试只增量叠加，不删改旧维度/旧金标。
- check-no-human-takeover / check-no-model-hint / check-no-sales-domain 三闸。
- D2 锚定 quote→anchor verify gate 不削弱。
