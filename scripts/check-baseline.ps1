#!/usr/bin/env pwsh
# scripts/check-baseline.ps1
#
# CI baseline verification (Windows / PowerShell).
# 关联：requirements.md R11.6 — 升级合并前 CI 必须跑：
#   - `cargo test --lib`：总通过数 >= 78（升级前基线），0 失败
#   - 4 个 PBT 文件累计通过数 >= 33（升级前基线 6+9+6+12），0 失败
#     (state_transition_pbt / memory_card_invariants / string_fact_risk_guard / llm_retry_jitter)
# 任一不达标即 exit 1。

$ErrorActionPreference = "Continue"

$LIB_BASELINE = 78
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

Write-Host "[baseline] step 1/2: cargo test --lib ..."
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
Write-Host "[baseline] step 2/2: cargo test 4 PBT files ..."
$pbtOut = Invoke-Cargo @(
    '--test', 'state_transition_pbt',
    '--test', 'memory_card_invariants',
    '--test', 'string_fact_risk_guard',
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

Write-Host ""
Write-Host ("baseline OK: lib={0}, pbt={1}" -f $libRes.Passed, $pbtRes.Passed)
exit 0
