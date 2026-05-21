"""Split src/routes.rs into submodules under src/routes/.

Reads src/routes.rs and emits files based on a manifest mapping items to modules.
Items are identified by name; their full source (including doc comments and attributes
preceding the item) is extracted by line range computed via brace counting.
"""
import os
import re

SRC = 'src/routes.rs'
DEST_DIR = 'src/routes'

# Manifest: module_name -> list of item names (in desired order)
MANIFEST = {
    'health': [
        'health',
    ],
    'accounts': [
        'UpdateAccountMcpKeyRequest',
        'list_accounts',
        'sync_accounts',
        'update_account_mcp_key',
    ],
    'contacts': [
        'OperationProfileRequest',
        'OperatingMemoryRequest',
        'MemoryCandidateQuery',
        'list_contacts',
        'search_import_contacts',
        'get_contact',
        'enable_agent',
        'disable_agent',
        'update_profile_note',
        'update_operation_profile',
        'analyze_contact_profile',
        'get_operating_memory',
        'update_operating_memory',
        'get_contact_memory_card',
        'list_contact_memory_candidates',
        'run_contact_memory_consolidation',
        'get_operation_health',
    ],
    'guides': [
        'GuidePreviewRequest',
        'GuideApplyRequest',
        'preview_user_operation_guide',
        'apply_user_operation_guide',
    ],
    'simulations': [
        'UserDialogueSimulationRequest',
        'UserOperationEvaluationRequest',
        'simulate_user_operation_dialogue',
        'run_user_operation_evaluation',
        'judge_user_operation_scenario',
        'doc_i32_opt',
    ],
    'conversations': [
        'list_messages',
    ],
    'events': [
        'list_events',
    ],
    'tasks': [
        'AgentRunQuery',
        'LlmUsageQuery',
        'list_tasks',
        'list_agent_runs',
        'list_llm_usage',
        'review_task_now',
        'cancel_agent_task',
    ],
    'reviews': [
        'DecisionReviewQuery',
        'list_decision_reviews',
        'get_decision_review',
    ],
    'outcome_metrics': [
        'OutcomeMetricsQuery',
        'list_agent_outcome_metrics',
        'outcome_metric_json',
    ],
    'evaluations': [
        'EvaluationScenarioRequest',
        'EvaluationScenarioQuery',
        'FormulaAdherenceRequest',
        'list_evaluation_scenarios',
        'create_evaluation_scenario',
        'update_evaluation_scenario',
        'delete_evaluation_scenario',
        'run_formula_adherence_evaluation',
        'evaluation_scenario_json',
        'score_key_for',
        'bson_to_f64',
    ],
    'assets': [
        'ContentAssetQuery',
        'ContentAssetRequest',
        'list_content_assets',
        'create_content_asset',
    ],
    'knowledge': [
        'OperationKnowledgeQuery',
        'OperationKnowledgeDocumentQuery',
        'OperationKnowledgeChunkQuery',
        'OperationKnowledgeDocumentRequest',
        'OperationKnowledgeRequest',
        'OperationKnowledgeChunkRequest',
        'OperationKnowledgeImportRequest',
        'OperationKnowledgeImportApplyRequest',
        'KnowledgeToolSearchRequest',
        'KnowledgeToolOpenRequest',
        'KnowledgeVerifyRequest',
        'KnowledgeAutoVerifyRequest',
        'OperationKnowledgeTestRequest',
        'list_operation_knowledge',
        'list_operation_knowledge_documents',
        'create_operation_knowledge_document',
        'get_operation_knowledge_document',
        'update_operation_knowledge_document',
        'delete_operation_knowledge_document',
        'list_operation_knowledge_chunks',
        'list_operation_knowledge_document_chunks',
        'create_operation_knowledge_chunk',
        'update_operation_knowledge_chunk',
        'delete_operation_knowledge_chunk',
        'get_operation_knowledge_chunk_source',
        'verify_operation_knowledge_chunk',
        'reject_operation_knowledge_chunk',
        'auto_verify_operation_knowledge_chunks',
        'get_operation_knowledge_catalog',
        'get_operation_knowledge_completeness',
        'refresh_operation_knowledge_completeness',
        'get_operation_knowledge_integrity_report',
        'search_operation_knowledge_tool',
        'open_operation_knowledge_slices',
        'create_operation_knowledge',
        'update_operation_knowledge',
        'delete_operation_knowledge',
        'import_operation_knowledge_preview',
        'import_operation_knowledge_apply',
        'test_operation_knowledge_match',
        'list_knowledge_usage',
        'operation_knowledge_json',
        'operation_knowledge_document_json',
        'operation_knowledge_chunk_json',
        'knowledge_usage_json',
        'validate_operation_knowledge',
        'validate_operation_knowledge_document',
        'validate_operation_knowledge_chunk',
        'operation_knowledge_from_request',
        'operation_knowledge_document_from_request',
        'operation_knowledge_chunk_from_request',
        'normalize_operation_knowledge_preview_item',
        'normalize_operation_knowledge_preview_document',
        'default_operation_knowledge_preview_document',
        'normalize_operation_knowledge_preview_chunk',
        'json_string_list',
        'split_lines',
        'string_bson_array',
        'stable_text_hash',
        'build_line_index',
        'build_section_index',
        'source_anchor_for_quote',
        'integrity_report_for_preview',
        'apply_chunk_integrity',
        'load_operation_knowledge_chunks_for_query',
        'build_operation_knowledge_catalog',
        'build_operation_knowledge_completeness',
        'default_user_operations_domain',
        'default_mixed_business_type',
        'default_manual_source_type',
        'default_imported_markdown_source_type',
        'default_active_status',
    ],
    'souls': [
        'AgentSoulRequest',
        'list_agent_souls',
        'create_agent_soul',
        'update_agent_soul',
        'publish_agent_soul',
        'ensure_default_souls',
    ],
    'domains': [
        'OperationDomainRequest',
        'list_operation_domains',
        'get_operation_domain',
        'update_operation_domain',
        'get_operation_domain_state_machine',
        'update_operation_domain_state_machine',
        'reset_operation_domain',
        'operation_domain_json',
        'validate_operation_domain_input',
        'ensure_operation_domains',
        'find_operation_domain',
    ],
    'prompt_templates': [
        'PromptTemplateQuery',
        'PromptTemplateRequest',
        'list_prompt_templates',
        'create_prompt_template',
        'update_prompt_template',
        'publish_prompt_template',
        'reset_system_prompt_pack',
        'prompt_template_json',
        'validate_prompt_template_input',
    ],
    'playbooks': [
        'OperationPlaybookQuery',
        'OperationPlaybookRequest',
        'GeneratePlaybookRequest',
        'OptimizePlaybookRequest',
        'list_operation_playbooks',
        'create_operation_playbook',
        'update_operation_playbook',
        'set_default_operation_playbook',
        'generate_operation_playbook',
        'optimize_operation_playbook',
        'playbook_json',
        'validate_playbook_input',
        'ensure_default_playbook',
        'unset_default_playbooks',
        'build_playbook_generation_prompt',
        'build_playbook_optimization_prompt',
    ],
    'management': [
        'CreateSessionRequest',
        'ManagementMessageRequest',
        'ManagementPlan',
        'PlannedToolCall',
        'create_management_session',
        'post_management_message',
        'get_management_command',
        'get_tool_catalog',
        'merge_product_tools',
        'apply_locked_send_content',
        'extract_locked_send_content',
        'extract_quoted_text',
        'trim_wrapping_quotes',
        'is_read_tool',
        'execute_management_tool',
        'string_arg',
        'optional_value_arg',
        'resolve_contact_arg',
        'management_context',
        'build_management_plan',
    ],
    'shared': [
        'AccountScopedQuery',
        'parse_object_id',
        'validate_account',
        'find_contact_by_id',
        'upsert_contact_from_value',
        'ensure_operating_memory',
        'latest_decision_review',
        'resolve_playbook_for_contact',
        'operation_health_json',
        'health_item',
        'health_scores_document',
        'score_presence',
        'apply_contact_changes',
        'apply_memory_changes',
        'apply_playbook_changes',
        'apply_domain_changes',
        'build_guide_preview_prompt',
        'playbook_brief',
        'guide_preview_json',
        'operating_memory_json',
        'effective_route_memory_card',
        'memory_candidate_json',
        'llm_call_log_json',
        'decision_review_json',
        'agent_run_json',
        'normalize_optional',
        'json_string_any',
        'json_document_any',
        'json_string_vec_any',
        'json_string',
        'doc_get_string',
        'doc_get_document',
        'doc_get_string_vec',
        'doc_string_ref',
        'doc_list_text',
        'merge_document',
    ],
}

DOC_COMMENTS = {
    'health': '//! 健康检查路由：返回服务状态及基础元数据，供前端 / 监控探活使用。',
    'accounts': '//! 微信账号路由：管理 `WechatAccount` 记录及 MCP key 同步。',
    'contacts': '//! 联系人路由：联系人画像、操作记忆、运营状态等用户级别接口。',
    'guides': '//! 用户运营引导路由：自然语言指令转配置预览与确认应用。',
    'simulations': '//! 用户运营模拟路由：影子对话和场景化评估。',
    'conversations': '//! 会话消息路由：根据联系人查询历史对话。',
    'events': '//! Agent 事件流路由：审计与运营追踪用。',
    'tasks': '//! Agent 任务路由：跟进任务、Run 日志、LLM 用量等运行时观测。',
    'reviews': '//! 决策复盘路由：列出 / 查询 Agent 决策审阅记录。',
    'outcome_metrics': '//! Agent 成效指标路由：聚合性指标暴露。',
    'evaluations': '//! 评估场景路由：场景增删改查与公式遵从度评估。',
    'assets': '//! 内容资产路由：私域素材库的列表与新增。',
    'knowledge': '//! 运营知识库路由：文档 / 切片 / 条目的全生命周期管理。',
    'souls': '//! Agent 灵魂提示路由：管理各 Agent 的人格 prompt。',
    'domains': '//! 运营领域配置路由：领域目标、方法论与状态机。',
    'prompt_templates': '//! Prompt 模板路由：分层 prompt 的发布与回滚。',
    'playbooks': '//! 运营 Playbook 路由：方法论模板的增删改查及自动生成。',
    'management': '//! 管理 Agent 路由：管理对话 session、计划生成与工具执行。',
    'shared': '//! 跨模块共享辅助：ObjectId 解析、联系人加载、JSON 序列化等。',
}

# Read source
with open(SRC, encoding='utf-8') as f:
    raw_lines = f.readlines()

n = len(raw_lines)


def find_item_block(name: str):
    """Find the line range (1-indexed inclusive) for the named item, including
    any leading doc comments and attributes."""
    # Find start line containing the item declaration
    pat = re.compile(rf'^(pub )?(async )?(fn|struct|enum) {re.escape(name)}\b')
    decl_line = None
    for i, line in enumerate(raw_lines):
        if pat.match(line):
            decl_line = i
            break
    if decl_line is None:
        raise SystemExit(f'item not found: {name}')

    # Walk back to include preceding doc comments / attributes
    start = decl_line
    while start > 0:
        prev = raw_lines[start - 1].rstrip('\n')
        stripped = prev.lstrip()
        if stripped.startswith('//') or stripped.startswith('#['):
            start -= 1
            continue
        # Multi-line attribute
        if prev.strip() == ')]' or prev.strip() == ']':
            # Walk back through bracket
            start -= 1
            continue
        break

    # Find end of item by brace counting
    # Locate first '{' in declaration
    open_line = decl_line
    while open_line < n and '{' not in raw_lines[open_line]:
        open_line += 1
    if open_line >= n:
        # single-line item (e.g., empty decl) — unlikely
        return start, decl_line

    bd = 0
    end = open_line
    found = False
    for j in range(open_line, n):
        for ch in raw_lines[j]:
            if ch == '{':
                bd += 1
            elif ch == '}':
                bd -= 1
                if bd == 0:
                    end = j
                    found = True
                    break
        if found:
            break
    return start, end


# Common header used by domain submodules
COMMON_HEADER = '''use axum::{
    extract::{Path, Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, to_bson, to_document, Bson, DateTime, Document, Regex},
    options::{FindOneOptions, FindOptions, UpdateOptions},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use std::sync::Arc;

use crate::{
    agent,
    config::AppConfig,
    db::Database,
    error::{AppError, AppResult},
    llm::LlmGenerator,
    mcp::{self, McpClient},
    models::{
        AgentCommandRun, AgentDecisionReview, AgentOutcomeMetric, AgentRunLog, AgentSoul,
        AgentToolCall, ApiContact, Contact, ContactQuery, ContentAsset, EnableAgentRequest,
        EvaluationScenario, KnowledgeUsageLog, LlmCallLog, ManagementAgentMessage,
        ManagementAgentSession, MemoryCandidate, OperatingMemory, OperationDomainConfig,
        OperationKnowledgeChunk, OperationKnowledgeDocument, OperationKnowledgeItem,
        OperationPlaybook, ProfileNoteRequest, PromptTemplate, SearchImportRequest,
        UserOperationGuidePreview, WechatAccount,
    },
    prompts,
};

use super::shared::*;
use super::AppState;
'''


# Lookup for items
items_meta = {}
for module, names in MANIFEST.items():
    for name in names:
        if name in items_meta:
            raise SystemExit(f'Duplicate manifest entry: {name}')
        items_meta[name] = module

# Compute line ranges (1-indexed)
item_blocks = {}
for name in items_meta:
    start, end = find_item_block(name)
    item_blocks[name] = (start, end)


def make_pub_super(text: str) -> str:
    """Convert top-level visibility to pub(super)."""
    # Convert each top-level item to pub(super) so siblings can use them.
    out_lines = []
    for line in text.splitlines(keepends=True):
        # Find lines starting with 'fn ', 'struct ', 'enum ', 'async fn ', 'pub fn ',
        # 'pub async fn ', 'pub struct ', 'pub enum '
        m = re.match(r'^(pub(\([^)]*\))? )?(async )?(fn|struct|enum) ', line)
        if m and not line.startswith(' '):
            # Replace prefix
            kind_start = m.end(0) - len(m.group(4)) - 1  # before kind keyword
            # We just rewrite from start
            # Strip existing pub(...)? prefix
            rest = line[m.end(1) if m.group(1) else 0:]
            out_lines.append('pub(super) ' + rest)
        else:
            out_lines.append(line)
    return ''.join(out_lines)


# Build module file contents
module_contents = {m: [] for m in MANIFEST}

for module, names in MANIFEST.items():
    parts = []
    for name in names:
        s, e = item_blocks[name]
        block = ''.join(raw_lines[s:e + 1])
        block = make_pub_super(block)
        parts.append(block)
    module_contents[module] = '\n'.join(parts)


os.makedirs(DEST_DIR, exist_ok=True)


def write_module(name, body):
    path = os.path.join(DEST_DIR, f'{name}.rs')
    header = DOC_COMMENTS.get(name, f'//! {name} 路由模块。')
    full = header + '\n\n' + COMMON_HEADER + '\n' + body + '\n'
    with open(path, 'w', encoding='utf-8') as f:
        f.write(full)
    print(f'wrote {path}')


for module in MANIFEST:
    if module == 'shared':
        continue
    write_module(module, module_contents[module])

# shared.rs has slightly different header (no super::shared::*)
SHARED_HEADER = '''use axum::extract::State;
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, to_bson, to_document, Bson, DateTime, Document},
    options::{FindOneOptions, FindOptions, UpdateOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    error::{AppError, AppResult},
    models::{
        AgentDecisionReview, AgentRunLog, ApiContact, Contact, LlmCallLog, MemoryCandidate,
        OperatingMemory, OperationPlaybook, UserOperationGuidePreview,
    },
};

use super::AppState;
'''

shared_path = os.path.join(DEST_DIR, 'shared.rs')
with open(shared_path, 'w', encoding='utf-8') as f:
    f.write(DOC_COMMENTS['shared'] + '\n\n' + SHARED_HEADER + '\n' + module_contents['shared'] + '\n')
print(f'wrote {shared_path}')

# Now write mod.rs containing AppState and api_router
# Read the AppState struct and api_router fn
appstate_s, appstate_e = find_item_block('AppState')
api_router_s, api_router_e = find_item_block('api_router')

appstate_block = ''.join(raw_lines[appstate_s:appstate_e + 1])
api_router_block = ''.join(raw_lines[api_router_s:api_router_e + 1])

mod_header = '''//! Routes 模块入口：组装 `AppState` 与 `api_router`，并通过 `pub use` 暴露子模块。
//!
//! 业务路由按职责切分到子模块；本入口只负责拼装 axum Router、共享状态和外部
//! 依赖（main.rs / agent.rs / mcp.rs / tasks.rs / webhooks.rs / 集成测试）需要
//! 看到的最小公开 API。

use axum::{
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;

use crate::{
    config::AppConfig,
    db::Database,
    llm::LlmGenerator,
    mcp::McpClient,
};

mod accounts;
mod assets;
mod contacts;
mod conversations;
mod domains;
mod evaluations;
mod events;
mod guides;
mod health;
mod knowledge;
mod management;
mod outcome_metrics;
mod playbooks;
mod prompt_templates;
mod reviews;
mod shared;
mod simulations;
mod souls;
mod tasks;

pub use shared::upsert_contact_from_value;

'''

# Use the imports in api_router from each module
# api_router uses many handler functions; we need to make them accessible via module paths
# Replace each handler call with module::handler form
# For mechanical safety, just bring everything into scope via use statements

# Build the use-imports for handlers
HANDLER_TO_MODULE = {}
for module, names in MANIFEST.items():
    if module == 'shared':
        continue
    for name in names:
        # Only handlers (functions used by api_router) need to be re-imported.
        # We'll generate use ... for all fns; structs aren't called.
        pass

# Build a list of all fn handler names referenced in api_router
api_router_text = ''.join(raw_lines[api_router_s:api_router_e + 1])
# extract identifiers used as routes (get(name), post(name), etc.)
handler_ids = set()
for m in re.finditer(r'(?:get|post|put|delete)\(\s*(\w+)\s*\)', api_router_text):
    handler_ids.add(m.group(1))
print('handlers used in api_router:', sorted(handler_ids))

# Map handler -> module
handler_module = {}
for module, names in MANIFEST.items():
    for name in names:
        if name in handler_ids:
            handler_module[name] = module

# Build use imports
use_lines = []
by_module = {}
for h, m in handler_module.items():
    by_module.setdefault(m, []).append(h)
for m in sorted(by_module):
    hs = sorted(by_module[m])
    if len(hs) == 1:
        use_lines.append(f'use {m}::{hs[0]};')
    else:
        use_lines.append(f'use {m}::{{{", ".join(hs)}}};')
use_block = '\n'.join(use_lines)

mod_full = (
    mod_header
    + use_block
    + '\n\n'
    + appstate_block
    + '\n'
    + api_router_block
    + '\n'
)

with open(os.path.join(DEST_DIR, 'mod.rs'), 'w', encoding='utf-8') as f:
    f.write(mod_full)
print(f'wrote {os.path.join(DEST_DIR, "mod.rs")}')
