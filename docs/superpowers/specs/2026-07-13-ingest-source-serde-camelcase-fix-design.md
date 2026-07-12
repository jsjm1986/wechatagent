# 外部源列表 serde 大小写不匹配修复 — 设计

**日期**：2026-07-13
**类型**：bug 修复（API 序列化层）
**发现来源**：2026-07-13 wiki 频道全功能 Playwright 真实验证（唯一真 bug）

## 问题陈述

控制台 → 外部源列表里，每行的「删除」「重新激活」按钮点了无效，「间隔」列显示 `undefinedm`。

## 根因（两端 file:line 亲验，非猜测）

- 存储结构体 `IngestSource`（`src/models.rs:1875-1908`）是 `Collection<IngestSource>`（`src/db/mod.rs:379`）的存储类型，字段以 **snake_case** 存 Mongo，结构体上**没有** `#[serde(rename_all = "camelCase")]`。
- 列表 handler `list_ingest_sources`（`src/routes/knowledge/sources_meta.rs:899`）用 `serde_json::to_value(src)` 直接把存储结构体转 JSON → 输出 snake_case 键：`source_id`/`schedule_minutes`/`last_fetched_at`/`last_etag`/`failure_streak`/`ingest_count`/`created_at`/`updated_at`。
- 前端接口 `IngestSourceItem`（`frontend/src/features/knowledge/steward.tsx:1840-1854`）声明全 camelCase；`load()`（:1871）裸 `r.json()` 无 snake→camel 转换。前端读 `it.sourceId`（:2013 行 key / :2050 handleReactivate / :2059 handleDelete 入参）、`it.scheduleMinutes`（:2021）等 → 恒 `undefined`。
- 后果：`DELETE /api/knowledge/ingest-sources/undefined`、`PATCH .../undefined` 永远打不中真实 `source_id`（`sources_meta.rs:988-991` PATCH、delete handler 均按 `source_id`+`workspace_id` 定位）→ 删除/重新激活对该行永久无效；`scheduleMinutes` undefined → 间隔列 `undefinedm`。`kind`/`url`/`label`/`status` 是单词，snake==camel 才照常显示，掩盖了问题。

**创建 handler 是反证**：`create_ingest_source`（`sources_meta.rs:947`）手写 `json!({ "sourceId": source_id })` = camelCase → 创建返回能被前端读到，唯独列表读不到，正是两处口径不一致的铁证。

## 为什么不能给结构体加 `rename_all = "camelCase"`

该结构体同时承担 **bson 存储**职责。加 `rename_all` 会让 serde 把存储键也写成 camelCase，而以下三处全部硬编码 snake_case、会一起崩：

1. worker 查询/更新：`ingest_worker.rs:268` `doc!{"workspace_id":ws,"status":...}`、:285 `doc!{"source_id":...}`、:288/:294 `"$set":{"last_fetched_at":...,"failure_streak":...}`/`"$inc":{"ingest_count":...}`。
2. 两条唯一/复合索引：`ingest_sources_source_id_unique` = `{source_id:1}`（`db/indexes.rs:1644`）、`ingest_sources_ws_kind_status_idx` = `{workspace_id:1,kind:1,status:1}`（`indexes.rs:1630`）。
3. 全部存量文档（生产 117 现有外部源行）。

故 struct 级 rename_all 被排除。修复必须只在 **API JSON 输出边界**做 snake→camel，存储层零改动。

## 修法（对齐全项目现有范式）

抽一个私有函数 `ingest_source_json(src: &IngestSource) -> Value`，逐字段构造 camelCase `json!({...})`，完全仿照 `operation_knowledge_chunk_json`（`src/routes/knowledge/mod.rs:283-317`）的做法。`list_ingest_sources` 里把 `items.push(serde_json::to_value(src)...)` 换成 `items.push(ingest_source_json(&src))`。

### 字段映射表（源 models.rs:1876-1908 → API camelCase，对照前端 steward.tsx:1840-1854）

| 存储字段(snake) | API 键(camel) | 转换 |
|---|---|---|
| `id: Option<ObjectId>` | 省略（前端 IngestSourceItem 不声明 id 字段，无需输出） | — |
| `source_id` | `sourceId` | 直传 |
| `workspace_id` | `workspaceId` | 直传 |
| `kind` | `kind` | 直传 |
| `url` | `url` | 直传 |
| `status` | `status` | 直传 |
| `schedule_minutes` | `scheduleMinutes` | 直传 |
| `label: Option<String>` | `label` | 直传 |
| `last_fetched_at: DateTime` | `lastFetchedAt` | `dt_to_string`（models.rs:3608）→ Option\<String\> RFC3339 |
| `last_etag: Option<String>` | `lastEtag` | 直传 |
| `last_error: Option<String>` | `lastError` | 直传 |
| `failure_streak: i32` | `failureStreak` | 直传 |
| `ingest_count: i64` | `ingestCount` | 直传 |
| `created_at: DateTime` | `createdAt` | `dt_to_string` |
| `updated_at: DateTime` | `updatedAt` | `dt_to_string` |

前端声明但表格未渲染的 `lastEtag`/`createdAt`/`updatedAt` 一并补齐，保持契约完整。`dt_to_string` 对 `None` 经 `.and_then` 产出 `null`，前端 `it.lastFetchedAt ? new Date(...) : "—"`（:2023）不炸。

## 全仓同类漏洞扫查结论

扫了全部 `serde_json::to_value(存储结构体) → API` 的点，逐个甄别：

- `ask_human_inbox.rs`（658/699/773）→ 均 `#[test]` 块内对 **InboxItem** DTO 的断言，InboxItem 自带 `#[serde(rename_all="camelCase")]`（:14-16），安全。
- `domain_profiles.rs:92` `profile_view` → `DomainProfile`（models.rs:1991）无 rename_all、是存储结构体，但前端 `DomainProfile` TS 类型**整体是 snake_case**（`strategyStore.ts:355-363` 读 `profile.profile_id`/`display_name`/`profile_dimensions`...），两端一致消费，是另一套自洽的 snake 契约，**非 bug**。
- `digest_inbox.rs:189/196`、`chat.rs:2125-2127` 的嵌套 `to_value` → 内层类型（completedSteps/plannedSteps/cards/budgetSnapshot 等）均带 camelCase 注解，安全。
- `evolution.rs:546`、`chunk_locks.rs`、`models.rs` → bson extjson 桥接或 test 块，非 storage→API。

**结论：`IngestSource` 列表是全仓唯一真漏点，修一处即闭合该 bug 家族。**

## 测试

后端单测（挂 `sources_meta.rs` 测试区，仿 mod.rs 现有 json 往返测试范式）：

1. `ingest_source_json_emits_camel_case`：构造字段全填的 `IngestSource`（`last_fetched_at=Some(...)`）→ `ingest_source_json` → 断言含 `sourceId`/`scheduleMinutes`/`lastFetchedAt`/`failureStreak`/`ingestCount`/`createdAt`/`updatedAt` 且值正确；断言**不含** snake_case 键（`assert!(v.get("source_id").is_none())` 等）。
2. `ingest_source_json_null_datetime`：`last_fetched_at=None` → 断言 `lastFetchedAt` 为 `null`。

基线门：`cargo test --lib`（当前 ≥350）不得回归，只加不减；`cargo check` 绿；`check-no-human-takeover` lint 新增行不踩禁词（本改纯字段映射，不涉及）。

## 验证 + 部署

真实 UI 回归（部署到 117 后，常驻 CDP 浏览器真实点击）：

1. 控制台 → 外部源，新建 `[E2E验证]` 源。
2. 断言删除按钮真删（`DELETE .../ing_xxx` 而非 `.../undefined`，删后行消失）。
3. 断言「间隔」列显示真实数字（如 `60m`）非 `undefinedm`。
4. 断言 `sourceId` 从 API 返回非空（可 fetch `/api/knowledge/ingest-sources` 直接核对）。
5. 清理测试数据，外部源列表回干净态。

部署：`cargo check` + `cargo test --lib` 本地绿 → 前端**无需 rebuild**（纯后端改动）→ paramiko `_push_bundle_direct.py` 推 + `_remote_run_direct.py` 重启 117 → 真实 UI 回归。

## 改动范围 / 风险

- 单文件：`src/routes/knowledge/sources_meta.rs`（新增 `ingest_source_json` 函数 + 改 list handler 一行 + 2 个单测）。
- 存储层、worker、索引、前端**零改动**。
- 风险极低：纯 API 输出层字段映射。唯一注意点是映射表字段别拼错（已对照两端亲验列全）。
- commit / PR 待改完、单测绿、部署验证后经用户显式许可再进行。
