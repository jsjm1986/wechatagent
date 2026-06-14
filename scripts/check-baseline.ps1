#!/usr/bin/env pwsh
# scripts/check-baseline.ps1
#
# CI baseline verification (Windows / PowerShell).
# 关联：requirements.md R11.6 — 升级合并前 CI 必须跑：
#   - `cargo test --lib`：总通过数 >= 350（knowledge cleanup 后基线），0 失败
#   - 4 个 PBT 文件累计通过数 >= 33（升级前基线 6+9+6+12），0 失败
#     (state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter)
# 任一不达标即 exit 1。
#
# step 2:cargo check --tests (RUSTFLAGS=-D warnings) —— 把 tests/ 目录纳入
# -D warnings 编译检查,堵住 cargo test --lib 不编译 tests/ 的 unused import 盲区。
#
# G-后续Ⅱ/4：可选 step 4 —— 当 $env:DOCKER_AVAILABLE = "1" 时跑无 LLM/MCP
# 的知识库集成测试 wiki_gap_signals_3kinds（3 个 #[ignore] 测试，纯 Mongo
# testcontainers 路径）。失败立刻退出 1。

$ErrorActionPreference = "Continue"

$LIB_BASELINE = 350
$PBT_BASELINE = 33

function Parse-CargoTestResults {
    param([string]$Output)

    $totalPassed = 0
    $totalFailed = 0

    # 匹配 cargo 输出中：
    # "test result: ok. 78 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in ..."
    $regex = 'test result:[^\r\n]*?(\d+)\s+passed;\s*(\d+)\s+failed'
    foreach ($m in [regex]::Matches($Output, $regex)) {
        $totalPassed += [int]$m.Groups[1].Value
        $totalFailed += [int]$m.Groups[2].Value
    }

    return @{ Passed = $totalPassed; Failed = $totalFailed }
}

function Invoke-Cargo {
    param([string[]]$ExtraArgs)

    # 把 stdout + stderr 都拿到（cargo test 失败时常 print 到 stderr）
    $output = & cargo test @ExtraArgs 2>&1 | Out-String
    Write-Host $output
    return $output
}

Write-Host "[baseline] step 1/3: cargo test --lib ..."
$libOut = Invoke-Cargo @('--lib')
$libRes = Parse-CargoTestResults $libOut
Write-Host ("[baseline] lib summary: passed={0} failed={1} (need >= {2} passed, 0 failed)" `
    -f $libRes.Passed, $libRes.Failed, $LIB_BASELINE)

if ($libRes.Failed -gt 0) {
    Write-Host "[baseline] FAIL: cargo test --lib has $($libRes.Failed) failed test(s)"
    exit 1
}
if ($libRes.Passed -lt $LIB_BASELINE) {
    Write-Host ("[baseline] FAIL: cargo test --lib only {0} passed (< baseline {1})" `
        -f $libRes.Passed, $LIB_BASELINE)
    exit 1
}

Write-Host ""
Write-Host "[baseline] step 2/3: cargo check --tests (RUSTFLAGS=-D warnings) ..."
# 把 tests/ 目录纳入 -D warnings 编译检查。只编译不运行,不增磁盘压力、不依赖 Docker。
# 目的:堵住 cargo test --lib 不编译 tests/ 导致的 unused import / warning 盲区
# (见 domain_profile_e2e.rs unused import 漏网事件)。
$env:RUSTFLAGS = "-D warnings"
& cargo check --tests --quiet
$checkExit = $LASTEXITCODE
Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue
if ($checkExit -ne 0) {
    Write-Host "[baseline] FAIL: cargo check --tests exited $checkExit (unused import / warning in tests/)"
    exit 1
}
Write-Host "[baseline] cargo check --tests OK"

Write-Host ""
Write-Host "[baseline] step 3/3: cargo test 4 PBT files ..."
$pbtOut = Invoke-Cargo @(
    '--test', 'state_transition_pbt',
    '--test', 'memory_card_invariants',
    '--test', 'wiki_chunk_revision_pbt',
    '--test', 'llm_retry_jitter'
)
$pbtRes = Parse-CargoTestResults $pbtOut
Write-Host ("[baseline] pbt summary: passed={0} failed={1} (need >= {2} passed, 0 failed)" `
    -f $pbtRes.Passed, $pbtRes.Failed, $PBT_BASELINE)

if ($pbtRes.Failed -gt 0) {
    Write-Host "[baseline] FAIL: PBT has $($pbtRes.Failed) failed test(s)"
    exit 1
}
if ($pbtRes.Passed -lt $PBT_BASELINE) {
    Write-Host ("[baseline] FAIL: PBT cumulative only {0} passed (< baseline {1})" `
        -f $pbtRes.Passed, $PBT_BASELINE)
    exit 1
}

# ── step 4 (可选)：DOCKER_AVAILABLE=1 时跑无 LLM/MCP 的知识库集成测试 ────
if ($env:DOCKER_AVAILABLE -eq "1") {
    $GAP_BASELINE = 3
    Write-Host ""
    Write-Host "[baseline] step 4 (DOCKER_AVAILABLE=1): cargo test --test wiki_gap_signals_3kinds -- --ignored ..."
    $gapOut = Invoke-Cargo @('--test', 'wiki_gap_signals_3kinds', '--', '--ignored')
    $gapRes = Parse-CargoTestResults $gapOut
    Write-Host ("[baseline] gap_signals summary: passed={0} failed={1} (need >= {2} passed, 0 failed)" `
        -f $gapRes.Passed, $gapRes.Failed, $GAP_BASELINE)
    if ($gapRes.Failed -gt 0) {
        Write-Host "[baseline] FAIL: wiki_gap_signals_3kinds has $($gapRes.Failed) failed test(s)"
        exit 1
    }
    if ($gapRes.Passed -lt $GAP_BASELINE) {
        Write-Host ("[baseline] FAIL: wiki_gap_signals_3kinds only {0} passed (< baseline {1})" `
            -f $gapRes.Passed, $GAP_BASELINE)
        exit 1
    }
    Write-Host ""
    Write-Host ("baseline OK: lib={0}, pbt={1}, gap_signals={2}" `
        -f $libRes.Passed, $pbtRes.Passed, $gapRes.Passed)
} else {
    Write-Host ""
    Write-Host "[baseline] step 4 skipped (DOCKER_AVAILABLE!=1)"
    Write-Host ("baseline OK: lib={0}, pbt={1}" -f $libRes.Passed, $pbtRes.Passed)
}
exit 0
