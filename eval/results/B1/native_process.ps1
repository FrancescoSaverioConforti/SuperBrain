function Invoke-BenchmarkProcess {
    param([string]$FilePath, [string[]]$Arguments, [string]$LogStem, [scriptblock]$OnWait)
    # All arguments are fixed switches, numbers or Windows paths. Never route
    # native stderr through PowerShell's ErrorActionPreference/RemoteException.
    $quoted = @($Arguments | ForEach-Object {
        if ($_.Contains('"')) { throw 'Unexpected quote in benchmark process argument' }
        '"' + ($_ -replace '(\\+)$', '$1$1') + '"'
    })
    if ($OnWait) { & $OnWait }
    # -Wait is intentional: Windows PowerShell otherwise can return a Process
    # with an unavailable ExitCode even after a fast child has already exited.
    # This wait runs inside the hidden supervisor, not the interactive tool.
    $process = Start-Process -FilePath $FilePath -ArgumentList $quoted -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput ($LogStem + '.stdout.log') -RedirectStandardError ($LogStem + '.stderr.log')
    try {
        if ($null -eq $process.ExitCode -or $process.ExitCode -ne 0) {
            throw "Native benchmark process failed; inspect $LogStem stdout/stderr logs"
        }
    } finally {
        $process.Dispose()
    }
}
