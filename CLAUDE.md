# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 红线中的红线（最高优先级 · 凌驾本文件其它一切要求）

**改代码、加功能、定 bug 之前，必须 100% 读懂相关代码与业务逻辑——绝不在任何猜测、假设、"大概是这样"或理解有缺口的前提下动手写/改任何代码。** 这是本项目最核心的红线，违反即视为严重失误，没有任何例外。

- **先读懂，再动手。** 任何修改（哪怕一行）落地前，必须把受影响的代码路径、调用链、数据流、相关业务规则、相关 spec/文档全部读完并真正理解。说不清它怎么跑、为什么这么写、改了会牵连什么——就继续读；读不出来就问用户，绝不"边写边猜"或"先改了再看"。
- **引用必亲验。** 任何 `file:line` 引用、任何"某函数/字段/标志位/行为是这样"的断言，都必须当场用 Read/Grep 亲自确认，不靠记忆、不靠印象、不靠本文件或 memory 的旧描述（它们可能已过期）。
- **猜测是禁区。** 看不懂的代码不要绕过去假装懂；模糊的需求不要自行脑补一版就开干。宁可多读十分钟、多问一句，也绝不在理解有缺口时写下任何改动。token 充足，任何为搞懂而做的查询都是值得的。
- **subagent 同此红线。** 派给 subagent 的实现/修复任务，指令里必须要求它先 100% 读懂相关代码再改，且必须核验它确实读过（看它给出的 file:line 证据），绝不接受基于猜测的产出。
- **与流程的关系：** `Superpowers` 是任务流程的入口（先调 skill），本红线是其中"写下任何改动"这一步雷打不动的前置硬门——两者层级不同、不冲突，brainstorming / 查证 / systematic-debugging 的内核本就是"先 100% 读懂再行动"。

## Superpowers（最优先）

**最优先使用 superpowers skills。** 任何任务开始前，只要有 ≥1% 可能某个 skill 适用，就必须先通过 Skill 工具调用它，再做其它动作（包括澄清提问、探索代码）。流程类 skill 优先于实现类：要写代码/做非平凡实现的任务，先走 `superpowers:brainstorming`（设计→获批→`writing-plans`）；调试先走 systematic-debugging。skill 一旦适用即非可选。唯一例外：用户的显式指令优先级最高（与 skill 冲突时听用户的）。

## Communication

Always reply to the user in Chinese (中文). This applies to all conversational responses, explanations, summaries, and status updates. Code, identifiers, commit messages, and file contents follow their existing conventions.

## Subagents

When spawning subagents (the Agent / Task tool), ALWAYS pass `model: "opus"`. Subagents must run on the same Opus-tier model as the main session — never let them fall back to a smaller/cheaper model. This applies to every agent type (Explore, general-purpose, etc.) without exception.

## Project

WechatAgent is a long-running WeChat private-domain operations AI agent system built as a single Rust (Axum) backend + React admin, talking to MongoDB, an external MCP server (for WeChat account/contact/send tooling), and a DeepSeek/OpenAI-compatible LLM.

Phase 1 scope is **user (private-chat) operations**. Group and Moments operations are planned separate operation domains; do not fold them into the user-ops code path. The product positioning is **fully AI-autonomous** — there is no "human takeover". Held/blocked sends use AI-internal status names (`held_by_ai_policy` / `blocked_by_safety_guard` / `ai_waiting_for_more_context`); admins observe these but the business semantics never become "human handoff".

**"无人工接管"的精确含义**：指客户永远只跟 AI 对话、永不直接面对真人。AI 在遇到超出自身职权/能力的事项时，向**幕后决策源（领导）**请示、拿回结论后用自己的口吻向客户转述——这不是人工接管（客户从不面对人、对话始终是 AI 在说）。详见决策请示通道设计 `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`。

**辅助模式（账号级可选，默认关）的受控例外**：当账号显式开启「辅助模式」且 AI 判定客户契合人类预先标注的引荐条件（如明确要签约/到店参观/需深入对接）时，AI 会主动把真人专属顾问的微信名片推送给客户，由客户与顾问对接完成临门一脚，此时 AI 退为辅助答疑角色。这是管理员显式配置的业务动作、AI 仍是发起方与辅助方（对话始终是 AI 在说，名片是 AI 主动引荐的"发送物"），不改变全自治模式（默认）下"客户永远只跟 AI 对话"的红线——后者一字不动。被引荐的台前顾问 ≠ 幕后决策源（领导），两者解耦。详见 `docs/superpowers/specs/2026-06-21-referral-card-push-design.md`。

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

Configuration is via `.env` (copy from `.env.example`). Required at startup: `MCP_API_KEY`, `OPENAI_API_KEY`. All other vars have defaults in `src/config.rs` (LLM retry/timeout, task worker interval, webhook rate limit, claim timeouts, etc.). 开发环境以会话运行时信息为准（2026-08-13 时点为 macOS + zsh），不要假设 Windows；项目根目录含非 ASCII 字符（`开发项目`），优先用工具的绝对路径而非 `cd`。

## Test baseline (do not regress)

`scripts/check-baseline.{sh,ps1}` is the merge gate, defined in `.kiro/specs/agent-autonomy-loop/requirements.md` R11.6. It enforces:

- `cargo test --lib`: **≥ 350 passed, 0 failed**（门槛值，与 `scripts/check-baseline.{sh,ps1}` 中的 `LIB_BASELINE=350` 同步——脚本行号会漂移，以变量名为准）。当前实际通过数约 **2562**（2026-08-13 优化波次后），远高于门槛；新工作只加不减。
- Cumulative across these four PBT files: **≥ 33 passed, 0 failed**（当前实际约 41）
  - `state_transition_pbt`, `memory_card_invariants`, `wiki_chunk_revision_pbt`, `llm_retry_jitter`

Either threshold failing or any failure → `exit 1`. New work should add tests, not lower these numbers. The `coreFacts` field must keep deserializing the legacy `Vec<String>` form for backward compat (R11).

A second merge gate is `scripts/check-no-human-takeover.{sh,ps1}` — a CI lint that scans `git diff` newly-added lines under `src/agent/`, `src/routes/`, `src/evolution/`, `frontend/src/` for forbidden words (`human[_ -]?takeover|takeover|hand[_ -]?off|人工接管|人工介入|人工托管|接管|人工`). Tests directories are excluded. The lint enforces the AI-autonomous positioning at the literal string level — pick AI-internal status names and labels (e.g. "AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文"), never "人工接管 / takeover / hand-off".

Most integration tests under `tests/` are `#[ignore]` by default and require Docker (testcontainers MongoDB). `cargo test` will compile them but skip ignored tests; run explicitly via `cargo test --test <name> -- --ignored` when Docker is available.

**Local vs CI split (disk-space discipline).** The dev disk is small and compiling the 100+ integration test binaries (`pdf-extract` / `feed-rs` / `scraper` / `jsonwebtoken` pull a large `target/`) plus pulling the `mongo` image routinely fills it (`os error 112` / `no space left on device`). So locally run only the cheap, small-footprint suites: `cargo test --lib` and individual PBT files (`cargo test --test <name>`). Leave the full `--ignored` integration suite to GitHub CI — `.github/workflows/ci.yml`'s `integration` job frees ~30GB of pre-installed SDKs before building, which a local machine can't. Every push to `main` / PR runs both the baseline gate and integration job, so committed work is always exercised on CI. When the local disk does fill, delete `target/debug/incremental` first (regenerated automatically, several GB, no dependency rebuild) before any heavier cleanup.

## Project-understanding 档案与金标回归（事实性导航）

- `project-understanding/` 存有 2026-08-13 全仓深读档案：`29-doc-code-divergence-master.md`（71 条"文档声称 X、代码实际 Y"偏差权威底单，含按误导源文件的反向索引）、`30-global-fact-cards.md`（闭集/阈值/幂等键/worker/prompt key 单一速查）、`28-crosscheck-tests-vs-prod.md`（无测试守护清单）及 19 份逐行深读记录；根目录 `PROJECT_UNDERSTANDING_LEDGER.md` 为总台账（含各优化波次追记，是新事实的权威）。台账内"改动前使用顺序"节给出查阅顺序：30 号速查 → 29 号反向索引 → 28 号 §4 → 对应深读记录 → 终裁记录 → 任何 file:line 动手前当场重验。
- 金标回归环：`bash scripts/quality-regression.sh` 一键对 105 条合成场景（`tests/fixtures/quality_gold/`，五类 × 21）做 shadow 回归（`tests/quality_gold_regression.rs`，零真实发送；红线硬门 + judge 软门）；CI nightly `quality-gold` job 以软门运行。

## Architecture (big picture)

Single Rust process: hosts the admin SPA, exposes the JSON API under `/api`, receives WeChat callbacks at `POST /webhooks/wechat`, and runs **16 个 supervised 后台 worker**（名单见 `src/supervisor.rs` `SUPERVISED_WORKERS`：task worker、inbound reply worker、outbox dispatcher、post-decision 投影等；其中 6 个由默认关闭的 env flag 控制——strategic_planner / cold_contact / silence_signal / evolutionary / knowledge_digest / ingest）.

```
React Admin (frontend/, served from frontend/dist)
  ↓
Rust Axum (src/main.rs → src/lib.rs)
  ├── routes/        REST API (split per resource; mounted in routes/mod.rs)
  │   └── chunk_locks.rs  WebSocket soft-lock + broadcast bus for collaborative chunk editing (P1-4)
  ├── webhooks.rs    POST /webhooks/wechat — parses payload, persists inbound, gates Agent
  ├── tasks.rs       follow-up task worker loop (interval = TASK_WORKER_INTERVAL_SECONDS)
  ├── supervisor.rs  16 个后台 worker 的熔断监督（SUPERVISED_WORKERS；panic 退避 + 熔断 open/half_open）
  ├── agent/         user-ops Agent (decision → review → send) — see below
  ├── evolution/     自演化子系统（EVOLUTION_ENABLED 默认 false；与发送链物理隔离，release 恒人工）
  ├── auth/          session cookie + Argon2; auth/jwt.rs = RS256 Bearer issue/verify (P1-7, gated by JWT_ENABLED)
  ├── knowledge_wiki/ knowledge subsystem; ingest_worker.rs = auto-ingest RSS/HTML loop (P1-6, gated by INGEST_WORKER_ENABLED)
  ├── prompts.rs     prompt pack v2 + ensure_prompt_pack_v2 (seeded at startup)
  ├── llm.rs         OpenAI-compatible client w/ retry/jitter, usage tracking, token-level streaming SSE (P1-3)
  ├── mcp.rs         MCP JSON-RPC 通用工具客户端（发送类工具调用只允许出现在 outbox dispatcher——CI delivery-protocol job 锁定）
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
| `guards` | state-machine transition legality + state-action policy（字符串级 fact-risk / knowledge 守卫已于 2026-05-25 删除，见 `guards.rs` 头注；评分闸在 `review/gates.rs`） |
| `memory` | long-term memoryCard consolidation (MP-8) |
| `reaction` | user-reaction analysis with claim lock (HP-3) |
| `knowledge_router` | gateway 决策前的知识预路由（多轮 catalog → search → open_slice 循环在路由内部执行，MP-9） |
| `decision` | Reply Agent main decision + initial profile generation |
| `review` | Independent Review Agent + revision flow (MP-10) |
| `gateway` | unified send gateway, `run_user_operation_gateway`, `handle_managed_message`, `handle_follow_up_task` |
| `outbox` | persistent outbox with idempotency key, second-pass safety gate, retry |
| `simulation` | shadow-mode `simulate_user_dialogue` |
| `taxonomy` | dual-layer tagging (`system_taxonomies` + `taxonomy_candidates`) |
| `run_envelope` | single-run envelope/log shape |

表为节选，完整子模块清单以 `src/agent/mod.rs` 为准——另有 `outbox_dispatcher`（发送状态机+二次安全门）、`escalation/`（幕后请示通道）、`quiet_hours`、`pacing`（长度加权发送间隔）、`post_decision`（发送后投影）、`knowledge_agent`、`chat_tool_loop` 等。

Every send (webhook auto-reply AND follow-up tasks) flows through the **same gateway**: reload context → check `managed`/cooldown/min-interval/daily cap/expiry → Reply Agent → independent Review → optionally one revision → outbox → MCP `message_send_text`. Bypassing the gateway is a bug.

`generate_agent_json` in `agent/mod.rs` is the only LLM JSON entrypoint. It owns the LRU prompt cache, writes `llm_call_logs` rows (status: `success` / `cache_hit` / `failed` / `json_error`), and accumulates token usage into the run-local `RunBudget`. New prompts go through it.

### Webhook → Agent flow

```
POST /webhooks/wechat
  → parse appId / fromWxid / content / msgId
  → resolve account + contact, write inbound to conversation_messages
  → if contact.agent_status != "managed": stop here (only persist, don't reply)
  → materialize durable `inbound_reply` task（静默时段 defer 到 next_wake_at；显式交易意向可豁免
    defer——quiet_hours.rs `bypass_deferral_for_explicit_buying_intent`）
  → task worker 认领任务（非静默时 webhook 另 spawn 低延迟唤醒）
      → run_user_operation_gateway(...)  // decision + review + send
  → write events / outcome metrics / decision review / run log
```

注意：webhook handler 本身**不同步执行 Agent**——它只落库并物化 durable 任务，决策执行在 task worker 层（崩溃后任务可恢复）。Only contacts with `agent_status = "managed"` get auto-replies. `normal` contacts are persisted only.

### MongoDB layer (`src/db/`)

`Database::connect` does **not** run migrations or create indexes — `main.rs` calls them in order: `migrations::run` first, `ensure_indexes` second (some migrations rebuild collections). Keep that order in any test setup. Typed `Collection<T>` accessors live on `Database` (e.g. `state.db.contacts()`, `state.db.agent_run_logs()`). Add new collections by both adding a typed accessor and an index entry.

## Hard rules baked into the code

These are enforced by `guards/`, `review/`, and the gateway. Removing any of them is almost certainly wrong — re-read `docs/agent-policy.md` and `.kiro/specs/agent-autonomy-loop/requirements.md` first.

- Auto-send is gated by the Review **评分闸体系**（2026-05-25 起替代字符串守卫；判定在 `src/agent/review/gates.rs`，阈值默认在 `RuntimeParametersTyped` 且 runtime 可覆盖）：`hallucinationScore`（wire alias `factRisk`）**≥ 6 硬拦**、`knowledgeGroundingScore`（alias `productAccuracy`）**< 7 硬拦**（情感域可条件旁路，DEFAULT 恒判）；`pressureRisk ≥ 7`、`humanLike < 6`、`emotionalValue < 6`、`boundaryPrivacySafety ≤ 3` 为**软闸**，合并触发一次 single-shot revision（pressure 是软闸、生产不产 block 终态）。wire 别名映射见 `src/agent/runtime.rs`：`fact_risk_block_at` 实际承载 hallucination 阈值、`product_accuracy_block_below` 实际承载 knowledge_grounding 阈值——按字面名调参会调错。
- Product claims 走 R5.4 **三路背书取或**：verified knowledge（`operation_knowledge_chunks`）∪ 产品目录结构化定价（priced_from_catalog）∪ 领导授权豁免（principal_product_exempted）；三者皆空才 `blocked_unverified_product_claim`。
- `operation_state` is **derived from the normalized `customer_stage`** at the gateway write site (C2 — same canonical id space, m006), so the two fields never drift; it falls back to the decision's own `operation_state` only when no `customer_stage` is present. The synced value goes through `check_state_transition` against the state-machine dictionary (`operation_domain_configs`). This is **fail-soft**: an illegal transition does NOT block the reply (already sent) — it skips the `operation_state` write (keeps the old state) and emits an `agent.operation_state_transition_rejected` audit event. Agents do not invent new state keys. The engine reads `initial` / `allowFromAny` / `allowedFrom` flags from the state machine, so it is industry-agnostic (DEFAULT sales profile marks only `new_contact` as `initial`).
- **Dual-layer tagging**: `customer_stage` / `intent_level` / `objection_type` must come from `system_taxonomies`. Free-form ideas go to `agent_generated_signals` and `taxonomy_candidates` for admin review; unreviewed candidates **must not** block runs.
- Each run has a token/call budget (`RunBudget`). Exceeding it returns `AppError::BudgetExceeded` and the gateway falls back (e.g. `local_decision_review`, skip rewrite). Don't surface this as a 5xx to webhook callers.
- Gateway/finalReview status enums are closed sets (R9.10.e in the autonomy-loop spec). Writing an unknown status must be rejected at the DB write site, not silently coerced.
- Outbox + idempotency: a decision that's `approved` MUST hit `agent_send_outbox` with an idempotency key before the MCP call. User rejection / cooldown cancels pending outbox entries.

## Prompt + knowledge conventions

Prompts are layered (Soul → System Contract → Policy → Business Context → Operator Instruction) and versioned in `prompt_templates` / `agent_souls` / `operation_playbooks`. Run logs record `promptVersions`. `prompts::ensure_prompt_pack_v2` seeds the v2 default pack at startup. The `reset-system-pack` route physically deletes and re-seeds — it is an explicit maintenance action, **not** an idempotent every-startup overwrite (would clobber operator edits).

Knowledge is progressive-disclosure（catalog → search → open_slice），但注意归属：user-ops 主决策是**单发**调用、不带工具中间轮（LLM 误吐 `tool_calling` 相位会被 gateway 强制转 final 并记 degraded）；知识由 gateway 在决策**前**经 `agent/knowledge_router.rs` 预路由，多轮工具循环发生在路由内部（`knowledge_agent`）。catalog → search → open_slice 的 tool-calling 循环形态服务于管理台 chat（`agent/chat_tool_loop.rs`，永不写库、不进 outbox）。

## Specs and roadmap

- `.kiro/specs/` 三大 spec（agent-autonomy-loop / user-ops-agent-hardening / agent-self-evolution）**均已 sunset**（各文件头部有 notice；notice 自身"3 闸 enforce_*"的描述是中间态，现行闸门以 `src/agent/review/gates.rs` 分数闸为准）。任务完成状态的唯一权威是 `.kiro/specs/task-status-manifest.json`——历史 markdown `[x]` 勾选不可信。读任何 spec 前先对照 `project-understanding/29-doc-code-divergence-master.md` 的反向索引防误导。
- Product/architecture docs live in `docs/`; `docs/README.md` lists the reading order. New top-level product modules update `docs/product-modules.md` first; new automation behaviors update `docs/agent-policy.md` first; new backend capabilities update `docs/architecture.md` and `docs/data-and-api.md` first.

## Frontend notes

The admin is a single Vite + React 19 + TypeScript app (no router lib — channel/tab state is in `App.tsx`). New pages/components must follow `docs/frontend-design-system.md` (enterprise white-channel layout). The dev server proxies `/api` to `:8080`; in production `cargo run` serves `frontend/dist` via `ServeDir` with SPA fallback to `index.html`.
