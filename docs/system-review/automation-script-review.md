# Automation and Operational Script Review

This batch covers all 65 baseline files classified as `automation_scripts`.
Every Git blob from baseline commit
`12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` was read to EOF. The machine
evidence in `automation-read-evidence.json` records path, baseline blob id,
ledger byte count, physical and non-empty line counts, content hash, first and
last non-empty lines, and coarse side-effect tags.

Set reconciliation is exact: 65 evidence rows, 65 ledger rows, no missing or
duplicate paths, no blob mismatch, 351,762 bytes, and 7,938 physical lines.

## Test-result semantics

- `scripts/biz-test/run_all.py` does not propagate a preflight failure as a
  non-zero process exit and ignores each domain script return code. Domain
  assertions generally append findings through `_lib.expect` instead of
  failing the process. A successful process exit is not a suite verdict.
- `_lib.assert_llm_success` accepts `cache_hit` as proof of a successful model
  call. That can prove an equivalent request succeeded previously, but not
  that the current run called the external model.
- Several scripts treat missing model artifacts or proposals as observations
  and continue. They remain diagnostic probes unless a typed outcome gate
  checks required cases, calls, artifacts, assertions, and final verdicts.
- Campaign, Guide, Memory, Relationship, Knowledge, and escalation scripts
  often query or mutate records without complete workspace/account identity.
  Some fixtures manufacture default-scope data to bypass a real deployment
  mismatch. Such runs cannot prove multi-tenant production behavior.

## Operational and data safety

- The set contains 43 scripts with write operations, 20 with delete
  operations, eight with remote execution, and 37 with model/provider
  interaction. They are operational programs, not harmless documentation.
- SSH helpers read passwords from environment variables but accept first-use
  host keys through `AutoAddPolicy`. `_push_bundle.py` also interpolates remote
  paths into shell commands without shell-safe quoting. These helpers are not
  a trusted deployment boundary without host-key pinning and literal remote
  path handling.
- `clean_knowledge_legacy.py` defaults to dry-run, but apply mode deletes
  documents, packs, and chunks selected by broad schema-shape predicates.
  Other one-off cleanup scripts omit full tenant scope in places. Production
  use requires a frozen manifest, exact target verification, backup, and
  post-read validation.
- `llm_providers_e2e.sh` activates a synthetic provider and can leave it active
  when no alternate provider exists. It is a mutation probe, not a safe
  unattended E2E test.

## Evidence boundary

No script in this batch was executed, no server was contacted, and no database
was changed. This batch establishes complete frozen-source review and records
evidence and safety limits. Existing findings for credentials, false-green
tests, tenant isolation, deployment safety, and reproducible audits remain the
owners; this review creates no duplicate SR number and does not claim current
production code is unfixed.
