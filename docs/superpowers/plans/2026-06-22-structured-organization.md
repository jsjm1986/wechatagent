# 结构化组织（structured organization）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为素材库 + 名片补两个"软增强注入"缺口：知识侧 render_chunk 注入 product_tags/business_topics（缺口7）；激活素材 tags 半死字段 + 名片新增 tags，注入候选清单 + 支持检索（缺口8）。

**Architecture:** 纯"提示词注入增强 + 标签字段激活"，不新建集合、不建关联表、不改决策/路由/grounding。三处渲染函数各加标签维度（纯函数可测）+ upload 写 tags + list 加 tag 过滤 + 名片加 tags 字段 + 前端 UI。

**Tech Stack:** Rust (Axum) + MongoDB + React 19 + TypeScript + Vite。设计文档：`docs/superpowers/specs/2026-06-22-structured-organization-design.md`。

## Global Constraints

- **软增强、不建硬关联**：缺口7 只往 render_chunk 注入现成的 product_tags/business_topics，不建素材↔知识关联表、不改知识路由选 chunk 逻辑。
- **tags 不作发送硬门**：tags 是 trigger_hint 之外的结构化补充维度，AI 综合 trigger_hint/stage/tags 自主判断（agent-first）。不在 filter_sendable_candidates / filter_referral_candidates 里用 tags 做硬过滤。
- **向后兼容**：ReferralCard 加 `tags: Vec<String>` 必带 `#[serde(default)]`，旧名片反序列化为空 Vec；旧素材/旧名片 tags 空时渲染跳过标签段，零迁移。
- **空标签不渲染**：tags/product_tags/business_topics 为空时不输出"| 标签:..." / "productTags=..." 段，避免 prompt 噪声。
- **workspace_id scope**：list 的 tag 过滤、所有查询 filter 保留 workspace_id。
- **no-human-takeover lint**：`src/agent/`、`src/routes/`、`frontend/src/` 新增行禁止 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。用"标签 / 业务主题"中性词。
- **测试基线**：`cargo test --lib` ≥ 350 passed / 0 failed。本地只跑 `cargo test --lib` + 单 PBT。新增测试只 append。
- **回复语言**：与用户对话用中文；代码/标识符/commit 沿用既有约定。

---

### Task 1: 缺口7 知识侧 render_chunk 注入 product_tags/business_topics

**Files:**
- Modify: `src/agent/knowledge_router.rs:236-251`（`render_chunk` 闭包加两字段；测试加进该文件 `#[cfg(test)] mod tests`，若无则新建）

**Interfaces:**
- Consumes: `OperationKnowledgeChunk.product_tags: Vec<String>`（`models.rs:1171`，≤5 产品/品牌名）、`business_topics: Vec<String>`（`models.rs:1174`，≤3 业务议题）。
- Produces: 无新对外接口（render 输出多两行）。

**背景**：知识 chunk 注入 prompt 时（`render_chunk`）当前不渲染 product_tags/business_topics，AI 看不到知识切片的业务主题，难与素材 tags 语义对照配套。本 task 注入这两个现成字段。纯注入增强，不改知识路由。

- [ ] **Step 1: 写失败测试（纯函数）**

`render_chunk` 是 `format_operation_knowledge_for_prompt_with_roles` 内的闭包，不便单独测。改测公开函数 `format_operation_knowledge_for_prompt`（`knowledge_router.rs:183`）的输出含标签。加到 `knowledge_router.rs` 的 `#[cfg(test)] mod tests`（文件末尾，若无则新建 `#[cfg(test)] mod tests { use super::*; use crate::models::OperationKnowledgeChunk; ... }`）：

```rust
    #[test]
    fn render_chunk_includes_product_tags_and_business_topics() {
        let mut chunk = OperationKnowledgeChunk::default();
        chunk.title = "价格说明".to_string();
        chunk.chunk_type = "product_fact".to_string();
        chunk.product_tags = vec!["套餐A".to_string(), "套餐B".to_string()];
        chunk.business_topics = vec!["价格".to_string()];
        let out = format_operation_knowledge_for_prompt(&[chunk]);
        assert!(out.contains("套餐A"), "应渲染 product_tags");
        assert!(out.contains("价格"), "应渲染 business_topics");
    }

    #[test]
    fn render_chunk_skips_empty_tags() {
        let mut chunk = OperationKnowledgeChunk::default();
        chunk.title = "无标签切片".to_string();
        chunk.chunk_type = "product_fact".to_string();
        // product_tags / business_topics 留空
        let out = format_operation_knowledge_for_prompt(&[chunk]);
        assert!(!out.contains("productTags"), "空 product_tags 不渲染该段");
        assert!(!out.contains("businessTopics"), "空 business_topics 不渲染该段");
    }
```

> 确认 `OperationKnowledgeChunk` 有 `Default`（`models.rs:1264` 附近的构造说明 product_tags/business_topics 有 default）。若没有 `#[derive(Default)]`，测试改用完整构造一个 chunk（参照 `models.rs` 既有 chunk 测试 fixture）。实现者先 grep 确认。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib render_chunk_includes_product_tags render_chunk_skips_empty_tags`
Expected: `render_chunk_includes_product_tags_and_business_topics` FAIL（当前不渲染标签）；`render_chunk_skips_empty_tags` 可能 PASS（当前就不渲染）。

- [ ] **Step 3: 实现注入**

`knowledge_router.rs:236-251` 的 `render_chunk` 闭包，在 format 字符串末尾追加 product_tags/business_topics（非空才加）。改造为先构造可选标签段再拼接：

```rust
    let render_chunk = |item: &OperationKnowledgeChunk| -> String {
        let mut s = format!(
            "- chunkId={} type={} chunkType={} context={} title={}\n  integrityStatus={} confidence={}\n  summary={}\n  body={}\n  sourceAnchors={}\n  sourceQuote={}",
            item.id.map(|id| id.to_hex()).unwrap_or_default(),
            item.knowledge_type.clone().unwrap_or_default(),
            item.chunk_type,
            item.business_context.clone().unwrap_or_default(),
            item.title,
            item.integrity_status.clone().unwrap_or_default(),
            item.confidence_score.unwrap_or_default(),
            item.summary.clone().unwrap_or_default(),
            item.body.clone().unwrap_or_default(),
            serde_json::to_string(&item.source_anchors).unwrap_or_default(),
            item.source_quote.clone().unwrap_or_default()
        );
        if !item.product_tags.is_empty() {
            s.push_str(&format!("\n  productTags={}", item.product_tags.join(",")));
        }
        if !item.business_topics.is_empty() {
            s.push_str(&format!("\n  businessTopics={}", item.business_topics.join(",")));
        }
        s
    };
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib render_chunk_includes_product_tags render_chunk_skips_empty_tags`
Expected: 两个 PASS。再跑 `cargo test --lib knowledge_router`（含 PBT chunk_type_routing 等）确认无回归——尤其 `format_..._with_roles` 的既有分桶测试不受影响（只加了行尾字段）。

> 注意：若有 PBT 锁死 render 逐字输出（如 `chunk_type_routing` 锁了 prompt 字符串），加字段可能让其失败。实证 `format_..._with_roles` 的注释（:194）说"DEFAULT 四态与改造前逐字等价（PBT chunk_type_routing 锁死）"——确认该 PBT 锁的是**分桶/header 顺序**还是**逐字全文**。若锁逐字全文，需同步更新该 PBT 的期望值（属本 task 合理改动，注明）。实现者跑 `cargo test --lib chunk_type_routing` 确认。

- [ ] **Step 5: 提交**

```bash
git add src/agent/knowledge_router.rs
git commit -m "feat(structured-org): 知识切片注入加 productTags/businessTopics(缺口7软增强)

render_chunk 输出加两个现成字段(非空才渲染),让AI看到知识切片业务主题,
与素材tags语义对照自主配套。不建关联不改路由。"
```

---

### Task 2: 缺口8 素材侧 tags 激活（render 注入 + upload 写 + list 过滤）

**Files:**
- Modify: `src/agent/media_send.rs:52-68`（`render_candidate_lines` 加 tags 段；测试加进该文件 `mod tests`）
- Modify: `src/routes/media_assets.rs`（upload 解析 + 写 tags）
- Modify: `src/routes/assets.rs:49-63`（list 加 `?tag=` 过滤）

**Interfaces:**
- Consumes: `ContentAsset.tags: Vec<String>`（`models.rs:687`）、`target_stages: Option<Vec<String>>`。
- Produces: 无新对外接口（render 多一段、upload 写 tags、list 多一个过滤参数）。

**背景**：素材 tags 是半死字段——upload 硬编码空、render_candidate_lines 不渲染、list 不能按 tag 筛。本 task 三处激活。

- [ ] **Step 1: 写失败测试（render_candidate_lines 纯函数）**

加到 `src/agent/media_send.rs` 的 `#[cfg(test)] mod tests`：

```rust
    #[test]
    fn render_candidate_includes_tags() {
        let mut a = ContentAsset::default();
        a.title = "报价单".to_string();
        a.tags = vec!["报价类".to_string(), "价格".to_string()];
        a.media_type = Some("file".to_string());
        let out = render_candidate_lines(&[&a]);
        assert!(out.contains("报价类"), "候选清单应渲染 tags");
    }

    #[test]
    fn render_candidate_skips_empty_tags() {
        let mut a = ContentAsset::default();
        a.title = "无标签素材".to_string();
        a.media_type = Some("file".to_string());
        // tags 留空
        let out = render_candidate_lines(&[&a]);
        assert!(!out.contains("标签:"), "空 tags 不渲染标签段");
    }
```

> 确认 `ContentAsset` 有 `Default`；若无，用 `media_send.rs` / `models.rs` 既有 ContentAsset fixture 构造（grep 现有 render_candidate 测试看怎么造的）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib render_candidate_includes_tags render_candidate_skips_empty_tags`
Expected: `render_candidate_includes_tags` FAIL（当前不渲染 tags）。

- [ ] **Step 3: 实现 render_candidate_lines 注入 tags**

`media_send.rs:52-68` 的 `render_candidate_lines`，在"表达:{pref}"后加可选 tags 段。当前：

```rust
        let stages = a.target_stages.as_ref().map(|v| v.join(",")).unwrap_or_default();
        let pref = a.expression_pref.as_deref().unwrap_or("file_support");
        let hint = a.send_trigger_hint.as_deref().unwrap_or("");
        out.push_str(&format!(
            "- [id:{id}] {} | 阶段:{stages} | 表达:{pref}\n  触发提示:{hint}\n",
            a.title
        ));
```

改为（tags 非空时插入"| 标签:..."）：

```rust
        let stages = a.target_stages.as_ref().map(|v| v.join(",")).unwrap_or_default();
        let pref = a.expression_pref.as_deref().unwrap_or("file_support");
        let hint = a.send_trigger_hint.as_deref().unwrap_or("");
        let tags_seg = if a.tags.is_empty() {
            String::new()
        } else {
            format!(" | 标签:{}", a.tags.join(","))
        };
        out.push_str(&format!(
            "- [id:{id}] {} | 阶段:{stages} | 表达:{pref}{tags_seg}\n  触发提示:{hint}\n",
            a.title
        ));
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib render_candidate_includes_tags render_candidate_skips_empty_tags media_send`
Expected: 新增 2 个 PASS；既有 media_send 测试无回归。

- [ ] **Step 5: upload 写 tags**

`src/routes/media_assets.rs` 的 `upload_media_asset`：现在 multipart 循环里没收 tags、构造 ContentAsset 时硬编码 `tags: vec![]`（:152）。

1）multipart 循环加一个 `tags` 字段分支（复用 target_stages 的逗号分隔解析，:77-88 同款）：

```rust
            "tags" => {
                tags = field
                    .text()
                    .await
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
```

2）循环前声明 `let mut tags: Vec<String> = vec![];`（在 target_stages 声明旁，:46 附近）。

3）构造 ContentAsset 时把 `tags: vec![]`（:152）改为 `tags`。

- [ ] **Step 6: list 加 `?tag=` 过滤**

`src/routes/assets.rs` 的 `ContentAssetQuery`（:25-28）加 `tag: Option<String>`；`list_content_assets`（:49-63）在 filter 构造里加（kind 过滤旁，:59）：

```rust
    if let Some(tag) = query.tag {
        if !tag.is_empty() {
            filter.insert("tags", tag);
        }
    }
```

> MongoDB 数组字段等值匹配：`{ tags: "报价类" }` 命中 tags 数组含该元素的文档。无需新索引（tag 筛选量小，200 limit 内）。

- [ ] **Step 7: 跑 lib 编译 + 测试**

Run: `cargo build --lib && cargo test --lib media_send`
Expected: 编译通过；render 测试 PASS；既有 media_assets/assets 测试无回归。

- [ ] **Step 8: 提交**

```bash
git add src/agent/media_send.rs src/routes/media_assets.rs src/routes/assets.rs
git commit -m "feat(structured-org): 素材tags激活(候选清单注入+upload写入+list按tag过滤)(缺口8素材侧)

render_candidate_lines注入tags(非空才渲染)+upload收multipart tags替换硬编码空+
list加?tag=数组等值过滤。tags不作硬门(agent-first)。"
```

---

### Task 3: 缺口8 名片侧 tags 新增（字段 + create 写 + render 注入 + list 输出）

**Files:**
- Modify: `src/models.rs`（ReferralCard 加 `tags` 字段 + roundtrip 测试）
- Modify: `src/routes/referral_cards.rs`（ReferralCardRequest 加 tags + create 写 + list 输出）
- Modify: `src/agent/referral.rs:45-70`（`render_referral_lines` 加 tags 段；测试加进该文件 `mod tests`）

**Interfaces:**
- Consumes: `ReferralCard`（`models.rs:851-871`）。
- Produces: `ReferralCard.tags: Vec<String>`。

**背景**：ReferralCard 完全无 tags 字段。本 task 新增 + create 写入 + 候选清单注入 + list 输出，与素材侧对称。

- [ ] **Step 1: ReferralCard 加字段 + 向后兼容测试**

`src/models.rs` 的 ReferralCard（:851-871），在 `target_stages` 字段旁加：

```rust
    #[serde(default)]
    pub tags: Vec<String>,
```

向后兼容测试加到 `models.rs` 的 ReferralCard roundtrip 测试旁（`:5688` 附近的 `referral_card_roundtrip` 类似测试）。先看现有该测试，append 一个旧文档（无 tags）反序列化测试：

```rust
    #[test]
    fn referral_card_without_tags_deserializes_to_empty() {
        // 旧名片文档无 tags 字段 → #[serde(default)] 回落空 Vec。
        let doc = mongodb::bson::doc! {
            "workspace_id": "ws1",
            "target_wxid": "wxid_boss",
            "display_name": "老王",
            "send_trigger_hint": "签约时引荐",
            "target_stages": ["意向"],
            "enabled": true,
            "review_status": "approved",
            "created_at": DateTime::now(),
            "updated_at": DateTime::now(),
        };
        let card: ReferralCard = mongodb::bson::from_document(doc).unwrap();
        assert!(card.tags.is_empty(), "旧名片无 tags 字段应回落空 Vec");
    }
```

> 字段顺序/具体字段名以现有 ReferralCard roundtrip 测试（`models.rs` 的 `referral_card_*` 测试）为准，实现者先读那个测试对齐字段。

- [ ] **Step 2: 跑测试确认通过**

Run: `cargo test --lib referral_card_without_tags`
Expected: PASS（加了 `#[serde(default)]` 字段后旧文档反序列化正常）。

- [ ] **Step 3: 写 render_referral_lines 注入 tags 失败测试**

加到 `src/agent/referral.rs` 的 `#[cfg(test)] mod tests`（现有 `assist_mode_override_beats_account_flag` 等测试旁）：

```rust
    #[test]
    fn render_referral_includes_tags() {
        let mut card = ReferralCard {
            id: None, workspace_id: "ws".into(), account_id: None,
            target_wxid: "wxid_boss".into(), display_name: "老王".into(),
            send_trigger_hint: "签约时引荐".into(), target_stages: vec!["意向".into()],
            tags: vec!["高客单".into()],
            enabled: true, review_status: "approved".into(), review_note: None,
            created_at: DateTime::now(), updated_at: DateTime::now(),
        };
        let out = render_referral_lines(&[&card], None);
        assert!(out.contains("高客单"), "引荐候选应渲染 tags");
        card.tags.clear();
        let out2 = render_referral_lines(&[&card], None);
        assert!(!out2.contains("标签:"), "空 tags 不渲染标签段");
    }
```

> ReferralCard 字面量字段以 Step 1 加完 tags 后的实际结构为准（含新加的 tags）。实现者对齐字段顺序。

- [ ] **Step 4: 跑测试确认失败**

Run: `cargo test --lib render_referral_includes_tags`
Expected: FAIL（当前 render_referral_lines 不渲染 tags；且未加 tags 字段前编译不过——Step 1 已加字段，故是断言 FAIL）。

- [ ] **Step 5: 实现 render_referral_lines 注入 tags**

`referral.rs:54-61` 的候选渲染循环，在"阶段:{stages}"后加可选 tags 段。当前：

```rust
        let stages = c.target_stages.join(",");
        out.push_str(&format!(
            "- [card:{id}] {} | 阶段:{stages} | 触发提示:{}\n",
            c.display_name, c.send_trigger_hint
        ));
```

改为：

```rust
        let stages = c.target_stages.join(",");
        let tags_seg = if c.tags.is_empty() {
            String::new()
        } else {
            format!(" | 标签:{}", c.tags.join(","))
        };
        out.push_str(&format!(
            "- [card:{id}] {} | 阶段:{stages}{tags_seg} | 触发提示:{}\n",
            c.display_name, c.send_trigger_hint
        ));
```

- [ ] **Step 6: create 写 tags + list 输出**

`src/routes/referral_cards.rs`：
1）`ReferralCardRequest`（:25-33）加 `#[serde(default)] tags: Vec<String>,`。
2）`create_referral_card` 构造 ReferralCard（:59-73）加 `tags: payload.tags,`。
3）`list_referral_cards` 的 json! 输出（:89-102）加 `"tags": card.tags,`。

- [ ] **Step 7: 跑测试确认通过 + 编译**

Run: `cargo test --lib render_referral_includes_tags referral_card_without_tags && cargo build --lib`
Expected: 两测 PASS；编译通过；既有 referral 测试无回归。

- [ ] **Step 8: 提交**

```bash
git add src/models.rs src/routes/referral_cards.rs src/agent/referral.rs
git commit -m "feat(structured-org): 名片新增tags字段(create写+候选清单注入+list输出)(缺口8名片侧)

ReferralCard加tags(serde default向后兼容)+create写入+render_referral_lines注入(非空才渲染)+list输出。与素材侧对称。"
```

---

### Task 4: 前端 tags UI（素材编辑/筛选/upload + 名片 create/展示）

**Files:**
- Modify: `frontend/src/types/index.ts`（ContentAsset 已有 tags？确认；ReferralCard + ReferralCardDraft 加 tags）
- Modify: `frontend/src/features/content-assets/index.tsx`（素材 tags 编辑/upload/筛选）
- Modify: `frontend/src/features/referral-cards/index.tsx`（名片 tags create/展示）
- Modify: `frontend/src/stores/contentStore.ts` / 名片 store（如需传 tags）

**Interfaces:**
- Consumes: Task 2/3 的后端端点（upload 收 tags、list 返回 tags、list `?tag=` 过滤、名片 create 收 tags、名片 list 返回 tags）。
- Produces: 无（纯前端）。

**背景**：前端零 tags UI。补素材/名片的 tags 编辑、展示、素材按 tag 筛选。

- [ ] **Step 1: 类型加 tags**

`frontend/src/types/index.ts`：
- ContentAsset（:165-185）：当前**无 tags 字段**（实证：只有 sendable 等，无 tags）。在 `sendable?: boolean;`（:184）旁加 `tags?: string[];`。
- ReferralCard（:119-132）加 `tags?: string[];`。
- ReferralCardDraft（:134-139）加 `tags: string;`（逗号分隔输入字符串）。

- [ ] **Step 2: 素材 tags UI**

`frontend/src/features/content-assets/index.tsx`：
- upload 表单（grep 上传 FormData 构造处）加 tags 输入（逗号分隔），append 到 FormData：`form.append("tags", tagsInput)`。
- MediaAssetRow 编辑表单（簇 C 加的编辑区）加 tags 输入（逗号分隔，同 targetStages 模式），保存时走 editAssetMeta 传 `{ tags: [...] }`。
- 列表展示：MediaAssetRow 显示 tags chips（复用既有 metaLine/badge 类名）。
- （可选）列表顶部 tag 筛选输入：调 loadAssets 时带 `?tag=`。本期可只做编辑+展示，筛选若复杂留后续——但 list 后端已支持 `?tag=`，前端加个输入框成本低，做。

- [ ] **Step 3: 名片 tags UI**

`frontend/src/features/referral-cards/index.tsx`：
- create 表单加 tags 输入（逗号分隔），提交时解析成数组传后端。
- 列表展示名片 tags chips。
- 设计语言遵现有 referral-cards 页类名，不新造样式。

- [ ] **Step 4: 构建验证**

Run: `cd frontend && npm run build`
Expected: 构建通过，无 TS 错误。文案"标签"无 no-human-takeover 禁词。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/types/index.ts frontend/src/features/content-assets/index.tsx frontend/src/features/referral-cards/index.tsx frontend/src/stores/contentStore.ts
git commit -m "feat(structured-org): 前端tags UI(素材编辑/筛选/upload+名片create/展示)(缺口8前端)

素材upload/编辑加tags输入+列表tags chips+按tag筛选;名片create加tags+列表展示。复用既有设计系统类名。"
```

---

### Task 5: 集成测试（tags 落库 + list 过滤，`#[ignore]` / CI）

**Files:**
- Create: `tests/structured_organization_integration.rs`

**Interfaces:**
- Consumes: 既有测试设施（`tests/common/mod.rs` TestApp、簇C `tests/media_asset_crud_integration.rs` 直调 handler 惯例）。Task 2/3 端点行为。
- Produces: 无（测试）。

**背景**：render 注入是纯函数（Task 1/2/3 已 lib 测）。upload 写 tags + list 按 tag 过滤是 DB 副作用，用 testcontainers 钉端到端。全部 `#[ignore]`。

- [ ] **Step 1: 读测试设施**

读 `tests/common/mod.rs` + 簇C `tests/media_asset_crud_integration.rs`（直调 handler 真函数惯例、seed content_asset fixture）。upload 端点取 Multipart（tests crate 不可构造，同簇 B/C 限制）——所以 upload 写 tags 的端到端改为：**直接 seed 一个带 tags 的 ContentAsset 入库 → 直调 list_content_assets（带 tag 过滤）→ 断言命中**。upload 的 tags 解析由 render 纯函数 + 代码审查保证（Multipart 不可测，在报告说明，参照簇 C）。

- [ ] **Step 2: 写测试（实义断言，全部 #[ignore]）**

```rust
//! 簇 D 结构化组织集成测试：素材 tags 落库 + list 按 tag 过滤。
//! 全部 #[ignore]，需 Docker testcontainers。直调 handler 真函数（本仓既有惯例）。
mod common;
use common::*;

// 缺口8：list 按 tag 过滤命中含该 tag 的素材、不含的被排除。
#[tokio::test]
#[ignore]
async fn list_filters_by_tag() {
    // seed 两条素材：A tags=["报价类"]、B tags=["案例类"] → 直调 list_content_assets(tag="报价类")
    // → 结果含 A、不含 B。
}

// 缺口8：跨 workspace 的 tag 过滤不泄漏（workspace 隔离）。
#[tokio::test]
#[ignore]
async fn list_tag_filter_respects_workspace() {
    // other_ws 有一条 tags=["报价类"] 的素材 → default workspace 的 admin 按 tag="报价类" 查 → 不含它。
}
```

实现者填充：seed 用 `state.db.content_assets().insert_one(...)` 构造带 tags 的 ContentAsset；直调 `list_content_assets`（带 `ContentAssetQuery { tag: Some(...), .. }`）；断言返回的 items 含/不含特定 id。helper 对齐 common/mod.rs 实际。若需放开 list_content_assets 可见性（pub(super)→pub）让 tests crate 直调，照簇 C 先例（纯可见性零逻辑），在报告列出。

> **list_filters_by_tag 是缺口8 检索的核心回归**，必须扎实（真 seed 两条不同 tag 的素材，断言过滤精确命中）。

- [ ] **Step 3: 编译验证（不跑 ignored）**

Run: `cargo test --test structured_organization_integration --no-run`
Expected: 编译通过。本地无 Docker，不跑 `-- --ignored`，CI integration job 跑。

- [ ] **Step 4: 提交**

```bash
git add tests/structured_organization_integration.rs
git commit -m "test(structured-org): tags落库+list按tag过滤集成测试(#[ignore]/CI)

list按tag精确过滤(命中含该tag/排除不含)+workspace隔离。upload因Multipart限制由纯函数+审查保证。"
```

---

## 执行顺序与依赖

- **Task 1**（缺口7 知识侧注入）独立——只改 knowledge_router.rs render_chunk。
- **Task 2**（缺口8 素材侧）独立——改 media_send.rs + media_assets.rs + assets.rs。
- **Task 3**（缺口8 名片侧）独立——改 models.rs + referral_cards.rs + referral.rs。
- **Task 4**（前端）依赖 Task 2/3 后端端点（upload/list 收返 tags、名片 create/list tags）。
- **Task 5**（集成测试）依赖 Task 2/3 端点行为。

顺序：1 → 2 → 3 → 4 → 5。Task 1/2/3 各含纯函数渲染测试（可真测），改不同文件、可独立 reviewer 判。Task 1-3 后端 `cargo test --lib` 验；Task 4 前端 `npm run build` 验；Task 5 仅 `--no-run` 编译验（CI 跑 ignored）。

