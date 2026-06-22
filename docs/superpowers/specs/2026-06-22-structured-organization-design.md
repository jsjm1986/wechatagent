# 结构化组织（structured organization）设计

> 簇 D / 8 缺口补全的第 4 个（最后一个）子项目。为素材库 + 专属顾问名片补两个"结构化组织"缺口：知识侧业务主题注入（缺口7）+ 标签激活与注入（缺口8）。簇 A（发送台账）、簇 B（标注质量门）、簇 C（素材 CRUD）已完成。

**Date:** 2026-06-22
**Status:** 设计已获批，待落实现计划（writing-plans）
**Scope:** 仅簇 D（结构化组织，缺口 7 + 8）。8 缺口至此全部覆盖。

## 1. 背景与动机

素材库（content-assets）与专属顾问名片（referral-cards）通过"人类标注（target_stages + send_trigger_hint）→ 过滤候选 → 注入 prompt 候选清单 → AI 在主决策里选"的**提示词注入**机制工作。知识库（话术轨）与素材库（交付轨）是两条独立注入线，AI 每轮并行评估，由 `expression_pref` 协调文字详略（详见 `2026-06-21-sales-media-asset-send-design.md` §3.2 双轨并行）。

缺口 7、8 都是对这条注入机制的**软增强**——让 AI 在 prompt 里看到更多人类标注的结构化维度，从而更好地自主判断该配什么文档、引荐谁。

### 缺口 7：知识侧业务主题未注入

用户原始诉求："知识库只能文字答问，但报价表/案例图/公司 PDF 需以文档形式发给客户，两者**互补**。" 当前双轨并行 + trigger_hint 已能覆盖"文字答 + 文档配套"，但有一个注入缺口：知识 chunk 注入 prompt 时（`render_chunk`，`knowledge_router.rs:236`）**只渲染 title/summary/body/sourceQuote 等，不渲染 `product_tags`/`business_topics`**。AI 看不到"已打开的知识切片属于哪些产品/业务主题"，难以把知识点与素材在语义上对照配套。

### 缺口 8：标签是半死字段

- **素材 tags**（`ContentAsset.tags: Vec<String>`，`models.rs:687`）：create/edit 能写、list 能读，但 **upload 硬编码 `tags: vec![]`**（`media_assets.rs:152`，文件素材永远空标签）、**`render_candidate_lines` 不渲染 tags**（`media_send.rs:52`，AI 选材看不到）、**list 不能按 tag 过滤**、**前端零 tags UI**。
- **名片 tags**：ReferralCard 完全无此字段（`models.rs:851-871`）。

用户对标签的定位："管理员素材库导入时录入/标注的信息，需注入提示词方便 agent 自己判断"——即 tags 是 trigger_hint（自然语言）之外的**结构化补充维度**，既进 prompt 帮 AI 选材，又供运营后台按标签检索。

## 2. 已锁定的关键决策（brainstorming 产出）

| # | 决策点 | 选择 | 理由 |
|---|---|---|---|
| D1 | 缺口7 知识关联方式 | **软增强注入（保 agent-first）**：注入知识 chunk 的 product_tags/business_topics，让 AI 看到知识点与素材的语义关系自主配套，**不建硬关联** | 双轨并行 + trigger_hint 已覆盖"文字答+文档配套"；显式硬关联与项目 agent-first 立场（LLM 语义判断、不硬匹配）有张力，且引入 drift 维护成本 |
| D2 | 缺口8 标签定位 | **结构化标签 + 注入 + 检索**：tags 作 trigger_hint 之外的结构化补充维度，既进 prompt 候选清单帮 AI 选材，又供运营后台按 tag 检索 | 用户明确"管理员标注注入帮 agent 判断" + 运营组织诉求 |
| D3 | 实现组织 | **缺口7+8 合一个"软增强注入"主题**：知识侧注入加 product_tags/business_topics、素材侧激活并注入 tags、名片新增 tags | 两缺口本质同源——往 prompt 注入更多人类标注的结构化维度帮 AI 判断；各处改动小、无新集合、无关联表、无 drift |
| D4 | tags 是否硬门 | **不作发送硬门**：tags 是增强维度，AI 综合 trigger_hint/stage/tags 自主判断 | agent-first：机器只给客观度量，语义判断交 LLM |
| D5 | 名片是否加 edit | **不加**：tags 在 create 录入，名片无 edit 端点（与名片现有设计一致，改靠删重建） | 避免范围蔓延；名片字段少删重建成本低 |

## 3. 架构

簇 D 是纯"提示词注入增强 + 标签字段激活"，**不新建集合、不建关联表、不改决策/路由/grounding 逻辑**。涉及文件：

- **缺口7**：`src/agent/knowledge_router.rs` 的 `render_chunk`（:236）注入加 `productTags`/`businessTopics`。
- **缺口8 素材侧**：`src/routes/media_assets.rs`（upload 写 tags）、`src/agent/media_send.rs` 的 `render_candidate_lines`（:52，注入加 tags）、`src/routes/assets.rs`（list 加 `?tag=` 过滤）。
- **缺口8 名片侧**：`src/models.rs`（ReferralCard 加 tags 字段）、`src/routes/referral_cards.rs`（create 写 tags + list 输出）、`src/agent/referral.rs` 的 `render_referral_lines`（:45，注入加 tags）。
- **前端**：`frontend/src/features/content-assets/index.tsx`（tags 编辑/筛选）、`frontend/src/features/referral-cards/index.tsx`（tags 编辑/展示）、`frontend/src/types/index.ts`（ReferralCard 类型加 tags）。

## 4. 缺口 7：知识侧业务主题注入

### 4.1 数据流

```
AI 知识路由选中 chunks（现有逻辑，不动）
  → format_operation_knowledge_for_prompt_with_roles → render_chunk 渲染每个 chunk
  → 【新增】render_chunk 输出加：productTags=[...] businessTopics=[...]（非空才渲染）
  → 注入 decision prompt
  → AI 看到"已打开知识切片属于哪些产品/业务主题"，结合素材候选 tags 自主语义配套
```

### 4.2 改动点

`render_chunk`（`knowledge_router.rs:236-251`）当前 format 字符串渲染 chunkId/type/chunkType/context/title/integrityStatus/confidence/summary/body/sourceAnchors/sourceQuote。**加 productTags + businessTopics**：

- `OperationKnowledgeChunk.product_tags: Vec<String>`（`models.rs:1171`，≤5，产品/品牌/解决方案名，LLM import 抽取 + 后台可编辑）。
- `OperationKnowledgeChunk.business_topics: Vec<String>`（`models.rs:1174`，≤3，业务议题如产品定位/竞品对比/部署方式）。
- 非空才渲染该行（空 Vec 跳过，避免空 `productTags=[]` 噪声）。

### 4.3 要点

- **纯注入增强**：不改知识路由选 chunk 的逻辑、不改分桶（`format_..._with_roles` 的 role 分桶不动）。只让已选中的 chunk 在 prompt 里多展示两个维度。
- product_tags/business_topics 是现成字段——**零新增字段、零关联**。
- AI 看到知识切片 business_topics + 素材候选 tags（缺口8），自主判断语义契合而配套发——软增强、无硬关联、无 drift。

## 5. 缺口 8 素材侧：tags 激活 + 注入

### 5.1 改动点

1. **upload 写 tags**：`media_assets.rs` upload 接收 multipart 的 `tags` 字段（逗号分隔，复用 target_stages 的 split/trim/filter 解析），替换硬编码 `tags: vec![]`（:152）。
2. **render_candidate_lines 注入 tags**（`media_send.rs:52`）：候选行加标签维度：
   ```
   - [id:a1] 产品报价单.xlsx | 阶段:意向 | 表达:文件为主 | 标签:报价类,价格
     触发提示:客户问价格时发
   ```
   tags 空时不渲染"| 标签:..."段。
3. **list 加 `?tag=` 过滤**：`list_content_assets`（`assets.rs:49`）加可选 `tag` query 参数，filter 加 `{ tags: tag }`（MongoDB 数组字段等值匹配，命中 tags 含该元素的文档）。
4. **前端**：MediaAssetRow 编辑表单加 tags 输入（逗号分隔，同 targetStages）；upload 表单加 tags 输入；list 顶部可选 tag 筛选。

### 5.2 要点

- tags 进候选清单 = trigger_hint（自然语言）之外的结构化补充维度，与缺口7 的知识 business_topics 在同一 prompt 里语义对照。
- list 按 tag 过滤用 MongoDB 数组等值匹配，无需新索引（tag 筛选量小，200 limit 内）。
- upload/前端 tags 解析复用簇 C target_stages 逗号分隔逻辑，不新造模式。
- **不作发送硬门**：AI 综合 trigger_hint/stage/tags 自主判断（D4）。
- 向后兼容：旧素材 tags 空，渲染时跳过标签行。

## 6. 缺口 8 名片侧：tags 新增 + 注入

### 6.1 改动点

1. **ReferralCard 加字段**：`tags: Vec<String>`（`#[serde(default)]`，向后兼容——旧名片反序列化为空 Vec）。
2. **create_referral_card** 请求体加 tags，写入。
3. **render_referral_lines 注入 tags**（`referral.rs:45`）：
   ```
   - [card:c1] 销售总监-老王 | 阶段:意向 | 标签:高客单,签约 | 触发提示:客户要签约时引荐
   ```
   tags 空时不渲染标签段。
4. **list_referral_cards**（`referral_cards.rs:81`）输出加 tags。
5. **前端**：名片库 create 表单加 tags 输入 + 列表展示 tags。

### 6.2 要点

- `tags: Vec<String>` + `#[serde(default)]` 是向后兼容硬要求（旧名片无此字段回落空 Vec）。
- 名片 tags 同样进候选清单注入，与素材对称。
- **名片不加 edit 端点**（D5）：tags 在 create 录入；改 tags 靠删重建（名片现有设计无 edit，本簇保持一致）。前端 create 表单加 tags 输入即可。

## 7. 前端

遵循 `docs/frontend-design-system.md` 企业白色基调，复用簇 C 的编辑表单模式 + 既有类名，不新造样式：

- **素材页**（content-assets）：MediaAssetRow 编辑表单加 tags 输入（逗号分隔）+ 展示；upload 表单加 tags 输入；列表顶部可选 tag 筛选下拉/输入。
- **名片页**（referral-cards）：create 表单加 tags 输入 + 列表展示 tags chips。
- 文案守 no-human-takeover 禁词（用"标签"中性词）。

## 8. 测试策略

遵项目铁律（纯函数确定性为主、不接受 skip 假绿、新增只 append、不过拟合）：

| 层 | 测什么 | 方式 |
|---|---|---|
| 纯函数 | `render_candidate_lines` 含 tags 行（tags 非空）/ tags 空时不渲染标签段 | lib 单测 |
| 纯函数 | `render_referral_lines` 含 tags 行 / 空时跳过 | lib 单测 |
| 纯函数 | `render_chunk` 含 productTags/businessTopics（非空）/ 空时跳过 | lib 单测 |
| 向后兼容 | ReferralCard 加 tags 后旧名片（无 tags）反序列化为空 Vec | lib 单测 |
| 集成（CI/`#[ignore]`） | upload 写 tags 落库；list `?tag=` 过滤命中含该 tag 的素材 | testcontainers |
| 前端 | `npm run build` 通过、无 TS 错误；tags 编辑/筛选用现有设计系统 | 构建 + 人工对照 |

baseline 不回归（`cargo test --lib` ≥350/0；4 PBT 累计 ≥33/0）；新增测试只 append。

## 9. 边界 / 不做（YAGNI）

- **不建**素材↔知识关联表（软增强注入、无硬关联——D1 用户明确选择）。
- **不改**知识路由 / 选 chunk 逻辑 / grounding（只增注入字段）。
- **不把** tags 作发送硬门（纯增强维度，AI 综合判断——D4）。
- **不给名片加 edit 端点**（tags create 时录入，与名片现有设计一致——D5）。
- **不做**标签云 / 标签管理页 / 标签词表治理（纯自由文本 tags；未来要受控词表再单独做）。
- **不做**按 business_topics 检索知识 → 反查素材（那需要关联；本簇是软注入）。
- **不动**文本类素材 create/list 既有行为（只补 upload 写 tags + list 加 tag 过滤）。

## 10. 红线守卫

- 全部纯注入增强 + 字段激活，**不改决策 / 路由 / grounding**。
- tags 不作硬门（agent-first，AI 综合判断）。
- ReferralCard 加字段 `#[serde(default)]` 向后兼容；旧数据零迁移。
- 知识侧 product_tags/business_topics 是现成字段，零新增、零关联。
- no-human-takeover 禁词（用"标签 / 业务主题"等中性词）。
- 渲染空 tags/空主题时跳过，不产生 prompt 噪声。

## 11. 与 8 缺口全局的收尾

簇 D 完成后，素材库 + 名片引荐两个对称功能的 8 个缺口全部覆盖：
- 簇 A：主动发送台账（效果追踪 + 防重发历史）。
- 簇 B：标注质量门（override 入口 / 审核审计 / 阶段归一校验）。
- 簇 C：素材 CRUD 补全（edit / delete / disable）。
- 簇 D：结构化组织（知识侧主题注入 + 标签激活与注入）。
