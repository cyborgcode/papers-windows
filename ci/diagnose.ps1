# Reproduce a crash scenario under gdb and save the backtrace.
# Used by the workflow's diagnose mode (debug-symbol build).
param(
    [Parameter(Mandatory = $true)][string]$Bundle,
    [Parameter(Mandatory = $true)][string]$Document,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$Gdb,
    # SendKeys strings, sent one by one with a pause after each
    [string[]]$Keys = @('{F5}', '{ESC}'),
    [string]$WindowTitle = 'sample.pdf'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$exe = Join-Path $Bundle 'bin\papers.exe'
$gdb = $Gdb
$log = Join-Path $OutDir 'gdb.txt'

$gdbArgs = @(
    '-batch',
    '-ex', '"set pagination off"',
    '-ex', '"set print thread-events off"',
    '-ex', 'run',
    '-ex', '"echo \n=== crashing thread ===\n"',
    '-ex', '"bt 60"',
    '-ex', '"echo \n=== all threads ===\n"',
    '-ex', '"thread apply all bt 30"',
    '--args', "`"$exe`"", "`"$Document`""
)
$gdbProc = Start-Process -FilePath $gdb -ArgumentList $gdbArgs -PassThru -RedirectStandardOutput $log -RedirectStandardError (Join-Path $OutDir 'gdb.stderr.txt')
Start-Sleep -Seconds 25

$shell = New-Object -ComObject WScript.Shell
foreach ($k in $Keys) {
    [void]$shell.AppActivate($WindowTitle)
    Start-Sleep -Milliseconds 500
    [System.Windows.Forms.SendKeys]::SendWait($k)
    Start-Sleep -Seconds 5
}
Start-Sleep -Seconds 20

if (-not $gdbProc.HasExited) {
    "papers did not crash within the scenario" | Add-Content $log
    Get-Process papers -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 5
    if (-not $gdbProc.HasExited) { Stop-Process -Id $gdbProc.Id -Force }
}
Get-Content $log -Tail 120
