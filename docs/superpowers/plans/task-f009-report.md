# F-009 修复报告：kind_has_entries 只算 active 条目

## 状态
DONE

## 问题
`customer_stage` 等维度字典只剩 deprecated 残留（active=0）时，运营填「目标阶段」任何值被 400 拒。
根因：`kind_has_entries` 把 active+deprecated 都算 → 有 deprecated 残留就 `!e.is_empty()==true` → `lookup_dict` 不降级 `KindUnconfigured` → Miss → Reject。

## Read 亲验证据（改前现状）
- `src/agent/taxonomy.rs:325-333` `kind_has_entries`：谓词确为 `.is_some_and(|e| !e.is_empty())`，不区分 status。
- `src/agent/taxonomy.rs:78-95` `CachedEntry`：有 `status: String` 字段，注释标 `"active" | "deprecated"`。
- `src/agent/taxonomy.rs:129-160` `reload_from_db`：`find(doc!{ "current_version": true })`，无 status 过滤，active+deprecated 都进缓存。
- `src/agent/taxonomy.rs:220-250` `check_value`：显式依赖缓存里的 deprecated 条目返回 `TaxonomyMatch::Deprecated`（canonical 与 alias 各一路）——故缓存加载不可动。

## grep 调用方结果
`grep -rn kind_has_entries src/`：
- 定义 `taxonomy.rs:325`；文档注释 `dimension_registry.rs:103,158`。
- 唯一生产调用点：`dimension_registry.rs:172`（`lookup_dict` 的 `DictLookup::Miss if !kind_has_entries(...)` → `KindUnconfigured` 降级判断）。
- 其余命中均在 `taxonomy.rs` 测试区（:935-975）。
与研究结论一致。

## 改动（仅 taxonomy.rs）
1. 谓词（约 :331）：`.is_some_and(|e| !e.is_empty())` → `.is_some_and(|e| e.iter().any(|c| c.status == "active"))`。
2. 同函数文档注释（约 :324）更新为 active-only 语义 + F-009 fail-soft 说明。
3. 新单测 `kind_has_entries_false_when_only_deprecated`（跟在 `kind_has_entries_false_when_empty` 后）：
   - 只有 1 条 deprecated `customer_stage` → 返回 false；
   - active+deprecated 混存 → 仍返回 true。

**未动**：`reload_from_db` 缓存加载、`check_value`、`dimension_registry`。

## 验证
- `cargo check`：通过（1m03s，无 error/warning 新增）。
- `cargo test --lib`：**1913 passed / 0 failed**（基线 ≥350，未回退）。
- 定向复核不回归：`match_to_dict_maps_all_variants`、`classify_known_accepts_canonical_and_deprecated`、`check_value_returns_deprecated_when_canonical_id_status_is_deprecated`、`kind_has_entries_true_when_configured`、`kind_has_entries_false_when_empty` 全 ok；新测 `kind_has_entries_false_when_only_deprecated` ok。

## commit
见 git log（fix(taxonomy): kind_has_entries 只算 active 条目...）。
