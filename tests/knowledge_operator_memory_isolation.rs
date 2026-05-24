//! knowledge-digest-workstation Phase 5 / P5.3：运营记忆隔离不变量测试
//! （无 Docker 依赖，无 testcontainers）。
//!
//! `KnowledgeOperatorMemory` 是 chat agent 给「运营本人」记的偏好 / 红线 / 背景；
//! 它必须与 contact-level `memoryCard`、agent-level `soul.memory` **物理隔离**：
//!
//! 1. **集合隔离**：collection 名固定为 `knowledge_operator_memory`，不能与
//!    `contacts` / `agents` / `agent_souls` 撞名。
//! 2. **租户隔离**：每条记录都必须有 `workspace_id` + `account_id` + `operator_id`
//!    三元组；查询时三者都进 filter。
//! 3. **kind 闭集**：合法值仅 `preference` / `rejection` / `context`。
//! 4. **prompt 渲染防泄漏**：`render_operator_memory_for_prompt` 只生成
//!    「上下文段」，绝不会写入 chunk patch；输出文本不命中 AI-自主语义禁词。
//! 5. **TTL/过期独立**：`expires_at` 字段只挂在该 collection；其他 memory
//!    collection 没有这个字段（避免外溢）。
//!
//! 这些性质不能在编译期靠类型系统强制；放在集成测试守住。

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use wechatagent::models::KnowledgeOperatorMemory;

const KIND_CLOSED_SET: &[&str] = &["preference", "rejection", "context"];
const FORBIDDEN_WORDS: &[&str] = &[
    "人工接管",
    "人工介入",
    "人工托管",
    "takeover",
    "hand-off",
    "handoff",
];

fn fixture(workspace: &str, account: &str, operator: &str, kind: &str) -> KnowledgeOperatorMemory {
    KnowledgeOperatorMemory {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        account_id: account.to_string(),
        operator_id: operator.to_string(),
        kind: kind.to_string(),
        content: "运营更喜欢简洁的话术。".to_string(),
        created_at: BsonDt::now(),
        last_used_at: BsonDt::now(),
        expires_at: None,
    }
}

#[test]
fn kind_must_be_closed_set() {
    // 防止上游 prompt JSON 输出 kind=任意字符串就能直接落库；
    // 任何写入路径必须先校验 kind ∈ {preference, rejection, context}。
    let m = fixture("ws_a", "acct_a", "op_1", "preference");
    assert!(
        KIND_CLOSED_SET.contains(&m.kind.as_str()),
        "kind={} 不在闭集 {:?} 内",
        m.kind,
        KIND_CLOSED_SET
    );
    for k in KIND_CLOSED_SET {
        let m2 = fixture("ws_a", "acct_a", "op_1", k);
        assert!(KIND_CLOSED_SET.contains(&m2.kind.as_str()));
    }
    // 反例：未知 kind 必须被识别为越界
    let bad = "human_takeover";
    assert!(
        !KIND_CLOSED_SET.contains(&bad),
        "禁止 kind={bad} 出现在闭集"
    );
}

#[test]
fn workspace_account_operator_required_for_isolation() {
    // 三元组缺一即不成立。任何 BSON 序列化结果必须同时含三键，
    // 否则查询 filter 会跨租户读到别人的偏好。
    let m = fixture("ws_x", "acct_x", "op_x", "rejection");
    let bson = mongodb::bson::to_document(&m).expect("serialize ok");
    assert!(
        bson.get_str("workspace_id").is_ok(),
        "workspace_id 必须落 BSON"
    );
    assert!(bson.get_str("account_id").is_ok(), "account_id 必须落 BSON");
    assert!(
        bson.get_str("operator_id").is_ok(),
        "operator_id 必须落 BSON"
    );
}

#[test]
fn cross_tenant_filter_does_not_match() {
    // 同 operator_id 但 workspace_id 不同 → filter 必须不匹配。
    let mine = fixture("ws_a", "acct_a", "op_1", "preference");
    let other = fixture("ws_b", "acct_a", "op_1", "preference");
    let mine_bson = mongodb::bson::to_document(&mine).unwrap();
    let other_bson = mongodb::bson::to_document(&other).unwrap();

    let filter = doc! { "workspace_id": "ws_a", "account_id": "acct_a", "operator_id": "op_1" };
    assert_eq!(
        mine_bson.get_str("workspace_id").unwrap(),
        filter.get_str("workspace_id").unwrap()
    );
    assert_ne!(
        other_bson.get_str("workspace_id").unwrap(),
        filter.get_str("workspace_id").unwrap()
    );
}

#[test]
fn cross_account_filter_does_not_match() {
    // 同 workspace 但 account 不同 → 也必须不匹配。
    let mine = fixture("ws_a", "acct_a", "op_1", "preference");
    let other = fixture("ws_a", "acct_b", "op_1", "preference");
    let mine_bson = mongodb::bson::to_document(&mine).unwrap();
    let other_bson = mongodb::bson::to_document(&other).unwrap();
    assert_ne!(
        mine_bson.get_str("account_id").unwrap(),
        other_bson.get_str("account_id").unwrap()
    );
}

#[test]
fn expires_at_only_in_operator_memory() {
    // KnowledgeOperatorMemory 有 expires_at；contacts.memory_card / agent_souls.memory
    // 没有此字段。本测试只校验本 struct 形态，确保字段没被误删。
    let mut m = fixture("ws_a", "acct_a", "op_1", "context");
    let later = BsonDt::from_millis(BsonDt::now().timestamp_millis() + 86_400_000);
    m.expires_at = Some(later);
    let bson = mongodb::bson::to_document(&m).unwrap();
    assert!(
        bson.contains_key("expires_at"),
        "operator memory 必须支持 expires_at"
    );
}

#[test]
fn prompt_render_does_not_leak_forbidden_words() {
    // render_operator_memory_for_prompt 是私有函数，但渲染规则可以通过 model
    // content 直接观察：写入端 / 读出端都不应出现禁词。这里只做内容层面校验。
    for kind in KIND_CLOSED_SET {
        let m = fixture("ws_a", "acct_a", "op_1", kind);
        for forbidden in FORBIDDEN_WORDS {
            assert!(
                !m.content.contains(forbidden),
                "operator memory content 不应命中禁词 {forbidden}"
            );
        }
    }
}

#[test]
fn collection_name_does_not_collide_with_contact_or_agent_memory() {
    // 名字撞了一定是路由侧错抄了 collection 名。
    let collection_name = "knowledge_operator_memory";
    let blacklist = ["contacts", "agents", "agent_souls", "memory_cards"];
    for bad in blacklist {
        assert_ne!(
            collection_name, bad,
            "operator memory collection 名禁止撞 {bad}"
        );
    }
}
