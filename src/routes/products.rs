//! 产品目录 admin 路由（objective-purchase-facts G2）。
//!
//! `products` 是 **workspace 级**结构化产品实体（product_id/价格/SKU），admin 在
//! 「产品与成交」频道录入；agent 报价从此读准确价格，区别于 `operation_knowledge_chunks`
//! 的非结构化话术。`OutcomeEvent.product_ref` 以**快照**方式引用本表（下单时刻拷贝
//! 名/价/sku），故 product 改名/下架不污染历史成交（订单系统标准做法）。
//!
//! 路由（全部挂在 `/api/products` 下）：
//!
//! - `GET    /products`             列表（按 current_workspace，可 `?activeOnly=true`）
//! - `POST   /products`             新建（product_id 在 workspace 内唯一）
//! - `PUT    /products/:product_id` 更新（product_id 是业务主键 slug，不是 ObjectId）
//! - `POST   /products/:product_id/archive`   归档（status→archived，不物理删，历史成交仍可解引用）
//! - `POST   /products/:product_id/restore`   恢复（status→active）
//!
//! IDOR 红线（spec §3.5）：每个 handler 的 Mongo filter 必含 `workspace_id =
//! current_workspace`，写入侧 `workspace_id` **由 admin 会话注入、绝不信前端请求体**。
//! 跨 workspace 同名 product_id 合法且互不可见。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::FindOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::Product,
};

use super::AppState;

const ALLOWED_STATUS: &[&str] = &["active", "archived"];

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListQuery {
    /// 仅返 active 产品（供报价/前端可售列表）；默认 false（admin 看全量含归档）。
    #[serde(default)]
    active_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateRequest {
    pub product_id: String,
    pub name: String,
    /// 单价，最小币种单位整数（分，19900=¥199.00）。前端 ×100 转分后传入。
    #[serde(default)]
    pub price: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub sku: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub attributes: Document,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateRequest {
    pub name: String,
    /// 单价，最小币种单位整数（分）。
    #[serde(default)]
    pub price: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub sku: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub attributes: Document,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProductView {
    product_id: String,
    workspace_id: String,
    name: String,
    /// 单价，最小币种单位整数（分）；前端展示时 ÷100 转元。
    price: Option<i64>,
    currency: Option<String>,
    sku: Option<String>,
    status: String,
    summary: Option<String>,
    attributes: Value,
    created_at: i64,
    updated_at: i64,
}

impl From<&Product> for ProductView {
    fn from(p: &Product) -> Self {
        let attributes = mongodb::bson::Bson::Document(p.attributes.clone()).into_relaxed_extjson();
        Self {
            product_id: p.product_id.clone(),
            workspace_id: p.workspace_id.clone(),
            name: p.name.clone(),
            price: p.price,
            currency: p.currency.clone(),
            sku: p.sku.clone(),
            status: p.status.clone(),
            summary: p.summary.clone(),
            attributes,
            created_at: p.created_at.timestamp_millis(),
            updated_at: p.updated_at.timestamp_millis(),
        }
    }
}

/// 校验产品金额字段：price 非负（最小币种单位整数，分），currency 符合 ISO-4217 形态。
/// 复用 models 的纯校验函数，把 false 转成 400。
fn validate_product_money(price: Option<i64>, currency: Option<&str>) -> AppResult<()> {
    if !crate::models::is_valid_minor_amount(price) {
        return Err(AppError::BadRequest(
            "price 必须是非负整数（最小币种单位，如分）".to_string(),
        ));
    }
    if let Some(cur) = currency.map(str::trim).filter(|s| !s.is_empty()) {
        if !crate::models::is_valid_currency_code(cur) {
            return Err(AppError::BadRequest(
                "currency 必须是 ISO-4217 三位大写字母币种码（如 CNY）".to_string(),
            ));
        }
    }
    Ok(())
}

pub(super) async fn list_products(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if params.active_only {
        filter.insert("status", "active");
    }
    let mut cursor = state
        .db
        .products()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(p) = cursor.try_next().await? {
        items.push(ProductView::from(&p));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_product(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<CreateRequest>,
) -> AppResult<Json<Value>> {
    let product_id = body.product_id.trim().to_string();
    if product_id.is_empty() {
        return Err(AppError::BadRequest("productId 不能为空".to_string()));
    }
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name 不能为空".to_string()));
    }
    validate_product_money(body.price, body.currency.as_deref())?;
    let now = DateTime::now();
    let product = Product {
        id: None,
        // IDOR 红线：workspace_id 由会话注入，绝不信前端请求体。
        workspace_id: admin.current_workspace.clone(),
        product_id: product_id.clone(),
        name: body.name.trim().to_string(),
        price: body.price,
        currency: normalize_opt(body.currency),
        sku: normalize_opt(body.sku),
        status: "active".to_string(),
        summary: normalize_opt(body.summary),
        attributes: body.attributes,
        created_at: now,
        updated_at: now,
    };
    // (workspace_id, product_id) unique 索引是幂等门：DuplicateKey → 友好提示。
    state
        .db
        .products()
        .insert_one(&product, None)
        .await
        .map_err(|err| {
            if is_duplicate_key(&err) {
                AppError::BadRequest(format!("productId {product_id} 在当前工作区已存在"))
            } else {
                AppError::BadRequest(format!("创建失败: {err}"))
            }
        })?;
    Ok(Json(json!({ "item": ProductView::from(&product) })))
}

pub(super) async fn update_product(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(product_id): Path<String>,
    Json(body): Json<UpdateRequest>,
) -> AppResult<Json<Value>> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name 不能为空".to_string()));
    }
    validate_product_money(body.price, body.currency.as_deref())?;
    let now = DateTime::now();
    let mut set = doc! {
        "name": body.name.trim(),
        "attributes": &body.attributes,
        "updated_at": now,
    };
    insert_opt_i64(&mut set, "price", body.price);
    insert_opt_str(&mut set, "currency", normalize_opt(body.currency));
    insert_opt_str(&mut set, "sku", normalize_opt(body.sku));
    insert_opt_str(&mut set, "summary", normalize_opt(body.summary));
    let res = state
        .db
        .products()
        .update_one(
            doc! { "workspace_id": &admin.current_workspace, "product_id": &product_id },
            doc! { "$set": set },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "product {product_id} not found"
        )));
    }
    let refreshed = load_product(&state, &admin.current_workspace, &product_id).await?;
    Ok(Json(json!({ "item": ProductView::from(&refreshed) })))
}

pub(super) async fn archive_product(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(product_id): Path<String>,
) -> AppResult<Json<Value>> {
    set_product_status(&state, &admin.current_workspace, &product_id, "archived").await
}

pub(super) async fn restore_product(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(product_id): Path<String>,
) -> AppResult<Json<Value>> {
    set_product_status(&state, &admin.current_workspace, &product_id, "active").await
}

async fn set_product_status(
    state: &AppState,
    workspace_id: &str,
    product_id: &str,
    status: &str,
) -> AppResult<Json<Value>> {
    debug_assert!(ALLOWED_STATUS.contains(&status));
    let now = DateTime::now();
    let res = state
        .db
        .products()
        .update_one(
            doc! { "workspace_id": workspace_id, "product_id": product_id },
            doc! { "$set": { "status": status, "updated_at": now } },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "product {product_id} not found"
        )));
    }
    let refreshed = load_product(state, workspace_id, product_id).await?;
    Ok(Json(json!({ "item": ProductView::from(&refreshed) })))
}

async fn load_product(
    state: &AppState,
    workspace_id: &str,
    product_id: &str,
) -> AppResult<Product> {
    state
        .db
        .products()
        .find_one(
            doc! { "workspace_id": workspace_id, "product_id": product_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("product {product_id} not found")))
}

fn normalize_opt(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn insert_opt_i64(doc: &mut Document, key: &str, value: Option<i64>) {
    match value {
        Some(v) => {
            doc.insert(key, v);
        }
        None => {
            doc.insert(key, mongodb::bson::Bson::Null);
        }
    }
}

fn insert_opt_str(doc: &mut Document, key: &str, value: Option<String>) {
    match value {
        Some(v) => {
            doc.insert(key, v);
        }
        None => {
            doc.insert(key, mongodb::bson::Bson::Null);
        }
    }
}

fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    matches!(
        *err.kind,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(ref e)) if e.code == 11000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_product_money_rejects_negative_and_bad_currency() {
        // 金额整数化：i64 无 NaN/Inf，只需查非负。
        assert!(
            validate_product_money(Some(-1), None).is_err(),
            "负数金额拒绝"
        );
        assert!(validate_product_money(Some(0), None).is_ok(), "0 分合法");
        assert!(
            validate_product_money(Some(19900), None).is_ok(),
            "正常金额合法"
        );
        assert!(validate_product_money(None, None).is_ok(), "未设价合法");
        // currency ISO-4217 形态校验。
        assert!(validate_product_money(Some(19900), Some("CNY")).is_ok());
        assert!(
            validate_product_money(Some(19900), Some("cny")).is_err(),
            "小写币种拒绝"
        );
        assert!(
            validate_product_money(Some(19900), Some("RMB币")).is_err(),
            "非法币种拒绝"
        );
        assert!(
            validate_product_money(Some(19900), Some("  ")).is_ok(),
            "空白币种按未设处理"
        );
    }

    #[test]
    fn normalize_opt_trims_and_drops_empty() {
        assert_eq!(
            normalize_opt(Some("  abc ".to_string())),
            Some("abc".to_string())
        );
        assert_eq!(normalize_opt(Some("   ".to_string())), None);
        assert_eq!(normalize_opt(None), None);
    }

    #[test]
    fn insert_opt_writes_null_when_none() {
        let mut d = Document::new();
        insert_opt_str(&mut d, "currency", None);
        assert_eq!(d.get("currency"), Some(&mongodb::bson::Bson::Null));
        insert_opt_i64(&mut d, "price", Some(19900));
        assert_eq!(d.get_i64("price").unwrap(), 19900);
    }
}
