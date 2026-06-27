//! 提示词自然语言编辑的三层分级 + 双闸校验（spec §4.4）。
//!
//! 实现已下沉到中立顶层模块 `crate::prompt_guard`（供人工编辑路径与
//! evolution release 路径共用）。本文件只 re-export prompt_templates.rs
//! 实际使用的符号，保持调用路径 `crate::routes::management_prompt_edit::{...}` 不破。

pub(super) use crate::prompt_guard::{review_prompt_edit, validate_prompt_edit, PromptEditVerdict};
