param([Parameter(Mandatory=$true)][string]$Workspace)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $Workspace
if (Get-Process -Name cargo,rustc,b1_scalability -ErrorAction SilentlyContinue) {
    throw 'Active build/benchmark: do not run concurrently'
}
$dependencies = Join-Path $Workspace 'target/debug/deps'
$build = Join-Path $Workspace '.superpowers/security-audit-build'
New-Item -ItemType Directory -Path $build -Force | Out-Null
$binary = Join-Path $build 'deletion-read-gate.exe'
. (Join-Path $Workspace 'eval/results/B1/native_process.ps1')
$arguments = @('--edition=2021', (Join-Path $PSScriptRoot 'deletion-read-gate.rs'),
    '-L', "dependency=$dependencies", '-o', $binary)
foreach ($name in @('vault_security', 'uuid', 'rusqlite')) {
    $rlib = Get-ChildItem -LiteralPath $dependencies -Filter "lib$name-*.rlib" |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (!$rlib) { throw "Missing cached dependency $name; no automatic Cargo build" }
    $arguments += @('--extern', "$name=$($rlib.FullName)")
}
Invoke-BenchmarkProcess -FilePath (Get-Command rustc).Source -Arguments $arguments -LogStem (Join-Path $build 'deletion-read-gate-build')
Invoke-BenchmarkProcess -FilePath $binary -Arguments @('--audit-probe') -LogStem (Join-Path $build 'deletion-read-gate-run')
Get-Content -LiteralPath (Join-Path $build 'deletion-read-gate-run.stdout.log')
