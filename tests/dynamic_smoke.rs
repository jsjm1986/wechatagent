//! R5.0.1 三族异族硬门 + R5.2 轨迹裁判的纯函数单测宿主。
//!
//! `tests/common/dynamic.rs` 的指纹/家族判定是纯函数，本文件把它们暴露成可本地
//! `cargo test --test dynamic_smoke` 直跑的用例（无 Docker、无真模型、无 env-gate），
//! 守住 R5.0.1 异族判定的契约：同 host 不同 vendor 算异族、完全同源被识别、默认三角色异族。

mod common;

use common::dynamic::{read_role_fingerprints, ProviderFingerprint};

#[test]
fn default_config_three_families_distinct() {
    let fps = read_role_fingerprints();
    assert_eq!(fps.len(), 3, "应有 agent/roleplayer/judge 三角色");
    let fam: Vec<String> = fps.iter().map(|f| f.family()).collect();
    assert_ne!(fam[0], fam[1], "agent vs roleplayer 默认应异族");
    assert_ne!(fam[0], fam[2], "agent vs judge 默认应异族（同 host rsxermu 不同 vendor claude/gpt）");
    assert_ne!(fam[1], fam[2], "roleplayer vs judge 默认应异族");
}

#[test]
fn same_host_different_vendor_is_distinct() {
    let claude = ProviderFingerprint {
        role: "a".into(),
        base_url: "https://rsxermu666.cn".into(),
        model: "claude-opus-4-8".into(),
    };
    let gpt = ProviderFingerprint {
        role: "j".into(),
        base_url: "https://rsxermu666.cn/v1".into(),
        model: "gpt-5.4".into(),
    };
    assert_ne!(claude.family(), gpt.family(), "同 host 不同 vendor 应异族");
}

#[test]
fn exact_same_provider_is_same_family() {
    let a = ProviderFingerprint {
        role: "a".into(),
        base_url: "https://rsxermu666.cn".into(),
        model: "claude-opus-4-8".into(),
    };
    let b = ProviderFingerprint {
        role: "b".into(),
        base_url: "https://rsxermu666.cn".into(),
        model: "claude-opus-4-8".into(),
    };
    assert_eq!(a.family(), b.family(), "完全同 provider 应判同族（硬门据此 panic）");
}

#[test]
fn vendor_prefix_extraction() {
    let nvidia = ProviderFingerprint {
        role: "rp".into(),
        base_url: "https://integrate.api.nvidia.com/v1".into(),
        model: "meta/llama-3.3-70b-instruct".into(),
    };
    assert_eq!(nvidia.family(), "integrate.api.nvidia.com|meta", "厂商段取 / 前");

    let mimo = ProviderFingerprint {
        role: "x".into(),
        base_url: "https://api.xiaomimimo.com/v1".into(),
        model: "mimo-v2.5-pro".into(),
    };
    assert_eq!(mimo.family(), "api.xiaomimimo.com|mimo", "系列名取 - 前");
}
