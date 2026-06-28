//! 前后端契约快照机制（共享地基）。
//!
//! 每个实体级投影函数（`xxx_json(model) -> Value`）配一个 `#[cfg(test)]` 测试：
//! 构造全量 model → 调投影 → `assert_contract_fixture` → 写/读
//! `frontend/src/contracts/<name>.fixture.json`。fixture 是前后端唯一真相源：
//! 后端测试写它、前端 vitest 导入同一份做键集对账，杜绝手抄漂移。
//!
//! 默认只读对账；`UPDATE_SNAPSHOTS=1 cargo test --lib <name>` re-bless 写文件。

#![cfg(test)]

use serde_json::Value;

/// 递归排序对象键，消除嵌套 BSON Document 的键序抖动，保证快照稳定。
pub(crate) fn canonicalize(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::new();
            for (k, val) in entries {
                out.insert(k, canonicalize(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

/// 从顶层对象剔除指定键（spec §4.3 第三档：纯审计、前端不读的 raw Document 字段）。
/// 非对象原样返回。
pub(crate) fn project_subset(value: Value, drop_keys: &[&str]) -> Value {
    match value {
        Value::Object(mut map) => {
            for k in drop_keys {
                map.remove(*k);
            }
            Value::Object(map)
        }
        other => other,
    }
}

/// 契约 fixture bless/对账。`UPDATE_SNAPSHOTS=1` 写文件，否则只读对账。
#[allow(dead_code)] // 供 Task2+ 各域投影测试调用；本 Task 仅落地共享地基。
pub(crate) fn assert_contract_fixture(name: &str, value: Value) {
    let canonical = canonicalize(value);
    let pretty = serde_json::to_string_pretty(&canonical).expect("serialize fixture") + "\n";

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("frontend/src/contracts")
        .join(format!("{name}.fixture.json"));

    if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create contracts dir");
        std::fs::write(&path, &pretty).expect("write fixture");
        return;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "契约 fixture 缺失:{}\n请运行 UPDATE_SNAPSHOTS=1 cargo test --lib {} 生成(bless)。",
            path.display(),
            name
        )
    });
    let existing_canonical =
        canonicalize(serde_json::from_str(&existing).expect("fixture 不是合法 JSON"));
    let existing_pretty =
        serde_json::to_string_pretty(&existing_canonical).expect("re-serialize") + "\n";

    assert_eq!(
        existing_pretty, pretty,
        "\n投影 {name} 的线上形状与 fixture 不一致。\n\
         若后端投影确有变更:运行 UPDATE_SNAPSHOTS=1 cargo test --lib {name} re-bless,\n\
         再同步前端 vitest 契约测试的 CANONICAL_KEYS。\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonicalize_sorts_nested_keys() {
        let input = json!({"b": 1, "a": {"d": 2, "c": 3}});
        let out = canonicalize(input);
        let s = serde_json::to_string(&out).unwrap();
        assert_eq!(s, r#"{"a":{"c":3,"d":2},"b":1}"#);
    }

    #[test]
    fn project_subset_drops_top_level_keys() {
        let input = json!({"keep": 1, "dropMe": 2});
        let out = project_subset(input, &["dropMe"]);
        assert_eq!(out, json!({"keep": 1}));
    }
}
