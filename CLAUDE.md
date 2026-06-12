# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Communication

Always reply to the user in Chinese (中文). This applies to all conversational responses, explanations, summaries, and status updates. Code, identifiers, commit messages, and file contents follow their existing conventions.

## Subagents

When spawning subagents (the Agent / Task tool), ALWAYS pass `model: "opus"`. Subagents must run on the same Opus-tier model as the main session — never let them fall back to a smaller/cheaper model. This applies to every agent type (Explore, general-purpose, etc.) without exception.

## Project

WechatAgent is a long-running WeChat private-domain operations AI agent system built as a single Rust (Axum) backend + React admin, talking to MongoDB, an external MCP server (for WeChat account/contact/send tooling), and a DeepSeek/OpenAI-compatible LLM.

Phase 1 scope is **user (private-chat) operations**. Group and Moments operations are planned separate operation domains; do not fold them into the user-ops code path. The product positioning is **fully AI-autonomous** — there is no "human takeover". Held/blocked sends use AI-internal status names (`held_by_ai_policy` / `blocked_by_safety_guard` / `ai_waiting_for_more_context`); admins observe these but the business semantics never become "human handoff".

**"无人工接管"的精确含义**：指客户永远只跟 AI 对话、永不直接面对真人。AI 在遇到超出自身职权/能力的事项时，向**幕后决策源（领导）**请示、拿回结论后用自己的口吻向客户转述——这不是人工接管（客户从不面对人、对话始终是 AI 在说）。详见决策请示通道设计 `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`。

## Common commands

The toolchain is `cargo` (Rust 2021) for the backend and `npm` + Vite for the frontend. There is no Cargo workspace and no top-level `Makefile`.

```sh
# Backend
cargo check
cargo run                              # serves API on $APP_PORT (default 8080) and hosts frontend/dist
cargo test --lib                       # unit tests (lib only — fast)
cargo test                             # unit + all integration tests under tests/
cargo test --test state_transition_pbt # run a single integration test file
cargo test some_test_name              # run a single test by name substring

# Frontend (admin UI in frontend/)
cd frontend && npm install
cd frontend && npm run dev             # vite dev server, proxies /api → http://localhost:8080
cd frontend && npm run build           # writes frontend/dist; cargo run will host it

# CI baseline gate (REQUIRED before merging) — see "Test baseline" below
scripts/check-baseline.ps1             # Windows / PowerShell
scripts/check-baseline.sh              # Linux / CI
```

Configuration is via `.env` (copy from `.env.example`). Required at startup: `MCP_API_KEY`, `OPENAI_API_KEY`. All other vars have defaults in `src/config.rs` (LLM retry/timeout, task worker interval, webhook rate limit, claim timeouts, etc.). The shell here is bash on Windows — use Unix paths and forward slashes; the project root contains non-ASCII characters (`工作项目`), so prefer absolute paths via the tooling rather than `cd`.

## Test baseline (do not regress)

`scripts/check-baseline.{sh,ps1}` is the merge gate, defined in `.kiro/specs/agent-autonomy-loop/requirements.md` R11.6. It enforces:

- `cargo test --lib`: **≥ 350 passed, 0 failed** (knowledge-cleanup 后基线，与 `scripts/check-baseline.{sh:25,ps1:17}` `LIB_BASELINE=350` 同步)
- Cumulative across these four PBT files: **≥ 33 passed, 0 failed**
  - `state_transition_pbt`, `memory_card_invariants`, `wiki_chunk_revision_pbt`, `llm_retry_jitter`

Either threshold failing or any failure → `exit 1`. New work should add tests, not lower these numbers. The `coreFacts` field must keep deserializing the legacy `Vec<String>` form for backward compat (R11).

A second merge gate is `scripts/check-no-human-takeover.{sh,ps1}` — a CI lint that scans `git diff` newly-added lines under `src/agent/`, `src/routes/`, `src/evolution/`, `frontend/src/` for forbidden words (`human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`). Tests directories are excluded. The lint enforces the AI-autonomous positioning at the literal string level — pick AI-internal status names and labels (e.g. "AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文"), never "人工接管 / takeover / hand-off".

Most integration tests under `tests/` are `#[ignore]` by default and require Docker (testcontainers MongoDB). `cargo test` will compile them but skip ignored tests; run explicitly via `cargo test --test <name> -- --ignored` when Docker is available.

**Local vs CI split (disk-space discipline).** The dev disk is small and compiling the 100+ integration test binaries (`pdf-extract` / `feed-rs` / `scraper` / `jsonwebtoken` pull a large `target/`) plus pulling the `mongo` image routinely fills it (`os error 112` / `no space left on device`). So locally run only the cheap, small-footprint suites: `cargo test --lib` and individual PBT files (`cargo test --test <name>`). Leave the full `--ignored` integration suite to GitHub CI — `.github/workflows/ci.yml`'s `integration` job frees ~30GB of pre-installed SDKs before building, which a local machine can't. Every push to `main` / PR runs both the baseline gate and integration job, so committed work is always exercised on CI. When the local disk does fill, delete `target/debug/incremental` first (regenerated automatically, several GB, no dependency rebuild) before any heavier cleanup.

## Architecture (big picture)

Single Rust process: hosts the admin SPA, exposes the JSON API under `/api`, receives WeChat callbacks at `POST /webhooks/wechat`, and runs the follow-up task worker in a background `tokio::spawn`.

```
React Admin (frontend/, served from frontend/dist)
  ↓
Rust Axum (src/main.rs → src/lib.rs)
  ├── routes/        REST API (split per resource; mounted in routes/mod.rs)
  │   └── chunk_locks.rs  WebSocket soft-lock + broadcast bus for collaborative chunk editing (P1-4)
  ├── webhooks.rs    POST /webhooks/wechat — parses payload, persists inbound, gates Agent
  ├── tasks.rs       follow-up task worker loop (interval = TASK_WORKER_INTERVAL_SECONDS)
  ├── agent/         user-ops Agent (decision → review → send) — see below
  ├── auth/          session cookie + Argon2; auth/jwt.rs = RS256 Bearer issue/verify (P1-7, gated by JWT_ENABLED)
  ├── knowledge_wiki/ knowledge subsystem; ingest_worker.rs = auto-ingest RSS/HTML loop (P1-6, gated by INGEST_WORKER_ENABLED)
  ├── prompts.rs     prompt pack v2 + ensure_prompt_pack_v2 (seeded at startup)
  ├── llm.rs         OpenAI-compatible client w/ retry/jitter, usage tracking, token-level streaming SSE (P1-3)
  ├── mcp.rs         MCP JSON-RPC client (account/contact/message_send_text)
  ├── db/            Mongo connect + ensure_indexes + migrations
  └── models.rs      All BSON-serde structs (very large; one file by convention)
```

Phase G P1 additions (multi-tenant workspace, graph community layout, token-level LLM streaming, WebSocket collab locks, multimodal PDF/vision import, auto-ingest worker, public JWT auth) are gated by env flags and default off where they touch deploy topology (`JWT_ENABLED`, `INGEST_WORKER_ENABLED`); see `.env.example`. The auth chain accepts a `wa_session` cookie by default and additionally `Authorization: Bearer <jwt>` only when `JWT_ENABLED=true` (`auth/middleware.rs`). Newly ingested/imported knowledge (PDF, image-vision, RSS/HTML) is always written `status=draft` + `integrity_status=needs_review` — the "AI never auto-verifies" red line holds across every ingestion entrypoint.

### `src/agent/` is the brain — read `src/agent/mod.rs` first

`src/agent.rs` was split (LP-11) into a module tree. Public entrypoints other code calls (`webhooks`, `tasks`, `routes::*`) are re-exported from `mod.rs`; do not bypass them.

| Submodule | Responsibility |
| --- | --- |
| `types` | internal contracts: `AgentDecision`, `DecisionReviewResult`, `KnowledgeRouteResult`, `AgentTrigger` |
| `runtime` | `UserRuntimeParameters` strongly-typed run params |
| `budget` | `RunBudget` task-local LLM token/call counter (MP-5) |
| `guards` | state-machine transitions, string-level fact-risk, knowledge-grounding checks |
| `memory` | long-term memoryCard consolidation (MP-8) |
| `reaction` | user-reaction analysis with claim lock (HP-3) |
| `knowledge_router` | catalog → search → open_slice tool-calling planner (MP-9) |
| `decision` | Reply Agent main decision + initial profile generation |
| `review` | Independent Review Agent + revision flow (MP-10) |
| `gateway` | unified send gateway, `run_user_operation_gateway`, `handle_managed_message`, `handle_follow_up_task` |
| `outbox` | persistent outbox with idempotency key, second-pass safety gate, retry |
| `simulation` | shadow-mode `simulate_user_dialogue` |
| `taxonomy` | dual-layer tagging (`system_taxonomies` + `taxonomy_candidates`) |
| `run_envelope` | single-run envelope/log shape |

Every send (webhook auto-reply AND follow-up tasks) flows through the **same gateway**: reload context → check `managed`/cooldown/min-interval/daily cap/expiry → Reply Agent → independent Review → optionally one revision → outbox → MCP `message_send_text`. Bypassing the gateway is a bug.

`generate_agent_json` in `agent/mod.rs` is the only LLM JSON entrypoint. It owns the LRU prompt cache, writes `llm_call_logs` rows (status: `success` / `cache_hit` / `failed` / `json_error`), and accumulates token usage into the run-local `RunBudget`. New prompts go through it.

### Webhook → Agent flow

```
POST /webhooks/wechat
  → parse appId / fromWxid / content / msgId
  → resolve account + contact, write inbound to conversation_messages
  → if contact.agent_status != "managed": stop here (only persist, don't reply)
  → run_user_operation_gateway(...)  // decision + review + send
  → write events / outcome metrics / decision review / run log
```

Only contacts with `agent_status = "managed"` get auto-replies. `normal` contacts are persisted only.

### MongoDB layer (`src/db/`)

`Database::connect` does **not** run migrations or create indexes — `main.rs` calls them in order: `migrations::run` first, `ensure_indexes` second (some migrations rebuild collections). Keep that order in any test setup. Typed `Collection<T>` accessors live on `Database` (e.g. `state.db.contacts()`, `state.db.agent_run_logs()`). Add new collections by both adding a typed accessor and an index entry.

## Hard rules baked into the code

These are enforced by `guards/`, `review/`, and the gateway. Removing any of them is almost certainly wrong — re-read `docs/agent-policy.md` and `.kiro/specs/agent-autonomy-loop/requirements.md` first.

- Auto-send is gated by methodology thresholds — current rules: `FactRisk ≥ 6` block, `PressureRisk ≥ 7` block, `HumanLikeScore < 6` rewrite once, `EmotionalValue < 5` rewrite once, `ProductAccuracyScore < 7` block product-claim sends.
- Product claims must be backed by **verified knowledge** in `operation_knowledge_chunks`; otherwise `blocked_unverified_product_claim`.
- `operation_state` is **derived from the normalized `customer_stage`** at the gateway write site (C2 — same canonical id space, m006), so the two fields never drift; it falls back to the decision's own `operation_state` only when no `customer_stage` is present. The synced value goes through `check_state_transition` against the state-machine dictionary (`operation_domain_configs`). This is **fail-soft**: an illegal transition does NOT block the reply (already sent) — it skips the `operation_state` write (keeps the old state) and emits an `agent.operation_state_transition_rejected` audit event. Agents do not invent new state keys. The engine reads `initial` / `allowFromAny` / `allowedFrom` flags from the state machine, so it is industry-agnostic (DEFAULT sales profile marks only `new_contact` as `initial`).
- **Dual-layer tagging**: `customer_stage` / `intent_level` / `objection_type` must come from `system_taxonomies`. Free-form ideas go to `agent_generated_signals` and `taxonomy_candidates` for admin review; unreviewed candidates **must not** block runs.
- Each run has a token/call budget (`RunBudget`). Exceeding it returns `AppError::BudgetExceeded` and the gateway falls back (e.g. `local_decision_review`, skip rewrite). Don't surface this as a 5xx to webhook callers.
- Gateway/finalReview status enums are closed sets (R9.10.e in the autonomy-loop spec). Writing an unknown status must be rejected at the DB write site, not silently coerced.
- Outbox + idempotency: a decision that's `approved` MUST hit `agent_send_outbox` with an idempotency key before the MCP call. User rejection / cooldown cancels pending outbox entries.

## Prompt + knowledge conventions

Prompts are layered (Soul → System Contract → Policy → Business Context → Operator Instruction) and versioned in `prompt_templates` / `agent_souls` / `operation_playbooks`. Run logs record `promptVersions`. `prompts::ensure_prompt_pack_v2` seeds the v2 default pack at startup. The `reset-system-pack` route physically deletes and re-seeds — it is an explicit maintenance action, **not** an idempotent every-startup overwrite (would clobber operator edits).

Knowledge is progressive-disclosure: catalog → list_chunks → open_slice via tool-calling, not a single "stuff everything in the prompt" call. The router lives in `agent/knowledge_router.rs`.

## Specs and roadmap

- Active specs: `.kiro/specs/agent-autonomy-loop/` (current wave) and `.kiro/specs/user-ops-agent-hardening/`. Each has `requirements.md`, `design.md`, `tasks.md`. Read the requirements before changing behavior the spec covers.
- Product/architecture docs live in `docs/`; `docs/README.md` lists the reading order. New top-level product modules update `docs/product-modules.md` first; new automation behaviors update `docs/agent-policy.md` first; new backend capabilities update `docs/architecture.md` and `docs/data-and-api.md` first.

## Frontend notes

The admin is a single Vite + React 19 + TypeScript app (no router lib — channel/tab state is in `App.tsx`). New pages/components must follow `docs/frontend-design-system.md` (enterprise white-channel layout). The dev server proxies `/api` to `:8080`; in production `cargo run` serves `frontend/dist` via `ServeDir` with SPA fallback to `index.html`.
