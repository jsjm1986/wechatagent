//! `import_pdf_smoke` —— P1-5 multimodal PDF 导入端到端冒烟。
//!
//! 守的红线：
//!   1. PDF 字节 → `import_pdf_bytes` → `ingest_chunked_text` 至少落 1 条 chunk；
//!   2. "AI 永不自动 verify"：导入的 chunk 一律 `status="draft"` +
//!      `integrity_status="needs_review"`（无论走 fence 还是 fallback blob）。
//!
//! 两个场景：
//!   - **fence 命中**：PDF 文本含 `---CHUNK/---END---` → 走 block 解析，产 ≥1 chunk。
//!   - **无 fence**：纯文本 PDF → fallback blob chunk（仍 draft + needs_review）。
//!
//! 不依赖外部 fixture 文件：用 [`minimal_pdf`] 在运行时拼一个合法单页 PDF（自带
//! 正确 xref 偏移），避免提交易碎的二进制。`#[ignore]` 守门：依赖 testcontainers
//! MongoDB，CI 用 `cargo test --test import_pdf_smoke -- --ignored`（需 Docker）。

mod common;

use wechatagent::routes::ext_knowledge::import_pdf_bytes;

use crate::common::TestApp;

/// 拼一个合法的单页 PDF，正文为 `text`（会被 PDF 字符串括号转义）。
/// 手算 xref 偏移以保证 lopdf / pdf-extract 能解析。
fn minimal_pdf(text: &str) -> Vec<u8> {
    // PDF 字符串里 ( ) \ 需转义；换行用真实换行不行（content stream 内），
    // 这里把每行拆成独立的 Tj，行间用 Td 下移，规避括号内换行问题。
    let lines: Vec<&str> = text.split('\n').collect();
    let mut content = String::from("BT /F1 12 Tf 72 720 Td 14 TL\n");
    for (i, line) in lines.iter().enumerate() {
        let escaped = line
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        if i == 0 {
            content.push_str(&format!("({escaped}) Tj\n"));
        } else {
            // T* 换行 + 显示
            content.push_str(&format!("T* ({escaped}) Tj\n"));
        }
    }
    content.push_str("ET");
    let content_bytes = content.as_bytes();

    let mut buf: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();

    buf.extend_from_slice(b"%PDF-1.4\n");
    // 二进制注释行，提示这是二进制文件。
    buf.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");

    let mut push_obj = |buf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(buf.len());
        buf.extend_from_slice(body);
    };

    push_obj(
        &mut buf,
        &mut offsets,
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    );
    push_obj(
        &mut buf,
        &mut offsets,
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
    );
    push_obj(
        &mut buf,
        &mut offsets,
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n",
    );
    // obj 4：内容流。
    {
        offsets.push(buf.len());
        let header = format!("4 0 obj\n<< /Length {} >>\nstream\n", content_bytes.len());
        buf.extend_from_slice(header.as_bytes());
        buf.extend_from_slice(content_bytes);
        buf.extend_from_slice(b"\nendstream\nendobj\n");
    }
    push_obj(
        &mut buf,
        &mut offsets,
        b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    );

    // xref
    let xref_start = buf.len();
    let obj_count = offsets.len() + 1; // +1 for free object 0
    buf.extend_from_slice(format!("xref\n0 {obj_count}\n").as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        buf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    // trailer
    buf.extend_from_slice(
        format!(
            "trailer\n<< /Size {obj_count} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF\n"
        )
        .as_bytes(),
    );
    buf
}

/// 场景 1：PDF 文本含 fence → 走 block 解析，产 ≥1 chunk，全部 draft + needs_review。
#[tokio::test]
#[ignore]
async fn import_pdf_with_fence_produces_review_chunks() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let fence_text = "---CHUNK: pdf-c1---\n# 价格异议处理\n先共情再说价值最后给方案。\n---END---";
    let pdf = minimal_pdf(fence_text);

    let outcome = import_pdf_bytes(&app.state, &ws, None, "smoke_pdf", pdf)
        .await
        .expect("import_pdf_bytes ok");

    assert!(
        !outcome.chunk_ids.is_empty(),
        "PDF 含 fence 应至少产 1 chunk: {outcome:?}",
    );

    assert_all_chunks_draft_needs_review(&app, &ws, &outcome.chunk_ids).await;
}

/// 场景 2：纯文本 PDF（无 fence）→ fallback blob chunk，仍 draft + needs_review。
#[tokio::test]
#[ignore]
async fn import_pdf_without_fence_falls_back_to_blob() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let plain = "这是一段没有任何 fence 标记的普通 PDF 正文。应当落一个待切分 blob chunk。";
    let pdf = minimal_pdf(plain);

    let outcome = import_pdf_bytes(&app.state, &ws, None, "smoke_pdf_plain", pdf)
        .await
        .expect("import_pdf_bytes ok");

    assert!(outcome.fallback_blob, "无 fence 应触发 fallback blob: {outcome:?}");
    assert_eq!(
        outcome.chunk_ids.len(),
        1,
        "fallback blob 应恰好 1 chunk: {outcome:?}",
    );
    assert_all_chunks_draft_needs_review(&app, &ws, &outcome.chunk_ids).await;
}

/// 场景 3：空 PDF（抽取后无文本）→ BadRequest，不写任何 chunk。
#[tokio::test]
#[ignore]
async fn import_empty_pdf_rejected() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();

    let pdf = minimal_pdf("");
    let result = import_pdf_bytes(&app.state, &ws, None, "smoke_pdf_empty", pdf).await;
    assert!(result.is_err(), "空 PDF 应被拒绝（抽取文本为空）");
}

async fn assert_all_chunks_draft_needs_review(app: &TestApp, ws: &str, chunk_ids: &[String]) {
    use mongodb::bson::{doc, oid::ObjectId};
    for id_hex in chunk_ids {
        let chunk = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(
                doc! {
                    "_id": ObjectId::parse_str(id_hex).unwrap(),
                    "workspace_id": ws,
                },
                None,
            )
            .await
            .unwrap()
            .expect("imported chunk should exist");
        assert_eq!(
            chunk.status, "draft",
            "导入 chunk 必须 draft（AI 永不自动 verify）: id={id_hex}",
        );
        assert_eq!(
            chunk.integrity_status.as_deref(),
            Some("needs_review"),
            "导入 chunk 必须 needs_review: id={id_hex}",
        );
    }
}
