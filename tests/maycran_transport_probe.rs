//! Temporary diagnostic: exercise Maycran through the production `LlmClient`.
//!
//! The test is ignored by default and is invoked only by the temporary probe
//! workflow. It performs no database or MCP writes and never prints the key.

use wechatagent::llm::{LlmClient, LlmProvider};

#[tokio::test]
#[ignore]
async fn production_client_reaches_candidate_models() {
    let Ok(api_key) = std::env::var("MAYCRAN_API_KEY") else {
        eprintln!("MAYCRAN_API_KEY is not configured; skipping temporary external probe");
        return;
    };
    let base_url = std::env::var("MAYCRAN_BASE_URL")
        .unwrap_or_else(|_| "https://api.maycran.com/v1".to_string());
    let candidates = [
        "claude-sonnet-4-6",
        "claude-sonnet-4.6",
        "claude-sonnet-4-5",
        "claude-opus-4-6",
        "qwen3-coder-next",
        "deepseek-v4-pro",
    ];

    let mut successes = Vec::new();
    for model in candidates {
        let client = LlmClient::new(
            base_url.clone(),
            api_key.clone(),
            model.to_string(),
            60,
            1,
            100,
        )
        .expect("build production LlmClient");
        match client
            .generate_json(
                "Return one strict JSON object and no surrounding text.",
                r#"Return {"ok":true}."#,
            )
            .await
        {
            Ok(value) if value.get("ok").and_then(|v| v.as_bool()) == Some(true) => {
                println!("RUST_CLIENT_PROBE model={model} result=ok");
                successes.push(model);
            }
            Ok(value) => {
                println!("RUST_CLIENT_PROBE model={model} result=invalid_json value={value}");
            }
            Err(error) => {
                println!("RUST_CLIENT_PROBE model={model} result=error detail={error}");
            }
        }
    }

    assert!(
        !successes.is_empty(),
        "production LlmClient could not reach any Maycran candidate model"
    );
    println!("RUST_CLIENT_USABLE_MODELS {}", successes.join(" "));
}

#[tokio::test]
#[ignore]
async fn production_client_handles_business_prompt_sizes() {
    let Ok(api_key) = std::env::var("MAYCRAN_API_KEY") else {
        eprintln!("MAYCRAN_API_KEY is not configured; skipping temporary external probe");
        return;
    };
    let base_url = std::env::var("MAYCRAN_BASE_URL")
        .unwrap_or_else(|_| "https://api.maycran.com/v1".to_string());
    let system = "你是业务配置生成器。只输出严格 JSON 对象，禁止解释和代码围栏。";
    let schema = r#"请按如下结构输出：{"ok":true,"summary":"简述","dimensions":[{"kind":"stage","displayName":"阶段","description":"说明"}],"policy":"方法论"}。"#;

    let mut successes = 0usize;
    for model in ["claude-sonnet-4-6", "deepseek-v4-pro"] {
        for size in [1_000usize, 4_000, 8_000, 16_000] {
            let filler = "客户处境、诉求、边界与沟通原则。".repeat(size / 16 + 1);
            let business_text: String = filler.chars().take(size).collect();
            let user = format!("业务资料：{business_text}\n{schema}");
            let client = LlmClient::new(
                base_url.clone(),
                api_key.clone(),
                model.to_string(),
                120,
                1,
                100,
            )
            .expect("build production LlmClient");
            match client.generate_json(system, &user).await {
                Ok(value) => {
                    println!(
                        "RUST_SIZE_PROBE model={model} chars={size} result=ok object={}",
                        value.is_object()
                    );
                    successes += 1;
                }
                Err(error) => {
                    println!(
                        "RUST_SIZE_PROBE model={model} chars={size} result=error detail={error}"
                    );
                }
            }
        }
    }

    assert!(successes > 0, "all production-size Maycran probes failed");
}
