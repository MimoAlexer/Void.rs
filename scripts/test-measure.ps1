#requires -Version 7.2
# Analyzer contract tests. All numeric fixtures below are synthetic test data,
# never rendering benchmarks or release evidence. No client/server is launched.
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testRoot = Join-Path $projectRoot ('.local/measure-script-test-' + [Guid]::NewGuid().ToString('N'))
[System.IO.Directory]::CreateDirectory($testRoot) | Out-Null
$measure = Join-Path $PSScriptRoot 'measure.ps1'
function Assert-That([bool] $Condition, [string] $Message) { if (-not $Condition) { throw $Message } }
function Write-Fixture([string] $Path, [object[]] $Samples) {
    $data = @{ schema_version = 1; kind = 'actual_client_frames'; scenario = 'synthetic_analyzer_unit_test'; mode = 'Baseline'; gpu = 'no GPU used'; samples = $Samples }
    [System.IO.File]::WriteAllText($Path, (ConvertTo-Json $data -Depth 5))
}
$samples = @(for ($i = 0; $i -lt 150; $i++) {
    [pscustomobject]@{ frame_us = 1000000.0; cpu_us = 200.0; gpu_us = 100.0; fence_wait_us = 0.0; input_to_submit_us = $(if ($i -eq 20) { 9000.0 } elseif ($i -eq 50) { 5000.0 } else { $null }); chunks = 9; vertices = 30000 }
})
$fixture = Join-Path $testRoot 'samples.json'
$output = Join-Path $testRoot 'analysis.json'
Write-Fixture $fixture $samples
& $measure -Metrics $fixture -WarmupSeconds 15 -DurationSeconds 120 -Output $output
$report = Get-Content -LiteralPath $output -Raw | ConvertFrom-Json
$run = $report.runs[0]
Assert-That ($run.measured_frames -eq 120 -and $run.measured_seconds -eq 120 -and $run.duration_requirement_met) 'Measured window must use 120 seconds after 15 seconds of warm-up.'
Assert-That ($run.discarded_warmup_frames -eq 15 -and $run.average_fps -eq 1.0) 'FPS must derive from actual retained interval sum.'
Assert-That ($run.input_sample_count -eq 2 -and $run.p95_input_to_submit_us -eq 9000) 'Missing input must not dilute the latency percentile.'
Assert-That (-not $run.acceptance_eligible -and $report.release_acceptance -eq 'not_evaluable') 'Raw frame logs must never independently pass release acceptance.'

$short = Join-Path $testRoot 'short.json'
Write-Fixture $short $samples[0..9]
& $measure -Metrics $short -WarmupSeconds 0 -DurationSeconds 120 -Output $output
$run = (Get-Content -LiteralPath $output -Raw | ConvertFrom-Json).runs[0]
Assert-That (-not $run.duration_requirement_met -and $run.measured_seconds -eq 10) 'Short smoke runs must not be accepted as timed trials.'
Assert-That ($null -eq $run.p95_input_to_submit_us -and $run.input_sample_count -eq 0) 'Absent input needs a null percentile.'

& $measure -Metrics $short -WarmupSeconds 15 -DurationSeconds 120 -Output $output
$run = (Get-Content -LiteralPath $output -Raw | ConvertFrom-Json).runs[0]
Assert-That ($run.measured_frames -eq 0 -and $null -eq $run.average_fps) 'Logs shorter than warm-up must produce empty measurements, not invented FPS.'

$invalid = Join-Path $testRoot 'invalid.json'
Write-Fixture $invalid @([pscustomobject]@{ frame_us = -1 })
$rejected = $false
try { & $measure -Metrics $invalid -WarmupSeconds 0 -Output $output } catch { $rejected = $true }
Assert-That $rejected 'Invalid frame intervals must be rejected.'

$planDirectory = Join-Path $testRoot 'plan with spaces'
& $measure -Prepare -ServerAddress 'localhost:25565' -RunDirectory $planDirectory
$plan = Get-Content -LiteralPath (Join-Path $planDirectory 'run-plan.json') -Raw | ConvertFrom-Json
Assert-That ($plan.trial_count -eq 18 -and $plan.kind -eq 'interactive_trial_plan_not_executed') 'Plan must prepare three repeats per three modes in two pacing conditions.'
$normalized = @($plan.trials | ForEach-Object {
    $config = Get-Content -LiteralPath $_.config -Raw
    Assert-That ($config -match '(?m)^width = 1920$' -and $config -match '(?m)^height = 1080$') 'Trial resolution must match.'
    $config -replace '(?m)^target_fps = \d+\r?$', 'target_fps = RATE' -replace '(?m)^mode = "[a-z_]+"\r?$', 'mode = "MODE"'
})
Assert-That (@($normalized | Select-Object -Unique).Count -eq 1) 'Only scheduler mode and pacing rate may differ between prepared configs.'
Assert-That (@($plan.trials | Where-Object { $_.target_fps -eq 240 }).Count -eq 9) 'Nine paced and nine uncapped trials are required.'
$tokens = $null; $errors = $null
[System.Management.Automation.Language.Parser]::ParseFile((Join-Path $planDirectory 'commands.ps1'), [ref] $tokens, [ref] $errors) | Out-Null
Assert-That ($errors.Count -eq 0) 'Generated launch commands must parse, including paths with spaces.'
Write-Host 'measure.ps1 contract tests passed. No game or benchmark was run.'
