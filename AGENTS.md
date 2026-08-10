# Repository Guidelines

## Project Structure & Module Organization

WechatAgent is a Rust 2021 service with a React 19 admin UI. Backend code lives in `src/`: HTTP handlers are under `routes/`, agent behavior under `agent/`, database setup and migrations under `db/`, and evolution logic under `evolution/`. Rust integration, property-based, and end-to-end tests live in `tests/`. The Vite/TypeScript frontend is in `frontend/src/`, organized into `features/`, shared `components/`, `contracts/`, `stores/`, and `__tests__/`. Guidance is in `docs/`; automation lives in `scripts/` and `.github/workflows/`.

## Build, Test, and Development Commands

- `cargo run`: start the Axum API on `APP_PORT` (default `8080`) and serve `frontend/dist`.
- `cargo check`: type-check the backend.
- `cargo test --lib`: run the fast Rust unit-test suite.
- `cargo test --test state_transition_pbt`: run one integration/PBT target.
- `scripts/check-baseline.sh`: run the required Linux/CI baseline gate.
- `cd frontend && npm install && npm run dev`: start Vite on port `5173`, proxying `/api` to `8080`.
- `cd frontend && npm run build`: run TypeScript checks and create the production bundle.
- `cd frontend && npm test`: run Vitest once.

## Coding Style & Naming Conventions

Run `cargo fmt --all -- --check` before submitting Rust changes; use four-space formatting, `snake_case` modules/functions, and `PascalCase` types. TypeScript is strict and uses two-space indentation. Name React components/types in `PascalCase`, hooks/functions in `camelCase`, and feature directories in kebab-case. Follow `docs/frontend-design-system.md` for UI changes. Keep migrations ordered (`m057_description.rs`) and register them in `src/db/migrations/mod.rs`.

## Testing Guidelines

Place Rust unit tests beside their modules and cross-module tests in `tests/`, using `_integration.rs`, `_pbt.rs`, or `_e2e.rs` suffixes. Frontend tests belong under `frontend/src/__tests__/` and use `*.test.ts(x)`. The merge baseline requires at least 350 passing library tests and 33 cumulative PBT tests. Most ignored integration tests require Docker/testcontainers MongoDB; run one with `cargo test --test <target> -- --ignored`, leaving the full suite to CI when disk is constrained.

## Commit & Pull Request Guidelines

Use the repository’s scoped Conventional Commit style: `fix(knowledge): repair source filters`, `feat(taxonomy): ...`, or `chore(ci): ...`. Keep each commit focused. PRs should explain behavior and risk, link the issue/spec, list commands run, and include screenshots for visible UI changes. Confirm baseline and relevant frontend checks before requesting review.

## Security & Configuration

Copy `.env.example` to `.env`; never commit credentials. `MCP_API_KEY` and `OPENAI_API_KEY` are required at startup. Preserve workspace scoping, fail-closed authorization, and durable outbox safety checks when changing routes or agent delivery behavior.
