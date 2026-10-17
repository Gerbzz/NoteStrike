$ErrorActionPreference = 'Stop'
$probeRoot = Join-Path $env:RUNNER_TEMP 'notestrike-editor-visual'
$notebook = Join-Path $probeRoot 'notebook'
New-Item -ItemType Directory -Force -Path $probeRoot | Out-Null
$cli = Join-Path $PWD 'target/debug/notestrike.exe'
$native = Join-Path $PWD 'target/debug/notestrike-native.exe'
& $cli init $notebook
if ($LASTEXITCODE -ne 0) { throw 'Fixture notebook initialization failed.' }
$hub = (& $cli new $notebook 'A Project Hub' "# Project Hub`n`nPlan the next release with a clean, local notebook.`n`n- Review priorities`n- Capture decisions`n- Follow up on open tasks").Trim()
if ($LASTEXITCODE -ne 0 -or $hub -notmatch '^[0-9a-f-]{36}$') { throw 'Fixture hub note creation failed.' }
for ($index = 1; $index -le 12; $index++) {
    $title = 'B Review ' + $index.ToString('00')
    $body = "# $title`n`nSee [[$hub|Project Hub]] for the running plan.`n`nA short note for the next review."
    & $cli new $notebook $title $body | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Fixture backlink $index failed." }
}
$backlinks = @(& $cli backlinks $notebook $hub)
if ($LASTEXITCODE -ne 0 -or $backlinks.Count -ne 12) { throw "Expected 12 real backlinks, found $($backlinks.Count)." }
Write-Host "FIXTURE_BACKLINKS:$($backlinks.Count)"
Add-Type -AssemblyName System.Drawing
foreach ($spec in @(@('WIDE', '1280', '800'), @('COMPACT', '800', '520'))) {
    $label = $spec[0]
    $capture = Join-Path $probeRoot ($label.ToLowerInvariant() + '.png')
    $stdout = Join-Path $probeRoot ($label.ToLowerInvariant() + '.out.txt')
    $stderr = Join-Path $probeRoot ($label.ToLowerInvariant() + '.err.txt')
    $process = Start-Process -FilePath $native -ArgumentList @($notebook, '--capture', $capture, $spec[1], $spec[2]) -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    try {
        $complete = $false
        for ($attempt = 0; $attempt -lt 45; $attempt++) {
            Start-Sleep -Seconds 1
            if (Test-Path -LiteralPath $capture) {
                $probe = [IO.File]::ReadAllBytes($capture)
                if ($probe.Length -gt 1000) {
                    $trailer = [byte[]](73, 69, 78, 68, 174, 66, 96, 130)
                    $complete = $true
                    for ($i = 0; $i -lt 8; $i++) {
                        if ($probe[$probe.Length - 8 + $i] -ne $trailer[$i]) { $complete = $false; break }
                    }
                    if ($complete) { break }
                }
            }
            $process.Refresh()
            if ($process.HasExited) { break }
        }
        if (-not $complete) {
            Write-Host "$label stdout:"; Get-Content -LiteralPath $stdout -ErrorAction SilentlyContinue
            Write-Host "$label stderr:"; Get-Content -LiteralPath $stderr -ErrorAction SilentlyContinue
            throw "Hosted $label editor did not produce a complete PNG within 45 seconds."
        }
    } finally {
        $process.Refresh()
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
    $bitmap = [System.Drawing.Bitmap]::new($capture)
    try {
        if ($bitmap.Width -lt [int]$spec[1] -or $bitmap.Height -lt [int]$spec[2]) { throw "$label dimensions too small." }
        $colors = [System.Collections.Generic.HashSet[int]]::new()
        $light = 0
        $ink = 0
        for ($y = 0; $y -lt $bitmap.Height; $y += 6) {
            for ($x = 0; $x -lt $bitmap.Width; $x += 6) {
                $pixel = $bitmap.GetPixel($x, $y)
                [void]$colors.Add($pixel.ToArgb())
                if ($pixel.R -gt 220 -and $pixel.G -gt 220 -and $pixel.B -gt 220) { $light++ }
                if ($pixel.R -lt 110 -and $pixel.G -lt 110 -and $pixel.B -lt 110) { $ink++ }
            }
        }
        if ($colors.Count -lt 20 -or $light -lt 200 -or $ink -lt 20) {
            throw "$label lacks expected editor content: colors=$($colors.Count), light=$light, ink=$ink"
        }
        Write-Host "${label}_DIMENSIONS:$($bitmap.Width)x$($bitmap.Height)"
        Write-Host "${label}_PIXELS:colors=$($colors.Count),light=$light,ink=$ink"
    } finally {
        $bitmap.Dispose()
    }
    $bytes = [IO.File]::ReadAllBytes($capture)
    Write-Host "${label}_SHA256:$((Get-FileHash -LiteralPath $capture -Algorithm SHA256).Hash)"
    Write-Host "${label}_BYTES:$($bytes.Length)"
    $encoded = [Convert]::ToBase64String($bytes)
    for ($offset = 0; $offset -lt $encoded.Length; $offset += 4000) {
        Write-Host ("${label}_CHUNK:" + $encoded.Substring($offset, [Math]::Min(4000, $encoded.Length - $offset)))
    }
}
