# 导入失败段可见性(M1) + 召回 haystack 补 product_tags(M4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让异步大文档导入某段抽取失败时前端预览页显式警示「Y 段失败、内容可能不完整」(M1)，并把 product_tags 补进召回 haystack 使其参与打分(M4)。

**Architecture:** M1 纯前端消费缺陷——后端 `progress.failed` 与 `result.importReport` 已全部返回(零后端改动)，只需前端 `ImportPreviewResult` 补类型 + step2 顶部渲染警示条。M4 是后端 `chunk_haystack` 补一行 product_tags 循环 + 一个 lib 单测。两者独立无耦合。

**Tech Stack:** Rust 2021 (Axum 后端) + React 19 + TypeScript + Vite (frontend/)。

## Global Constraints

- 基线门(合并前必过)：`cargo test --lib` ≥ 350 passed / 0 failed；`RUSTFLAGS=-D warnings cargo check --tests` 零警告；4 PBT 文件累计 ≥ 33/0；3 lint(`scripts/check-no-human-takeover.sh` / `check-no-model-hint.sh` / `check-evolution-isolation.sh`)。
- 反过拟合红线：不改任何阈值 / 业务逻辑 / prompt / guards 去迎合测试；M4 仅补检索信号。
- DEFAULT 字节等价：failed=0(同步单段路径)与 product_tags 空 Vec 时行为与今天完全一致，零回归。
- 前端颜色纪律 / 设计系统：复用现有 `wikiAlert` 样式类，不新造颜色 token(见 `docs/frontend-design-system.md`)。
- 后端零改动：不碰 import.rs / import_worker.rs / 任何 job / API(M1 数据链已就绪，file:line 见 spec)。
- 提交信息以 `Co-Authored-By: Claude <noreply@anthropic.com>` 结尾。未经用户许可不 push / 不建 PR。

---

### Task 1: M4 — chunk_haystack 补 product_tags(后端 + lib 单测)

**Files:**
- Modify: `src/agent/knowledge_agent.rs`(`chunk_haystack` 函数，当前 :1886-1906；business_topics 循环当前 :1897-1900)
- Test: `src/agent/knowledge_agent.rs`(同文件 `#[cfg(test)] mod tests`，复用现有 `rk_chunk` helper :2081)

**Interfaces:**
- Consumes: `rk_chunk(title, body, wiki_type, confidence, priority) -> OperationKnowledgeChunk`(已存在 :2081，`product_tags` 初始为 `Vec::new()` :2102，可测试后直接赋值)；`rank_key(query, &chunk, now) -> RankKey`(:1665)；`chunk_haystack(&chunk) -> String`(:1886，当前私有 `fn`)。
- Produces: `chunk_haystack` 输出新增包含 `product_tags` 各元素(空格分隔)，语义与 `business_topics` 并列。

- [ ] **Step 1: 写失败测试**

在 `src/agent/knowledge_agent.rs` 的 `mod tests` 内、`rank_key_blank_superseded_by_is_still_live`(:2179-2185)测试之后新增：

```rust
    #[test]
    fn chunk_haystack_includes_product_tags() {
        // product_tags 里的词(如品牌别名)即使不在 title/body，也应进 haystack 并参与打分。
        // 场景：body 不含 "星零感"，只有 product_tags 里有——修复前召不回，修复后能命中。
        let now = DateTime::now();
        let mut with_tag = rk_chunk("去眼袋方案", "微孔技术介绍", "product_fact", 0.5, 0);
        with_tag.product_tags = vec!["星零感".to_string()];
        let without_tag = rk_chunk("去眼袋方案", "微孔技术介绍", "product_fact", 0.5, 0);

        // haystack 直接断言含 tag 词
        assert!(
            chunk_haystack(&with_tag).contains("星零感"),
            "product_tags 的词必须进 haystack"
        );
        // rank_key：查询命中 product_tags 的词时，带 tag 的 chunk 相关度必 > 不带 tag 的
        let k_with = rank_key("星零感", &with_tag, now);
        let k_without = rank_key("星零感", &without_tag, now);
        assert!(
            k_with.effective_relevance_micros > k_without.effective_relevance_micros,
            "命中 product_tags 词的 chunk 相关度应高于同结构但无该 tag 的 chunk"
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib chunk_haystack_includes_product_tags`
Expected: FAIL —— `assert!(chunk_haystack(&with_tag).contains("星零感"))` 失败(修复前 product_tags 未进 haystack)，或 `k_with == k_without`(相关度无差异)。

- [ ] **Step 3: 实现最小修改**

在 `chunk_haystack`(:1886) 内，`business_topics` 循环(当前 :1897-1900)之后、`if let Some(x) = &c.wiki_type` 之前，插入 product_tags 循环。改后函数体：

```rust
fn chunk_haystack(c: &OperationKnowledgeChunk) -> String {
    let mut s = String::with_capacity(256);
    s.push_str(&c.title);
    if let Some(x) = &c.summary {
        s.push(' ');
        s.push_str(x);
    }
    if let Some(x) = &c.body {
        s.push(' ');
        s.push_str(x);
    }
    for t in &c.business_topics {
        s.push(' ');
        s.push_str(t);
    }
    for t in &c.product_tags {
        s.push(' ');
        s.push_str(t);
    }
    if let Some(x) = &c.wiki_type {
        s.push(' ');
        s.push_str(x);
    }
    s
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib chunk_haystack_includes_product_tags`
Expected: PASS。

- [ ] **Step 5: 跑既有 rank_key 测试确认零回归**

Run: `cargo test --lib rank_key_`
Expected: 5 个既有 rank_key 测试全 PASS(`rank_key_relevance_beats_static_confidence` / `_superseded_demoted_below_live_peer` / `_expired_demoted_below_live_peer` / `_empty_query_falls_back_to_static_order` / `_blank_superseded_by_is_still_live`)。它们的 chunk product_tags 均为空 Vec，haystack 不变，必全绿。

- [ ] **Step 6: Commit**

```bash
git add src/agent/knowledge_agent.rs
git commit -m "$(cat <<'EOF'
fix(knowledge): 召回 haystack 补 product_tags 使其参与打分(M4)

chunk_haystack 此前拼了 title/summary/body/business_topics/wiki_type
但独漏 product_tags，与同族 business_topics 不一致。补上后 list_catalog
的 rank_key/relevance_score 对 product_tags(品牌别名/管理员手工 tag,
词面不在正文时)也能命中打分。纯补检索信号，不改阈值/业务逻辑。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: M1 — 前端 ImportPreviewResult 补 importReport 类型

**Files:**
- Modify: `frontend/src/features/knowledge/steward.tsx`(`interface ImportPreviewResult` 当前 :604-608)

**Interfaces:**
- Consumes: 后端 `import_job_progress_json`(`src/routes/knowledge/import.rs:368-380`)返回的 `result` 字段结构，其 `importReport` 形状为 `{ totalSegments, succeeded, failed }`(`import.rs:512-516`)。
- Produces: `ImportPreviewResult.importReport?: { totalSegments: number; succeeded: number; failed: number }`——Task 3 的警示条据此读取。

- [ ] **Step 1: 加类型字段**

在 `frontend/src/features/knowledge/steward.tsx` 把 `ImportPreviewResult`(:604-608)改为：

```ts
interface ImportPreviewResult {
  document?: { title?: string; summary?: string; catalogSummary?: string } | null;
  items?: unknown[];
  chunks?: ImportPreviewChunk[];
  importReport?: { totalSegments: number; succeeded: number; failed: number };
}
```

- [ ] **Step 2: 类型检查通过**

Run: `cd frontend && npx tsc --noEmit`
Expected: 无新增类型错误(仅加了可选字段，不破坏任何现有读点)。

- [ ] **Step 3: Commit**

```bash
git add frontend/src/features/knowledge/steward.tsx
git commit -m "$(cat <<'EOF'
fix(import-ui): ImportPreviewResult 补 importReport 类型(M1 前置)

后端 import_job_progress_json 已在 result 里返回 importReport
{totalSegments,succeeded,failed}，前端此前 interface 不声明导致丢弃。
先补类型，警示条渲染在下个提交。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: M1 — step2 预览页顶部渲染失败段警示条

**Files:**
- Modify: `frontend/src/features/knowledge/steward.tsx`(step2 渲染区块——`{step === 2 ...}` 分支；`preview` 状态已由 `acceptPreviewResult` :711-718 填入)

**Interfaces:**
- Consumes: `preview: ImportPreviewResult | null` 状态；`preview.importReport`(Task 2 定义)。
- Produces: 用户可见的非阻断警示条(仅 `importReport.failed > 0` 时)。

- [ ] **Step 1: 定位 step2 渲染区块**

Run: `grep -n "step === 2" frontend/src/features/knowledge/steward.tsx`
先读该分支起始处(step2 区块顶部，chunk 列表渲染之前)，确认插入锚点。step1 的警示样例见 :848 `{error ? <div className="wikiAlert error">{error}</div> : null}`——复用 `wikiAlert` 类，本条用 warning 语气。

- [ ] **Step 2: 插入警示条**

在 step2 区块最顶部(chunk 列表 / 「应用」按钮之前)插入。用可选链兜底 `importReport` 缺失，仅 `failed > 0` 渲染：

```tsx
{preview?.importReport && preview.importReport.failed > 0 ? (
  <div className="wikiAlert" style={{ marginBottom: 10 }}>
    ⚠ 共 {preview.importReport.totalSegments} 段，其中 {preview.importReport.failed} 段抽取失败，
    下方仅为成功段内容，可能不完整。
  </div>
) : null}
```

> 说明：`className="wikiAlert"`(不带 ` error`)用中性/警告底色；若项目 `wikiAlert` 无独立 warning 变体，保留裸 `wikiAlert` 即可(与 error 条区分靠文案 ⚠ 前缀 + 无 error 类)。实现时先 grep `wikiAlert` 在 CSS 里的定义(`grep -rn "wikiAlert" frontend/src`)确认有无 warning 修饰类可用；有则用 `wikiAlert warning`，无则裸 `wikiAlert`。

- [ ] **Step 3: 类型检查 + 构建**

Run: `cd frontend && npx tsc --noEmit && npm run build`
Expected: 类型无错，`npm run build` 成功写 `frontend/dist`。

- [ ] **Step 4: 人工核对渲染逻辑(三条路径)**

阅读改后代码，确认三条路径行为正确(前端无单测框架，靠逻辑核对)：
1. 同步小文档(单段)：后端 `importReport.failed=0` → `failed > 0` 假 → 不渲染警示。✅ 零回归。
2. 异步全成功：`failed=0` → 不渲染。✅
3. 异步部分失败：`completed` 态 + `result.importReport.failed>0` → `acceptPreviewResult` 存 preview → 警示条显示「共 X 段，Y 段失败」。✅
4. 异步全失败：job 是 `failed` 态，走 :744 error 分支不进 step2，警示条不涉及。✅

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/knowledge/steward.tsx
git commit -m "$(cat <<'EOF'
fix(import-ui): 预览页显式警示抽取失败段(M1)

异步大文档导入某段 LLM 抽取失败时，job 仍 completed 只列成功段，
用户此前看不到失败提示、误以为内容完整。现在 step2 预览页顶部读
result.importReport.failed，>0 时非阻断警示「共 X 段，Y 段失败，
内容可能不完整」。failed=0(含同步单段路径)零回归。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 基线门验证(合并前)

**Files:** 无(仅验证)

- [ ] **Step 1: 后端 lib 测试基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed(含 Task 1 新增 `chunk_haystack_includes_product_tags`)。

- [ ] **Step 2: tests/ 编译零警告**

Run: `RUSTFLAGS=-D warnings cargo check --tests`
Expected: 退出码 0，无警告(本改动未动任何函数签名，tests/ 不应受影响，但按基线纪律必跑)。

- [ ] **Step 3: 前端构建**

Run: `cd frontend && npm run build`
Expected: 成功。

- [ ] **Step 4: 3 个 lint 门**

Run(逐个)：
```bash
bash scripts/check-no-human-takeover.sh
bash scripts/check-no-model-hint.sh
bash scripts/check-evolution-isolation.sh
```
Expected: 均退出 0。本改动新增文案「抽取失败 / 内容可能不完整」无禁词(人工接管 / takeover / 模型名)。

- [ ] **Step 5: PBT(可选本地，CI 必跑)**

Run: `cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter`
Expected: 累计 ≥ 33/0(本改动不涉这些域，应全绿；本地磁盘紧时可留给 CI)。

---

## Self-Review

**1. Spec coverage**：
- spec 组件1(ImportPreviewResult 补类型)→ Task 2 ✅
- spec 组件2(step2 警示条)→ Task 3 ✅
- spec 组件3(chunk_haystack 补 product_tags)→ Task 1 ✅
- spec 测试(M4 lib 单测 / M1 类型+构建 / 基线门)→ Task 1 Step1-5 / Task 2-3 / Task 4 ✅
- spec 非目标(后端零改动 / 不做进度条实时 / 不加 DB filter)→ 计划无相关任务 ✅

**2. Placeholder scan**：无 TBD/TODO；所有代码步骤含完整代码块；Task 3 Step2 的 `wikiAlert` 类名有 grep 兜底指令(非占位)。✅

**3. Type consistency**：`importReport?: { totalSegments; succeeded; failed }` 在 Task 2 定义、Task 3 读 `preview.importReport.failed`/`.totalSegments`，字段名一致；`chunk_haystack` / `rank_key` / `rk_chunk` 签名与 knowledge_agent.rs 现状(:1886/:1665/:2081)一致。✅

**4. 顺序合理性**：Task 1(后端 M4，独立)→ Task 2(前端类型，Task 3 前置)→ Task 3(前端渲染)→ Task 4(全量门)。Task 2 必须在 Task 3 前(后者依赖前者的类型)。✅
