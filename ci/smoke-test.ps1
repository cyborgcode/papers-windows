# Launch the bundled papers.exe on each sample document, check that it keeps
# running, and capture a screenshot plus its stderr for the CI log branch.
param(
    [Parameter(Mandatory = $true)][string]$Bundle,
    [Parameter(Mandatory = $true)][string]$Samples,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [int]$WaitSeconds = 15
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

function Save-Screenshot([string]$Path) {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

$exe = Join-Path $Bundle 'bin\papers.exe'
$cases = @(
    @{ Name = 'start-screen'; File = $null },
    @{ Name = 'pdf'; File = 'sample.pdf' },
    @{ Name = 'djvu'; File = 'sample.djvu' },
    @{ Name = 'tiff'; File = 'sample.tiff' },
    @{ Name = 'cbz'; File = 'sample.cbz' },
    @{ Name = 'cbt'; File = 'sample.cbt' },
    @{ Name = 'cb7'; File = 'sample.cb7' }
)

$failures = @()
$summary = @()
foreach ($case in $cases) {
    $name = $case.Name
    $stderr = Join-Path $OutDir "$name.stderr.txt"
    $stdout = Join-Path $OutDir "$name.stdout.txt"
    $docArgs = @()
    if ($case.File) {
        $doc = Join-Path $Samples $case.File
        if (-not (Test-Path $doc)) { $summary += "$name : SKIPPED (no sample)"; continue }
        $docArgs += "`"$doc`""
    }
    $env:G_MESSAGES_DEBUG = ''
    $startArgs = @{ FilePath = $exe; PassThru = $true; RedirectStandardError = $stderr; RedirectStandardOutput = $stdout }
    if ($docArgs.Count -gt 0) { $startArgs.ArgumentList = $docArgs }
    $proc = Start-Process @startArgs
    Start-Sleep -Seconds $WaitSeconds
    Save-Screenshot (Join-Path $OutDir "$name.png")
    if ($proc.HasExited) {
        $code = '0x{0:X8}' -f $proc.ExitCode
        $failures += $name
        $summary += "$name : EXITED early with code $code"
    } else {
        $title = (Get-Process -Id $proc.Id).MainWindowTitle
        $summary += "$name : running, window title '$title'"
        Stop-Process -Id $proc.Id -Force
    }
    Start-Sleep -Seconds 2
}

# Feature tour on the PDF sample, driven by Papers' own keyboard shortcuts
$tourDoc = Join-Path $Samples 'sample.pdf'
if (Test-Path $tourDoc) {
    $proc = Start-Process -FilePath $exe -ArgumentList "`"$tourDoc`"" -PassThru `
        -RedirectStandardError (Join-Path $OutDir 'tour.stderr.txt') -RedirectStandardOutput (Join-Path $OutDir 'tour.stdout.txt')
    Start-Sleep -Seconds $WaitSeconds
    $shell = New-Object -ComObject WScript.Shell
    $steps = @(
        @{ Name = 'find'; Keys = @('^f', 'page'); Undo = @('{ESC}') },
        @{ Name = 'properties'; Keys = @('%{ENTER}'); Undo = @('{ESC}') },
        @{ Name = 'night-mode'; Keys = @('^i'); Undo = @('^i') },
        @{ Name = 'dual-page'; Keys = @('d'); Undo = @('d') },
        @{ Name = 'zoom-in'; Keys = @('^{ADD}', '^{ADD}'); Undo = @('a') },
        @{ Name = 'rotate'; Keys = @('^{RIGHT}'); Undo = @('^{LEFT}') },
        @{ Name = 'shortcuts'; Keys = @('^?'); Undo = @('{ESC}') },
        @{ Name = 'presentation'; Keys = @('{F5}'); Undo = @('{ESC}') },
        @{ Name = 'print-dialog'; Keys = @('^p'); Undo = @('{ESC}') }
    )
    $i = 0
    foreach ($step in $steps) {
        $i++
        if ($proc.HasExited) { $summary += "tour: papers exited before step $($step.Name)"; $failures += 'tour'; break }
        [void]$shell.AppActivate($proc.Id)
        Start-Sleep -Milliseconds 500
        foreach ($k in $step.Keys) { [System.Windows.Forms.SendKeys]::SendWait($k); Start-Sleep -Milliseconds 400 }
        Start-Sleep -Seconds 3
        Save-Screenshot (Join-Path $OutDir ("tour-{0:D2}-{1}.png" -f $i, $step.Name))
        foreach ($k in $step.Undo) { [System.Windows.Forms.SendKeys]::SendWait($k); Start-Sleep -Milliseconds 400 }
        Start-Sleep -Seconds 1
    }
    if (-not $proc.HasExited) {
        $summary += "tour: completed $i steps, papers still running"
        Stop-Process -Id $proc.Id -Force
    }
}

$summary | Tee-Object -FilePath (Join-Path $OutDir 'summary.txt')
if ($failures.Count -gt 0) {
    Write-Error "Smoke test failed for: $($failures -join ', ')"
}
