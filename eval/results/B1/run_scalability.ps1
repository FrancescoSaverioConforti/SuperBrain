param([Parameter(Mandatory=$true)][string]$ConfigurationPath)
$ErrorActionPreference = 'Stop'
$configuration = Get-Content -LiteralPath $ConfigurationPath -Raw -Encoding UTF8 | ConvertFrom-Json
$workspace = [IO.Path]::GetFullPath($configuration.workspace)
Set-Location -LiteralPath $workspace
$resultDirectory = [IO.Path]::GetFullPath($configuration.result_directory)
$statusPath = Join-Path $resultDirectory 'job-status.json'
$utf8 = New-Object System.Text.UTF8Encoding($false)
. (Join-Path $PSScriptRoot 'native_process.ps1')
# Exclusive supervisor ownership, including across app/session resumes.
$jobLock = [IO.File]::Open((Join-Path $resultDirectory 'coordinator.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)

function Write-Status([string]$stage, [int]$scale, [string]$message) {
    $seen = 0
    $latencies = Join-Path $resultDirectory "latencies-$scale.csv"
    if (Test-Path -LiteralPath $latencies) {
        $seen = [Math]::Max(0, @(Get-Content -LiteralPath $latencies).Count - 1)
    }
    $status = [ordered]@{
        stage = $stage; scale = $scale; rows_seen_not_yet_validated = $seen
        message = $message; updated_utc = [DateTime]::UtcNow.ToString('o')
        process_id = $PID; core_commit = $configuration.core_commit
        original_full_protocol_executed = $false
    }
    [IO.File]::WriteAllText($statusPath, ($status | ConvertTo-Json) + "`n", $utf8)
}

function Check-Binary {
    if ((Get-FileHash -LiteralPath $configuration.binary -Algorithm SHA256).Hash -ne $configuration.binary_sha256) {
        throw 'Benchmark binary changed; refusing to mix implementations'
    }
}

function Summary-Present([int]$scale) {
    $path = Join-Path $resultDirectory "summary-$scale.csv"
    if (!(Test-Path -LiteralPath $path)) { return $false }
    $summary = @(Import-Csv -LiteralPath $path)
    return $summary.Count -eq 1 -and [int]$summary[0].n -eq $scale -and [int]$summary[0].measured_queries -eq 100
}

function Run-Scale([int]$scale) {
    Check-Binary
    $root = $configuration.roots."$scale"
    if ([string]::IsNullOrWhiteSpace($root)) { throw 'Missing synthetic vault path' }
    Write-Status 'preparing_or_measuring' $scale 'Only completed CSVs will be validated as measured results'
    $log = Join-Path $resultDirectory "process-$scale"
    Invoke-BenchmarkProcess -FilePath $configuration.binary -Arguments @([string]$scale, $resultDirectory, '100', $root) -LogStem $log -OnWait { Write-Status 'preparing_or_measuring' $scale 'Native process active; only completed CSVs will be validated' }
    if (!(Summary-Present $scale)) { throw "No complete summary for N=$scale" }
}

try {
    Check-Binary
    # The already-running large-scale process may need substantial time. Do
    # not touch its live vault, recover it concurrently or infer its metrics.
    $largeId = [int]$configuration.existing_large_process_id
    while ($largeId -gt 0) {
        $existing = Get-Process -Id $largeId -ErrorAction SilentlyContinue
        if ($null -eq $existing) { break }
        if ([string]::IsNullOrWhiteSpace($existing.Path)) { throw 'Cannot verify running benchmark identity' }
        if ([IO.Path]::GetFullPath($existing.Path) -ne [IO.Path]::GetFullPath($configuration.binary)) { break }
        Write-Status 'large_scale_running' 100000 'Waiting for the authorized 100-query run; no extrapolation'
        Start-Sleep -Seconds 30
    }
    foreach ($scale in @(100, 1000, 10000)) {
        if (!(Summary-Present $scale)) { Run-Scale $scale }
    }
    if (!(Summary-Present 100000)) {
        # An interrupted native process is resumed through the public fixture
        # path once. Any failure is reported; there is no automatic data repair.
        Run-Scale 100000
    }
    Write-Status 'validating' 100000 'Checking all four actual samples, phases and percentiles'
    Invoke-BenchmarkProcess -FilePath $configuration.python -Arguments @('-B', (Join-Path $PSScriptRoot 'summarize_scalability.py'), $resultDirectory, $configuration.artifact_directory) -LogStem (Join-Path $resultDirectory 'validation')
    Copy-Item -LiteralPath (Join-Path $resultDirectory 'build-manifest.json') -Destination (Join-Path $configuration.artifact_directory 'build-manifest.json')
    Write-Status 'authorized_sample_complete_review_pending' 100000 'All four measured samples validated; final human/agent evidence review remains required'
} catch {
    Write-Status 'failed_review_required' 0 $_.Exception.Message
    throw
} finally {
    $jobLock.Dispose()
}
