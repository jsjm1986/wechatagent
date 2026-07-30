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

    /// 防腐烂:扫 src/routes/** 找所有投影函数(`fn <name>_json(...) -> Value`),
    /// 断言每个非豁免投影都"被契约测试覆盖"(投影名出现在某个 assert_contract_fixture
    /// 调用的窗口内)。新增投影忘配测试 → 红。现有 no_orphan_pub_async_route_handlers
    /// (mod.rs)手维护清单已腐烂,故用运行时扫描。纯 std 实现(本项目无 regex 依赖)。
    #[test]
    fn every_projection_has_contract_test() {
        use std::fs;
        use std::path::{Path, PathBuf};

        // 非实体投影豁免清单(helper / 非 model→Value / 异步生成器 / 其它批次域),逐条注明理由。
        const ALLOWLIST: &[&str] = &[
            "bson_from_json",        // helper:JSON→BSON Document,非投影
            "bson_doc_to_json",      // helper:Document→Value 通用桥
            "parse_warning_to_json", // 解析告警,非实体投影
            "vision_generate_json",  // async LLM 调用,非 model→Value
            "canonical_json",        // import hash canonicalizer, not an API projection
            "lesson_doc_to_json",    // 入参是裸 Document 非 model(批次2 评估纳入)
            "cohort_run_ids_json", // helper:返回裸数组(json!([hex...]))非对象投影,无顶层键集;形状由 proposal 详情端点 cohortRunIds 键间接覆盖
        ];

        fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    collect_rs(&p, out);
                } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }

        // 从一行形如 `... fn operation_knowledge_chunk_json(item: ...` 抽出投影名。
        fn extract_projection_name(line: &str) -> Option<String> {
            let after_fn = line.split("fn ").nth(1)?;
            let name_end = after_fn.find(|c| c == '(' || c == '<')?;
            let name = after_fn[..name_end].trim();
            if name.ends_with("_json") && !name.is_empty() {
                Some(name.to_string())
            } else {
                None
            }
        }

        let routes_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
        let mut files = Vec::new();
        collect_rs(&routes_dir, &mut files);

        // (路径, 源文本) 对:覆盖扫描要按文件名排除本 helper 模块(它自己满篇
        // `assert_contract_fixture` 字面量 + 文档注释里的投影名,会污染覆盖判定)。
        let all_src: Vec<(PathBuf, String)> = files
            .iter()
            .map(|f| (f.clone(), fs::read_to_string(f).unwrap_or_default()))
            .collect();

        // 覆盖集:一个投影"被契约测试覆盖"当且仅当它出现在某个**契约测试块**里。
        // 契约测试块 = 含 `assert_contract_fixture` 调用的代码区。production handler 里
        // 调用投影(document=4/chunk=14 次)不算覆盖——必须是测试块里调用,才挡得住
        // "有 production 调用方但零测试"的投影。
        // 纯 std 近似:在每次 `assert_contract_fixture` 出现处切前 600 / 后 200 字符窗口
        // (覆盖一个测试函数体),窗口里出现的 `_json` 名即记为已覆盖。
        // **排除 contract_snapshot.rs 自身**:它是 helper 定义处,满篇 `assert_contract_fixture`
        // 字面量与文档/ALLOWLIST 注释里的投影名,纳入会把任意投影名误判为已覆盖。
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (path, src) in &all_src {
            if path.file_name().and_then(|n| n.to_str()) == Some("contract_snapshot.rs") {
                continue;
            }
            let mut from = 0usize;
            while let Some(rel) = src[from..].find("assert_contract_fixture") {
                let pos = from + rel;
                let mut s = pos.saturating_sub(600);
                while s < src.len() && !src.is_char_boundary(s) {
                    s += 1;
                }
                let mut e = (pos + 200).min(src.len());
                while e < src.len() && !src.is_char_boundary(e) {
                    e += 1;
                }
                let window = &src[s..e];
                for tok in window.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    if tok.ends_with("_json") && tok.len() > 5 {
                        covered.insert(tok.to_string());
                    }
                }
                from = pos + "assert_contract_fixture".len();
            }
        }

        // 收集所有投影定义,逐个比对覆盖集。
        // 投影签名可能多行(operation_health_json/decision_review_json 的 `-> Value` 在独立行),
        // 故命中 `fn <name>_json` 后向下最多 6 行找 `-> Value`(遇到函数体 `{` 之前),
        // 单行签名也覆盖。否则多行签名投影会被静默漏扫 → 防腐烂护栏对它们成空门。
        let mut orphans = Vec::new();
        for (_path, src) in &all_src {
            let lines: Vec<&str> = src.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.contains("fn ") || !line.contains("_json") {
                    continue;
                }
                let Some(name) = extract_projection_name(line) else {
                    continue;
                };
                // 在签名行 + 后续最多 6 行内找 `-> Value`(到函数体 `{` 为止)。
                let mut returns_value = false;
                for probe in lines.iter().skip(i).take(7) {
                    if probe.contains("-> Value") {
                        returns_value = true;
                        break;
                    }
                    // 已进入函数体却没见到 `-> Value` → 不是投影(如 `-> AppResult<...>`)。
                    if probe.contains(" {") || probe.ends_with('{') {
                        break;
                    }
                }
                if !returns_value {
                    continue;
                }
                if ALLOWLIST.contains(&name.as_str()) {
                    continue;
                }
                if !covered.contains(&name) {
                    orphans.push(name);
                }
            }
        }

        assert!(
            orphans.is_empty(),
            "以下投影函数缺契约测试(加测试或加入 ALLOWLIST 并注明理由):\n{}",
            orphans.join("\n")
        );
    }
}
