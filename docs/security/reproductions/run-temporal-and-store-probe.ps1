param([Parameter(Mandatory=$true)][string]$Workspace)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $Workspace
if (Get-Process -Name cargo,rustc,b1_scalability -ErrorAction SilentlyContinue) {
    throw 'Active build/benchmark: do not run concurrently'
}
$base = Join-Path $Workspace '.superpowers/security-audit-build'
$legacy = Join-Path $base 'legacy-index'
New-Item -ItemType Directory -Path $legacy -Force | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
# Exact historical source; no checkout reset, worktree or production mutation.
foreach ($name in @('lib','bm25','document','diagnostics','indexer_impl','search','time_range','trigram')) {
    $content = & git show "338fbf1:crates/index/src/$name.rs"
    if ($LASTEXITCODE -ne 0) { throw 'Historical source export failed' }
    [IO.File]::WriteAllText((Join-Path $legacy "$name.rs"), ($content -join "`n") + "`n", $utf8)
}
$rootPath = $Workspace.Replace('\','/')
$sourcePath = (Join-Path $PSScriptRoot 'temporal-and-store-probe.rs').Replace('\','/')
$legacyManifest = @"
[package]
name = "historical-index-audit"
version = "0.0.0"
edition = "2021"
[lib]
path = "lib.rs"
[dependencies]
domain = { path = "$rootPath/crates/domain" }
storage = { path = "$rootPath/crates/storage" }
chrono = { version = "0.4", default-features = false, features = ["std"] }
chrono-tz = "0.10"
"@
[IO.File]::WriteAllText((Join-Path $legacy 'Cargo.toml'), $legacyManifest, $utf8)
$manifest = @"
[package]
name = "security-audit-probe"
version = "0.0.0"
edition = "2021"
[workspace]
[[bin]]
name = "security-audit-probe"
path = "$sourcePath"
[dependencies]
historical_index = { package = "historical-index-audit", path = "legacy-index" }
index = { path = "$rootPath/crates/index" }
domain = { path = "$rootPath/crates/domain" }
storage = { path = "$rootPath/crates/storage" }
policy = { path = "$rootPath/crates/policy" }
vault-security = { path = "$rootPath/crates/vault-security" }
chrono = { version = "0.4", default-features = false, features = ["std"] }
uuid = { version = "1", features = ["v4"] }
git2 = "0.21"
"@
$manifestPath = Join-Path $base 'Cargo.toml'
[IO.File]::WriteAllText($manifestPath, $manifest, $utf8)
# Unify feature/type identities instead of choosing arbitrary cached rlibs.
# Only this isolated lock changes; the repository lock stays untouched.
Copy-Item -LiteralPath (Join-Path $Workspace 'Cargo.lock') -Destination (Join-Path $base 'Cargo.lock')
$log = Join-Path $base 'probe'
$arguments = @(
    'run','--offline','--manifest-path',$manifestPath,'--target-dir',(Join-Path $Workspace 'target'),
    '--bin','security-audit-probe'
)
# In Windows PowerShell 5 informational stderr must not become Stop errors.
# Native redirection with Continue and the explicit exit code avoids waiting
# for an entire descendant job after Cargo itself has finished.
$previousErrorAction = $ErrorActionPreference
try {
    $ErrorActionPreference = 'Continue'
    & (Get-Command cargo).Source @arguments 1> ($log + '.stdout.log') 2> ($log + '.stderr.log')
    $nativeExit = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousErrorAction
}
if ($nativeExit -ne 0) { throw 'Probe failed; inspect stderr log, do not infer passing checks' }
Get-Content -LiteralPath ($log + '.stdout.log') -Encoding UTF8
