//! 跨 migration step 共享的纯函数 helper。
//!
//! 所有 helper 都是不接 `Database` 的纯计算，便于在 `mod tests` 里直接断言。
//! 把它们集中在一个文件里有两个好处：
//! 1. step 文件保持只有"读 mongo / 写 mongo"的纯 IO 形态，可读性高；
//! 2. `mod tests` 不依赖某个具体 step 文件存在与否，未来 step 删除时
//!    pure-fn 单测仍稳定。

use mongodb::bson::{doc, Bson, DateTime, Document};

/// merge_allowed_from_defaults：仅补齐缺失字段，保留运营人员已写过的值。
///
/// 详见 `m003_state_machine_allowed_from.rs`。
pub(crate) fn merge_allowed_from_defaults(
    state_machine: &mut Document,
    default_states: &[Document],
) -> bool {
    let Ok(states) = state_machine.get_array_mut("states") else {
        return false;
    };
    let mut changed = false;
    for state in states.iter_mut().filter_map(Bson::as_document_mut) {
        let Some(key) = state.get_str("key").ok().map(ToString::to_string) else {
            continue;
        };
        let Some(default_state) = default_states
            .iter()
            .find(|item| item.get_str("key").ok() == Some(key.as_str()))
        else {
            continue;
        };
        if !state.contains_key("allowedFrom") {
            let allowed = default_state
                .get_array("allowedFrom")
                .cloned()
                .unwrap_or_default();
            state.insert("allowedFrom", Bson::Array(allowed));
            changed = true;
        }
        if !state.contains_key("allowFromAny") {
            let allow_any = default_state.get_bool("allowFromAny").unwrap_or(false);
            if allow_any {
                state.insert("allowFromAny", true);
                changed = true;
            }
        }
    }
    changed
}

/// 把 `Vec<Bson>`（混合 String / Document）升级为全结构化 Document 数组。
/// 返回 `(new_array, changed)`：`changed=false` 表示数组中所有元素已经是
/// 结构化（有 `id` 字段），跳过本次写入。
pub(crate) fn upgrade_fact_array(
    raw: Option<&Vec<Bson>>,
    now: DateTime,
    counter: &mut i64,
) -> (Vec<Bson>, bool) {
    let Some(raw) = raw else {
        return (Vec::new(), false);
    };
    let mut changed = false;
    let mut out = Vec::with_capacity(raw.len());
    for item in raw {
        match item {
            Bson::String(text) => {
                changed = true;
                *counter += 1;
                out.push(Bson::Document(structured_fact_doc(text, now)));
            }
            Bson::Document(doc) => {
                if doc.get_str("id").is_ok() {
                    out.push(Bson::Document(doc.clone()));
                } else {
                    changed = true;
                    *counter += 1;
                    let text = doc.get_str("text").unwrap_or("").to_string();
                    let mut upgraded = structured_fact_doc(&text, now);
                    for (k, v) in doc.iter() {
                        if k == "id" || k == "text" {
                            continue;
                        }
                        upgraded.insert(k, v.clone());
                    }
                    out.push(Bson::Document(upgraded));
                }
            }
            other => {
                out.push(other.clone());
            }
        }
    }
    (out, changed)
}

pub(crate) fn structured_fact_doc(text: &str, now: DateTime) -> Document {
    doc! {
        "id": uuid::Uuid::new_v4().to_string(),
        "text": text,
        "confidence": 7i32,
        "importance": 5i32,
        "mayExpire": false,
        "sourceMessageIds": Vec::<Bson>::new(),
        "createdAt": now,
        "updatedAt": now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 波 D2：`merge_allowed_from_defaults` 只补缺失字段，不覆盖运营人员
    /// 已经写过的 `allowedFrom` / `allowFromAny` / `name` / `goal` 等。
    #[test]
    fn merge_allowed_from_does_not_overwrite_user_values() {
        let defaults = vec![
            doc! {
                "key": "new_contact",
                "allowedFrom": ["new_contact"]
            },
            doc! {
                "key": "cooldown",
                "allowedFrom": [],
                "allowFromAny": true
            },
        ];
        let mut machine = doc! {
            "states": [
                {
                    "key": "new_contact",
                    "name": "客户运营改名",
                    "allowedFrom": ["custom_state"]
                },
                {
                    "key": "cooldown",
                    "name": "我的冷却"
                }
            ]
        };
        let changed = merge_allowed_from_defaults(&mut machine, &defaults);
        assert!(changed, "应该有补齐字段");

        let states = machine.get_array("states").unwrap();
        let nc = states[0].as_document().unwrap();
        assert_eq!(nc.get_str("name").unwrap(), "客户运营改名");
        assert_eq!(
            nc.get_array("allowedFrom").unwrap()[0].as_str(),
            Some("custom_state")
        );
        let cd = states[1].as_document().unwrap();
        assert_eq!(cd.get_str("name").unwrap(), "我的冷却");
        assert!(cd.contains_key("allowedFrom"));
        assert_eq!(cd.get_bool("allowFromAny").unwrap(), true);
    }

    /// 波 D2：默认状态完全包含、用户没改时也不会做无意义写入。
    #[test]
    fn merge_allowed_from_skips_when_already_complete() {
        let defaults = vec![doc! {
            "key": "new_contact",
            "allowedFrom": ["new_contact"]
        }];
        let mut machine = doc! {
            "states": [
                {
                    "key": "new_contact",
                    "allowedFrom": ["new_contact"]
                }
            ]
        };
        let changed = merge_allowed_from_defaults(&mut machine, &defaults);
        assert!(
            !changed,
            "无需改动时返回 false，避免对 mongo 做空 update"
        );
    }
}
