# 素材库 CRUD 补全（media asset CRUD completion）设计

> 簇 C / 8 缺口补全的第 3 个子项目。为素材库（content-assets / media-asset）补齐 edit / delete / disable 三个能力，对齐已完整的专属顾问名片（referral-cards 已有 toggle + delete）。簇 A（主动发送台账）、簇 B（标注质量门）已完成。

**Date:** 2026-06-22
**Status:** 设计已获批，待落实现计划（writing-plans）
**Scope:** 仅簇 C（素材 CRUD 补全，缺口 4）。簇 D（结构化组织：知识库关联 / 标签）后续独立 spec。

## 1. 背景与动机

素材库与专属顾问名片是两个**形态对称**的"AI 按触发条件主动发送物 + 人类标注审核"功能。当前能力不对称：

| 操作 | 名片（referral-cards） | 素材（content-assets） |
|---|---|---|
| 新建 | ✅ create | ✅ create（文本）/ upload（媒体） |
| 列表 | ✅ list | ✅ list |
| 审核 | ✅ review | ✅ review |
| 启停 | ✅ toggle（enabled） | ❌ **缺** |
| 编辑 | （无文件，改 create 即可） | ❌ **缺** |
| 删除 | ✅ delete | ❌ **缺** |

素材一旦上传，运营**无法编辑**（改个错别字的触发提示都得删了重传——但删也没有）、**无法停用**（只能 review 退 draft，语义混淆）、**无法删除**（DB 和磁盘只增不减）。缺口 4 给素材补齐 edit / delete / disable，对齐名片。

### 现状校准（实证，纠正初步摸底）

初步摸底有一处错误结论需纠正：`sendable` **不是死字段**。发送准入函数 `validate_asset_sendable`（`src/agent/media_send.rs:24`）**已经检查 `sendable == Some(true)`**：

```rust
pub(crate) fn validate_asset_sendable(asset: &ContentAsset) -> bool {
    asset.sendable == Some(true)
        && asset.review_status.as_deref() == Some("approved")
        && asset.media_type.as_deref().and_then(mcp_tool_for_media_type).is_some()
}
```

且存在索引 `{workspace_id: 1, sendable: 1, review_status: 1}`（`src/db/indexes.rs:202`）。所以发送门禁早已在看 `sendable`——本簇的 disable **只需加一个写 sendable 的端点**，不用改门禁逻辑。

## 2. 已锁定的关键决策（brainstorming 产出）

| # | 决策点 | 选择 | 理由 |
|---|---|---|---|
| C1 | disable 语义 | **启用 `sendable` 字段做启停**（对称名片 enabled） | `validate_asset_sendable` 已查 sendable，门禁现成；与 review_status（审核态）正交，语义干净——停用不必退审、重启不必重审 |
| C2 | delete 清磁盘 | **查引用计数再删**：无其他 asset 引用同 file_path 才物理删文件 | upload 不去重，同文件传两次 = 两条记录共享同一物理文件；盲删会让兄弟记录 file_path 指空 |
| C3 | edit 范围 | **元数据 + 换文件**（两个端点） | 用户要完整能力；元数据（高频、JSON）与换文件（低频、有副作用、multipart）职责分离 |
| C4 | edit 后重审 | **换文件 → 强制退 draft + 清 media_id；纯改元数据 → 不动 review_status** | 换文件 = 发送物变了，必须重新人工核验（AI 不自我核验红线）；描述词变不影响已核验的文件本体 |
| C5 | 端点组织 | **4 个端点**：PUT 改元数据 / POST 换文件 / POST toggle / DELETE | 见 §3 |

## 3. 架构与端点清单

簇 C 不新建模块。改动落在：

- `src/routes/media_assets.rs`：加 4 个 handler + 请求结构 + 纯函数（引用计数判定、换文件副作用判定）
- `src/media_storage.rs`：加 `delete_bytes`（物理删文件，幂等）
- `src/routes/assets.rs`：`list_content_assets` 输出补 `sendable` 字段（前端 toggle 回显）
- `src/routes/mod.rs`：挂载 4 个路由
- `frontend/src/features/content-assets/index.tsx` + `frontend/src/stores/contentStore.ts`：加编辑/换文件/停用/删除 UI + store actions

端点清单（全部 `workspace_id` scope 防 IDOR）：

```
PUT    /content-assets/:id        改元数据(JSON,api.put)——不动 file_*/media_id/review_status
POST   /content-assets/:id/file   换文件(multipart,api.postForm)——清 media_id + 退 draft + 旧文件按引用计数清理
POST   /content-assets/:id/toggle 启停(JSON {sendable})——对称名片 toggle
DELETE /content-assets/:id        删除——查引用计数,无其他引用才物理删文件
```

> HTTP 方法选择迁就前端 `api` helper 现状（`src/lib/api.ts` 有 `put`(JSON) / `postForm`(multipart POST) / `delete`，无 multipart PUT、无 patch）：换文件用 **POST**（对齐 postForm），改元数据用 **PUT**（对齐 put）。与簇 B 同样的迁就思路。

## 4. edit 两个端点

### 4.1 `PUT /content-assets/:id` —— 改元数据（JSON）

```
PUT /content-assets/:id  { title?, body?, tags?, url?, usageScene?, sendTriggerHint?, expressionPref?, targetStages?, requiresPrincipalApproval? }
  → parse_object_id + 回查 asset(workspace_id filter,跨 workspace/不存在 → 404)
  → targetStages 若提供 → normalize_target_stages(db, account_id_or_empty, stages),越界 → 400
  → $set 仅客户端提供的字段 + updated_at
  → 不动 file_*/media_id/review_status/sendable
```

**要点：**
- 只改"描述性"元数据，不碰文件本体、不碰审核态 → 不退 draft（描述词变不影响已核验的文件，对齐 C4）。
- `targetStages` 归一复用簇 B 的 `crate::agent::dimension_registry::normalize_target_stages`（单一事实源），越界 400，与 upload/create 行为一致。
- 归一 scope 用被编辑 asset 自身的 `account_id`（回查得到），缺失走空串（global scope）。
- **部分更新语义**：请求字段都是 `Option<T>`，`Some(v) → $set 该字段`、`None（JSON 缺失或 null）→ 不动该字段`。serde 默认把缺失和 null 都反序列化为 `None`，二者不区分——即本端点**不支持把字段显式清成 null**；要清空走传空串 `""` / 空数组 `[]`（它们是 `Some("")` / `Some([])`，会被 `$set`）。这是有意的简化（YAGNI：清空元数据是罕见操作，空串足够表达）。动态构造 `$set` doc，只插入 `Some` 的字段，全 `None` 则只更新 `updated_at`。

### 4.2 `POST /content-assets/:id/file` —— 换文件（multipart）

```
POST /content-assets/:id/file   (multipart: file + 可选 mediaType)
  → parse_object_id + 回查 asset(workspace_id filter → 404)
  → 记下旧 file_path（供后面引用计数清理）
  → 校验:文件非空、大小上限(media_max_file_size_mb)、media_type 白名单、sanitize_ext（同 upload）
  → store_bytes 落新文件(workspace/新sha/ext)
  → $set: file_path/file_name/file_size/mime_type/file_sha256=新值,
          media_id=None（清缓存,防 TTL 内发旧文件——硬约束）,
          review_status="draft"（发送物变了强制重审——红线）,
          updated_at
  → 旧文件清理:若旧 file_path ≠ 新 file_path,查 content_assets 还有没有别的记录(本 id 已被 $set 成新路径,自然排除)在同 workspace 引用旧 file_path → 无引用则 media_storage::delete_bytes 物理删旧文件
```

**要点：**
- **清 media_id=None**：`ensure_media_uploaded`（`media_send.rs:82`）在 TTL（`media_id_cache_ttl_hours` 默认 24h）内复用 media_id，不清则 AI 发旧文件；`outbox_dispatcher.rs` 还用 media_id 做去重防重发。换文件必清。
- **退 review_status=draft**：发送物变了 = 必须重新人工核验（AI 不自我核验红线）。
- **旧文件清理**：换文件后旧 file_path 可能无人引用，按引用计数清（与 delete 同一套逻辑，§5）。新旧 sha 相同（传同一文件）→ file_path 不变 → 不删。
- **落盘逻辑与 upload 重复** → 抽共用纯 IO helper（见 §7），upload 和换文件都调，避免两份漂移。

## 5. toggle + delete

### 5.1 `POST /content-assets/:id/toggle` —— 启停

```
POST /content-assets/:id/toggle  { sendable: bool }
  → parse_object_id
  → update_one filter { _id, workspace_id } $set { sendable, updated_at }
  → matched_count == 0 → 404
```

直接对称 `toggle_referral_card`（`referral_cards.rs:181`），字段名 `sendable`。停用语义干净：`validate_asset_sendable` 已查 `sendable==Some(true)`，`sendable=false` 的素材 AI 选不到。与 review_status 正交（启停不动审核态，重启不必重审）。

### 5.2 `DELETE /content-assets/:id` —— 删除（查引用计数再删）

```
DELETE /content-assets/:id
  → parse_object_id + 回查 asset(workspace_id filter → 404)拿 file_path
  → delete_one { _id, workspace_id }（deleted_count == 0 → 404）
  → 若有 file_path:count content_assets {workspace_id, file_path}（本记录已删,自然排除）
      → count == 0 → media_storage::delete_bytes 物理删
      → count > 0  → 保留文件（有兄弟引用）
  → 物理删文件失败 fail-soft:只 warn 不返 Err（DB 已删=既成事实,残留文件无害）
```

**要点：**
- **引用计数**：upload 不去重，同文件多记录共享 file_path；删一条前确认无兄弟引用，否则物理删让兄弟指空。
- **查询时机**：先 delete_one 再 count，本记录已移除，`count > 0` 即有兄弟。
- **fail-soft 删文件**：物理删失败不回滚 DB（残留文件无害，回滚更复杂）。

### 5.3 `media_storage::delete_bytes`

新增 `pub async fn delete_bytes(root: &Path, rel: &str) -> io::Result<()>`：`tokio::fs::remove_file(root.join(rel))`，文件不存在（NotFound）视为成功（幂等）。

## 6. 前端

`content-assets/index.tsx` 媒体行 `MediaAssetRow` 现仅"标记为可发送"（approve）按钮；`contentStore.ts` 仅 loadAssets/createAsset/uploadMediaAsset/reviewMediaAsset。

**`stores/contentStore.ts`** 加 4 个 action（照搬名片 store / reviewMediaAsset 的 setBusy/setError + 刷新链路）：
- `editAssetMeta(id, fields)` → `api.put('/api/content-assets/${id}', fields)`
- `replaceAssetFile(id, file, mediaType)` → `api.postForm('/api/content-assets/${id}/file', formData)`
- `toggleAssetSendable(id, sendable)` → `api.post('/api/content-assets/${id}/toggle', { sendable })`
- `deleteAsset(id)` → `api.delete('/api/content-assets/${id}')`

**`features/content-assets/index.tsx`** 在 `MediaAssetRow` 加：
- 停用/启用开关（读 list 返回的 `sendable`，缺省视为 true）
- 删除按钮（**二次确认**——不可逆）
- 编辑入口：改元数据表单（title/sendTriggerHint/targetStages 等）+ 换文件的文件选择
- 设计语言遵现有 `ContentAssets.module.css` + 既有按钮/表单类名，**不新造样式**（对齐项目"前端遵守现有设计系统"立场）。

## 7. 实现期细节

- **落盘共用 helper**：upload（`media_assets.rs:120-131` 的 sanitize_ext + sha + safe_relative_path + store_bytes）与换文件重复 → 抽 `store_uploaded_file(state, workspace, bytes, file_name, mime) -> Result<(rel, sha, ext), AppError>` 纯 IO helper，两处都调。实现时按重复度决定，倾向抽（DRY）。
- **引用计数纯函数**：`should_delete_physical_file(remaining_refs: u64) -> bool`（= `remaining_refs == 0`），lib 真测；count 查询做外层薄壳（同簇 B 内核/薄壳分层）。
- **换文件副作用纯函数**：把"换文件 → review_status=draft + media_id 清空"抽成可测的纯决策（如 `file_replace_effects() -> (new_status, clear_media_id)`），lib 真测语义不被误改。

## 8. 测试策略

| 层 | 测什么 | 方式 |
|---|---|---|
| 纯函数 | `should_delete_physical_file(refs)` = refs==0 才删 | lib 单测 |
| 纯函数 | 换文件副作用：review_status→draft + media_id→None | lib 单测 |
| 集成（CI/`#[ignore]`） | edit 元数据：改 title/trigger_hint 落库、review_status 不变；targetStages 越界 400 | testcontainers |
| 集成（CI/`#[ignore]`） | 换文件：file_* 更新 + media_id 清空 + review_status 退 draft；旧文件无兄弟引用物理删、有兄弟引用保留 | testcontainers |
| 集成（CI/`#[ignore]`） | toggle：sendable=false 落库；跨 workspace → 404（IDOR） | testcontainers |
| 集成（CI/`#[ignore]`） | delete：删 DB 记录；无兄弟引用物理删、有兄弟引用保留文件；跨 workspace → 404 | testcontainers |
| 前端 | `npm run build` 通过、无 TS 错误；操作入口用现有设计系统、删除二次确认 | 构建 + 人工对照 |

## 9. 边界 / 不做（YAGNI）

- **不做**素材版本历史（换文件直接覆盖，不存旧版本）。
- **不做**批量编辑/删除（逐个操作）。
- **不改**发送门禁逻辑（`validate_asset_sendable` 已查 sendable，本簇只加写端点）。
- **不动**文本类素材的 create/list 行为（只给 list 补返回 sendable 字段）。
- **不做** delete 的软删除/回收站（硬删，引用计数保护物理文件）。

## 10. 红线守卫

- **AI 不自我核验**：换文件 → 强制退 draft 重审（发送物变了）。
- **既成事实纪律**：delete / 换文件的物理删文件 fail-soft（删文件失败不回滚 DB、不返 Err）。
- **media_id 一致性**：换文件必清 media_id=None（防 TTL 内发旧文件）。
- **引用计数保护**：物理删文件前确认无兄弟记录引用同 file_path（防误删共享文件）。
- **workspace_id scope**：4 个端点全 workspace 隔离防 IDOR。
- **no-human-takeover 禁词**：端点/前端命名用"编辑/停用/删除/换文件"中性词。
- **target_stages 归一**：edit 元数据复用簇 B `normalize_target_stages`（单一事实源）。

## 11. 与后续簇衔接

- 簇 D（结构化组织：知识库关联 / 标签）独立。本簇的 edit 元数据端点未来可承载簇 D 的标签字段编辑，但本期不耦合。
