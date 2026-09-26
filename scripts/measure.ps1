#requires -Version 7.2
<#
.SYNOPSIS
Analyze actual Void.rs frame logs, or prepare a repeatable three-mode trial plan.
.DESCRIPTION
This script never treats a frame count as a duration. -Prepare writes identical
trial configurations and interactive launch commands; it does not run or time
the client. Analysis validates the actual duration in saved frame intervals.
No report from this script can pass the release gate: the current client logs
do not attest visual correctness or record nearby terrain presentation age.
.EXAMPLE
pwsh -File scripts/measure.ps1 -Prepare -ServerAddress localhost:25565
.EXAMPLE
pwsh -File scripts/measure.ps1 -Metrics .local/bench/baseline.json -WarmupSeconds 15
#>
[CmdletBinding(DefaultParameterSetName = 'Analyze')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Analyze')]
    [string[]] $Metrics,
    [Parameter(Mandatory, ParameterSetName = 'Prepare')]
    [switch] $Prepare,
    [Parameter(Mandatory, ParameterSetName = 'Prepare')]
    [ValidateNotNullOrEmpty()]
    [string] $ServerAddress,
    [Parameter(ParameterSetName = 'Prepare')]
    [string] $ClientPath,
    [Parameter(ParameterSetName = 'Prepare')]
    [string] $ConfigPath,
    [Parameter(ParameterSetName = 'Prepare')]
    [ValidatePattern('^[A-Za-z0-9_]{1,16}$')]
    [string] $Username = 'VoidBenchmark',
    [Parameter(ParameterSetName = 'Prepare')]
    [string] $RunDirectory,
    [Parameter(ParameterSetName = 'Analyze')]
    [string] $Output,
    [ValidateRange(0, 3600)]
    [double] $WarmupSeconds = 15,
    [ValidateRange(1, 3600)]
    [double] $DurationSeconds = 120
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'

function Get-Field {
    param([object] $Object, [string] $Name)
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Get-Percentile {
    param([double[]] $Values, [double] $Percent)
    if ($null -eq $Values -or $Values.Length -eq 0) { return $null }
    $sorted = [double[]] $Values.Clone()
    [Array]::Sort($sorted)
    $index = [Math]::Max(0, [int][Math]::Ceiling($sorted.Length * $Percent / 100.0) - 1)
    return $sorted[$index]
}

function Write-Json {
    param([string] $Path, [object] $Value)
    $absolute = [System.IO.Path]::GetFullPath($Path)
    $parent = [System.IO.Path]::GetDirectoryName($absolute)
    [System.IO.Directory]::CreateDirectory($parent) | Out-Null
    $json = ConvertTo-Json -InputObject $Value -Depth 12
    [System.IO.File]::WriteAllText($absolute, $json, [System.Text.UTF8Encoding]::new($false))
}

function Quote-PowerShell {
    param([string] $Value)
    return "'" + $Value.Replace("'", "''") + "'"
}

function Set-TomlScalar {
    param([string] $Text, [string] $Section, [string] $Key, [string] $Literal)
    $current = ''
    $matchesFound = 0
    $lines = foreach ($line in [regex]::Split($Text, '\r?\n')) {
        if ($line -match '^\s*\[([^\]]+)\]\s*(?:#.*)?$') { $current = $Matches[1] }
        if ($current -eq $Section -and $line -match ('^\s*' + [regex]::Escape($Key) + '\s*=')) {
            $matchesFound++
            "$Key = $Literal"
        } else { $line }
    }
    if ($matchesFound -ne 1) { throw "Config needs exactly one [$Section].$Key setting; found $matchesFound." }
    return $lines -join [Environment]::NewLine
}

if ($Prepare) {
    if ([string]::IsNullOrWhiteSpace($ClientPath)) { $ClientPath = Join-Path $projectRoot 'target/release/void-client.exe' }
    if ([string]::IsNullOrWhiteSpace($ConfigPath)) { $ConfigPath = Join-Path $projectRoot 'config/default.toml' }
    if ([string]::IsNullOrWhiteSpace($RunDirectory)) { $RunDirectory = Join-Path $projectRoot ".local/measure-$stamp" }
    $ClientPath = [System.IO.Path]::GetFullPath($ClientPath)
    $ConfigPath = [System.IO.Path]::GetFullPath($ConfigPath)
    $RunDirectory = [System.IO.Path]::GetFullPath($RunDirectory)
    if (Test-Path -LiteralPath $RunDirectory) { throw "Run directory already exists; choose a new directory to preserve prior evidence: $RunDirectory" }
    $base = [System.IO.File]::ReadAllText($ConfigPath)
    $base = Set-TomlScalar $base 'graphics' 'width' '1920'
    $base = Set-TomlScalar $base 'graphics' 'height' '1080'
    $base = Set-TomlScalar $base 'graphics' 'render_distance' '12'
    $base = Set-TomlScalar $base 'graphics' 'vsync' 'false'
    [System.IO.Directory]::CreateDirectory($RunDirectory) | Out-Null
    $modes = @('baseline', 'frame_coordination', 'event_horizon')
    $trials = [System.Collections.Generic.List[object]]::new()
    $commands = [System.Collections.Generic.List[string]]::new()
    $commands.Add('# Generated interactive trial plan. This file has no automatic duration control.')
    $commands.Add('# Close each client only after the same scene is ready, warm-up is finished,')
    $commands.Add("# and at least $DurationSeconds seconds of the fixed workload have elapsed.")
    $commands.Add('# Use an external stopwatch. A frame-count smoke run is not a timed benchmark.')
    $commands.Add('# No server is created; no EULA or server configuration is changed.')
    $commands.Add("if (-not (Test-Path -LiteralPath $(Quote-PowerShell $ClientPath))) { throw 'Build the release client first: cargo build --release -p void-client' }")
    foreach ($rate in @(240, 0)) {
        $condition = if ($rate -eq 0) { 'uncapped' } else { 'paced240' }
        for ($trial = 1; $trial -le 3; $trial++) {
            # Rotate mode order to reduce always-running-the-baseline-first bias.
            for ($offset = 0; $offset -lt 3; $offset++) {
                $mode = $modes[($offset + $trial - 1) % 3]
                $id = "$condition-$mode-$trial"
                $config = Set-TomlScalar $base 'performance' 'target_fps' ([string] $rate)
                $config = Set-TomlScalar $config 'performance.event_horizon' 'mode' ('"' + $mode + '"')
                $configFile = Join-Path $RunDirectory "$id.toml"
                $metricsFile = Join-Path $RunDirectory "$id.json"
                [System.IO.File]::WriteAllText($configFile, $config, [System.Text.UTF8Encoding]::new($false))
                $arguments = @('--config', $configFile, '--metrics', $metricsFile, '--connect', $ServerAddress, '--username', $Username)
                $trials.Add([ordered]@{ id = $id; mode = $mode; condition = $condition; repetition = $trial; target_fps = $rate; config = $configFile; metrics = $metricsFile; executable = $ClientPath; arguments = $arguments })
                $commands.Add('')
                $commands.Add("# $id : same recorded workload; warm-up >= $WarmupSeconds s, measured interval >= $DurationSeconds s.")
                $commands.Add('& ' + (Quote-PowerShell $ClientPath) + ' ' + (($arguments | ForEach-Object { Quote-PowerShell $_ }) -join ' '))
            }
        }
    }
    $commandsFile = Join-Path $RunDirectory 'commands.ps1'
    [System.IO.File]::WriteAllLines($commandsFile, $commands, [System.Text.UTF8Encoding]::new($false))
    $plan = [ordered]@{
        schema_version = 1
        kind = 'interactive_trial_plan_not_executed'
        created_utc = [DateTime]::UtcNow.ToString('o')
        server = $ServerAddress
        config_source = $ConfigPath
        warmup_seconds = $WarmupSeconds
        requested_measurement_seconds = $DurationSeconds
        trial_count = $trials.Count
        duration_control = 'manual stopwatch and normal window close; analyzer checks saved duration'
        controlled_mods_directory = (Join-Path $RunDirectory 'mods')
        commands = $commandsFile
        trials = $trials.ToArray()
        warnings = @('This plan has not run any client or measured performance.', 'Use the same world, position, camera/input sequence, server load, mods, power profile and foreground window state in every trial.', 'The current colored diagnostic renderer cannot provide complete vanilla visual acceptance.', 'The current CLI joins offline-mode servers with the supplied name; account login for online servers must be completed through the client.', 'Place the controlled test mods in the shared run directory mods folder before every trial; user config-directory mods are not silently copied.')
    }
    Write-Json (Join-Path $RunDirectory 'run-plan.json') $plan
    Write-Host "Prepared $($trials.Count) trial configurations in $RunDirectory"
    Write-Host "Nothing executed. Review $commandsFile and use a stopwatch for every run."
    return
}

if ([string]::IsNullOrWhiteSpace($Output)) { $Output = Join-Path $projectRoot ".local/measurement-analysis-$stamp.json" }
$files = [System.Collections.Generic.List[string]]::new()
foreach ($spec in $Metrics) {
    foreach ($file in Get-ChildItem -Path $spec -File -ErrorAction Stop) {
        if (-not $files.Contains($file.FullName)) { $files.Add($file.FullName) }
    }
}
if ($files.Count -eq 0) { throw 'No metrics files matched.' }
$runs = [System.Collections.Generic.List[object]]::new()
foreach ($file in $files) {
    $source = Get-Content -LiteralPath $file -Raw | ConvertFrom-Json -Depth 16
    if ((Get-Field $source 'schema_version') -ne 1 -or (Get-Field $source 'kind') -ne 'actual_client_frames') { throw "Not a supported actual-client frame log: $file" }
    $samples = @(Get-Field $source 'samples')
    if ($samples.Count -eq 0 -or $null -eq $samples[0]) { throw "No frame samples in $file" }
    $frames = [System.Collections.Generic.List[double]]::new()
    $inputs = [System.Collections.Generic.List[double]]::new()
    $gpu = [System.Collections.Generic.List[double]]::new()
    $cpu = [System.Collections.Generic.List[double]]::new()
    $fences = [System.Collections.Generic.List[double]]::new()
    $warnings = [System.Collections.Generic.List[string]]::new()
    $elapsed = 0.0
    $measured = 0.0
    $discarded = 0
    $terrainSamples = 0
    $maximumVertices = 0L
    $maximumChunks = 0L
    foreach ($sample in $samples) {
        $rawFrame = Get-Field $sample 'frame_us'
        if ($null -eq $rawFrame) { throw "Missing frame_us in $file" }
        $frame = [double] $rawFrame
        if (-not [double]::IsFinite($frame) -or $frame -le 0) { throw "Invalid frame duration in $file" }
        if ($elapsed -lt $WarmupSeconds * 1e6) {
            $elapsed += $frame
            $discarded++
            continue
        }
        if ($measured -ge $DurationSeconds * 1e6) { break }
        $frames.Add($frame)
        $measured += $frame
        foreach ($field in @('input_to_submit_us', 'gpu_us', 'cpu_us', 'fence_wait_us')) {
            $value = Get-Field $sample $field
            if ($null -ne $value) {
                $numeric = [double] $value
                if (-not [double]::IsFinite($numeric) -or $numeric -lt 0) { throw "Invalid $field in $file" }
                switch ($field) {
                    'input_to_submit_us' { $inputs.Add($numeric) }
                    'gpu_us' { $gpu.Add($numeric) }
                    'cpu_us' { $cpu.Add($numeric) }
                    'fence_wait_us' { $fences.Add($numeric) }
                }
            }
        }
        $chunks = Get-Field $sample 'chunks'
        $vertices = Get-Field $sample 'vertices'
        if ($null -ne $chunks -and [long] $chunks -gt 0) { $terrainSamples++ }
        if ($null -ne $chunks) { $maximumChunks = [Math]::Max($maximumChunks, [long] $chunks) }
        if ($null -ne $vertices) { $maximumVertices = [Math]::Max($maximumVertices, [long] $vertices) }
    }
    if ($samples.Count -ge 120000) { $warnings.Add('Log reached the current 120000-frame retention ceiling; the original beginning may have been discarded and trial alignment is unknown.') }
    if ($measured -lt $DurationSeconds * 1e6) { $warnings.Add('Saved frame intervals do not contain the requested measurement duration after warm-up.') }
    if ($inputs.Count -eq 0) { $warnings.Add('No input latency samples. Missing input is not zero latency.') }
    if ($gpu.Count -eq 0) { $warnings.Add('No GPU timestamp samples.') }
    if ($terrainSamples -ne $frames.Count -or $terrainSamples -eq 0) { $warnings.Add('Some or all measured frames have no received terrain; this is not a continuous terrain workload.') }
    if ($maximumVertices -ge 3900000) { $warnings.Add('Diagnostic terrain vertex cap was reached; geometry may be incomplete.') }
    $warnings.Add('Nearby terrain presentation age and independent visual-correctness evidence are absent from the raw client schema.')
    $averageFps = if ($measured -gt 0) { $frames.Count * 1e6 / $measured } else { $null }
    $runs.Add([ordered]@{
        file = $file
        mode = (Get-Field $source 'mode')
        scenario = (Get-Field $source 'scenario')
        gpu = (Get-Field $source 'gpu')
        input_metric = (Get-Field $source 'input_metric')
        retained_source_frames = $samples.Count
        discarded_warmup_frames = $discarded
        discarded_warmup_seconds = $elapsed / 1e6
        measured_frames = $frames.Count
        measured_seconds = $measured / 1e6
        duration_requirement_met = ($measured -ge $DurationSeconds * 1e6)
        average_fps = $averageFps
        p99_frame_us = (Get-Percentile $frames.ToArray() 99)
        p999_frame_us = (Get-Percentile $frames.ToArray() 99.9)
        maximum_frame_us = (Get-Percentile $frames.ToArray() 100)
        input_sample_count = $inputs.Count
        p95_input_to_submit_us = (Get-Percentile $inputs.ToArray() 95)
        p99_cpu_us = (Get-Percentile $cpu.ToArray() 99)
        gpu_sample_count = $gpu.Count
        p99_gpu_us = (Get-Percentile $gpu.ToArray() 99)
        p99_fence_wait_us = (Get-Percentile $fences.ToArray() 99)
        terrain_frame_count = $terrainSamples
        maximum_chunks = $maximumChunks
        maximum_vertices = $maximumVertices
        acceptance_eligible = $false
        warnings = $warnings.ToArray()
    })
}
$report = [ordered]@{
    schema_version = 1
    kind = 'descriptive_actual_frame_analysis'
    analyzed_utc = [DateTime]::UtcNow.ToString('o')
    requested_warmup_seconds = $WarmupSeconds
    requested_measurement_seconds = $DurationSeconds
    quantile_method = 'nearest rank; absent input/GPU measurements excluded, not replaced by zero'
    average_fps_method = 'measured frame count divided by sum of measured frame intervals'
    release_acceptance = 'not_evaluable'
    speedup_claim = 'none'
    reason = 'Current raw logs lack nearby-update age and independent visual validation; this descriptive analyzer does not attest matched workload/settings or full client correctness.'
    runs = $runs.ToArray()
}
Write-Json $Output $report
$runs | ForEach-Object { [pscustomobject]@{ Mode = $_.mode; Seconds = [Math]::Round($_.measured_seconds, 2); Frames = $_.measured_frames; FPS = $(if ($null -eq $_.average_fps) { 'n/a' } else { [Math]::Round($_.average_fps, 2) }); P99ms = $(if ($null -eq $_.p99_frame_us) { 'n/a' } else { [Math]::Round($_.p99_frame_us / 1000, 3) }); Inputs = $_.input_sample_count; DurationMet = $_.duration_requirement_met } } | Format-Table -AutoSize | Out-Host
Write-Host "Descriptive report: $([System.IO.Path]::GetFullPath($Output))"
Write-Host 'Release acceptance: NOT EVALUABLE. No speedup or complete-client claim is made.'
