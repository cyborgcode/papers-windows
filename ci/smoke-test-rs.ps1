# Start the Rust build of Papers on the sample PDF, drive a few features through
# keyboard shortcuts, take screenshots, and fail if it exits on its own.
param(
    [Parameter(Mandatory = $true)][string]$Bundle,
    [Parameter(Mandatory = $true)][string]$Samples,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [int]$WaitSeconds = 8
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$env:PAPERS_NO_UPDATE_CHECK = '1'
$env:RUST_LOG = 'info'

function Save-Screenshot([string]$Path) {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

$exe = Join-Path $Bundle 'papers.exe'
$updater = Join-Path $Bundle 'papers-updater.exe'
$failures = @()
$summary = @()

$v = & $updater --version
$summary += "updater: $v"
if ($LASTEXITCODE -ne 0) { $failures += 'updater-version' }

$doc = Join-Path $Samples 'sample.pdf'
foreach ($case in @(@{ Name = 'start-screen'; Args = @() }, @{ Name = 'pdf'; Args = @("`"$doc`"") })) {
    $start = @{ FilePath = $exe; PassThru = $true
        RedirectStandardError = (Join-Path $OutDir "$($case.Name).stderr.txt")
        RedirectStandardOutput = (Join-Path $OutDir "$($case.Name).stdout.txt") }
    if ($case.Args.Count) { $start.ArgumentList = $case.Args }
    $proc = Start-Process @start
    Start-Sleep -Seconds $WaitSeconds
    Save-Screenshot (Join-Path $OutDir "$($case.Name).png")
    if ($proc.HasExited) {
        $failures += $case.Name
        $summary += ('{0}: exited early with code 0x{1:X8}' -f $case.Name, $proc.ExitCode)
        continue
    }
    $summary += "$($case.Name): running, window title '$((Get-Process -Id $proc.Id).MainWindowTitle)'"
    if ($case.Name -eq 'pdf') {
        $shell = New-Object -ComObject WScript.Shell
        $steps = @(
            @{ Name = 'night-mode'; Keys = @('^i') },
            @{ Name = 'day-mode'; Keys = @('^i') },
            @{ Name = 'find'; Keys = @('^f', 'page', '{ENTER}') },
            @{ Name = 'close-find'; Keys = @('{ESC}') },
            @{ Name = 'zoom-in'; Keys = @('^{ADD}', '^{ADD}') },
            @{ Name = 'fit-width'; Keys = @('^0') },
            @{ Name = 'rotate'; Keys = @('^{RIGHT}') },
            @{ Name = 'rotate-back'; Keys = @('^{LEFT}') },
            @{ Name = 'sidebar'; Keys = @('{F9}') },
            @{ Name = 'next-page'; Keys = @('n') },
            @{ Name = 'properties'; Keys = @('%{ENTER}') },
            @{ Name = 'presentation'; Keys = @('{F5}') },
            @{ Name = 'leave-presentation'; Keys = @('{ESC}') },
            @{ Name = 'night-again'; Keys = @('^i') }
        )
        $i = 0
        foreach ($step in $steps) {
            $i++
            if ($proc.HasExited) { break }
            [void]$shell.AppActivate($proc.Id)
            Start-Sleep -Milliseconds 400
            foreach ($k in $step.Keys) { [System.Windows.Forms.SendKeys]::SendWait($k); Start-Sleep -Milliseconds 300 }
            Start-Sleep -Seconds 2
            Save-Screenshot (Join-Path $OutDir ('tour-{0:D2}-{1}.png' -f $i, $step.Name))
        }
        if ($proc.HasExited) {
            $failures += 'tour'
            $summary += ('tour: exited during the tour with code 0x{0:X8}' -f $proc.ExitCode)
            continue
        }
        $summary += "tour: $i steps, still running"
    }
    Stop-Process -Id $proc.Id -Force
    Start-Sleep -Seconds 1
}

$summary | Tee-Object -FilePath (Join-Path $OutDir 'summary.txt')
if ($failures.Count -gt 0) { Write-Error "Smoke test failed for: $($failures -join ', ')" }
