<#
.SYNOPSIS
AI upscaling round, step V1 (doc/process/rounds/2026-10-ai-upscaling.md): does
RTX Video Super Resolution change the picture on this machine?

.DESCRIPTION
1. Makes the 1080p reference (C:\vestra\rec\ref1080.mp4) from the 4K clip with
   the bundled ffmpeg (area downscale, first 10 s); skipped if present.
2. Runs gpu-probe three times, offscreen, 3840x2160 target, d3d11va decoding,
   vf d3d11vpp=scale=2 (1080p -> 2160p):
     nvidia-vsr   NVIDIA adapter, scaling-mode=nvidia
     nvidia-std   NVIDIA adapter, scaling-mode=standard
     amd-vsr      AMD adapter,    scaling-mode=nvidia (expect a warning, same pixels as standard)
   Each run plays for -Warmup + -Seconds, then pauses on the frame at -At and
   writes it as <label>.png. Runs planned longer than 20 s (warm-up +
   measurement + 7 s start-up and seek) are refused; a run still
   going 30 s after its planned length is killed.
3. Extracts the same frame (t = -At) from the 4K source, and the 1080p frame
   upscaled by ffmpeg (bicubic, lanczos) as CPU baselines.
4. Prints SSIM/PSNR of each PNG against the 4K frame.
5. Samples \GPU Engine(*engtype_3D)\Utilization Percentage per adapter LUID
   (WMI Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine, all
   processes and the probe alone) every ~0.5 s during each run.

Everything lands in -Out (default C:\vestra\rec\vsr): PNG files, the probe logs
(<label>.log, full mpv log at -Log level), summary.txt.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File C:\vestra\wt-wgpu-port\src\host\unsafe\gpu-probe\vsr-check.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File ...\vsr-check.ps1 -Nvidia 1 -Amd 0 -Only nvidia-vsr
#>
param(
    # DXGI adapter indices (gpu-probe adapters), or warp; empty = pick by vendor id.
    [string]$Nvidia = '',
    [string]$Amd = '',
    [string]$Probe = 'C:\vestra\gpu-probe\gpu-probe.exe',
    [string]$Ffmpeg = 'C:\vestra\deps\ffmpeg\ffmpeg.exe',
    [string]$Source = 'C:\vestra\clips\h264-2160p60.mp4',
    [string]$Ref = 'C:\vestra\rec\ref1080.mp4',
    [string]$Out = 'C:\vestra\rec\vsr',
    # The compared frame (seconds; 5 = frame 300 of a 60 fps clip, on a frame boundary).
    [double]$At = 5,
    [double]$Warmup = 3,
    [double]$Seconds = 5,
    # mpv log level for the probe (its log file); the filter summary needs at least v.
    [string]$Log = 'v',
    # Run only these labels (nvidia-vsr, nvidia-std, amd-vsr); default all three.
    [string[]]$Only = @()
)

$ErrorActionPreference = 'Stop'
$MaxRunSeconds = 20   # coordinator rule: one probe run <= 20 s
$OverrunKill = 30     # kill a run this long after its planned length

function Fail([string]$message) { Write-Host "vsr-check: $message" -ForegroundColor Red; exit 1 }

if ($Warmup -lt 0 -or $Seconds -le 0) { Fail 'bad -Warmup/-Seconds' }
# Planned length: start-up (~2 s) + warm-up + measurement + the --at seek (gives up after 5 s).
$planned = $Warmup + $Seconds + 7
if ($planned -gt $MaxRunSeconds) {
    Fail "refusing: a probe run would take up to $planned s (-Warmup + -Seconds + 7 s start-up and seek), longer than $MaxRunSeconds s"
}
foreach ($f in @($Probe, $Ffmpeg, $Source)) { if (-not (Test-Path $f)) { Fail "missing $f" } }
New-Item -ItemType Directory -Force $Out | Out-Null
New-Item -ItemType Directory -Force (Split-Path $Ref) | Out-Null
$summary = Join-Path $Out 'summary.txt'
Set-Content -Path $summary -Value "vsr-check $(Get-Date -Format s)" -Encoding utf8
function Say([string]$line) { Write-Host $line; Add-Content -Path $summary -Value $line -Encoding utf8 }

# ffmpeg with its arguments; returns stderr+stdout lines, fails on a non-zero exit.
function Invoke-Ffmpeg([string[]]$arguments) {
    $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    $lines = & $Ffmpeg -hide_banner -nostdin @arguments 2>&1 | ForEach-Object { "$_" }
    $code = $LASTEXITCODE; $ErrorActionPreference = $old
    if ($code -ne 0) { $lines | Select-Object -Last 15 | ForEach-Object { Write-Host $_ }; Fail "ffmpeg exited $code" }
    $lines
}

# ---------------------------------------------------------------- 1. reference
if (Test-Path $Ref) {
    Say "reference: $Ref exists, not remade"
} else {
    Say "reference: making $Ref (first 10 s of $Source, scale=1920:1080:flags=area, x264 crf 12)"
    Invoke-Ffmpeg @('-y', '-i', $Source, '-t', '10', '-vf', 'scale=1920:1080:flags=area',
        '-c:v', 'libx264', '-preset', 'medium', '-crf', '12', '-pix_fmt', 'yuv420p', '-an', $Ref) | Out-Null
}

# ---------------------------------------------------------------- adapters
$adapterLines = & $Probe adapters
if ($LASTEXITCODE -ne 0) { Fail 'gpu-probe adapters failed' }
$adapters = foreach ($line in $adapterLines) {
    if ($line -match '^\[(\d+)\] (.+) vendor=0x([0-9a-f]{4}) \(.*\) LUID=([0-9a-f]{8}):([0-9a-f]{8})') {
        [pscustomobject]@{
            Index = [int]$matches[1]; Name = $matches[2]; Vendor = $matches[3]
            # WMI instance names write the LUID as luid_0x<high>_0x<low>.
            Luid = ('0x{0}_0x{1}' -f $matches[4], $matches[5]).ToLower()
        }
    }
}
$adapters | ForEach-Object { Say ("adapter [{0}] {1} vendor=0x{2} luid={3}" -f $_.Index, $_.Name, $_.Vendor, $_.Luid) }
function Pick([string]$given, [string]$vendor, [string]$what) {
    if ($given) { return $given }
    $a = $adapters | Where-Object Vendor -eq $vendor | Select-Object -First 1
    if (-not $a) { Fail "no $what adapter (vendor 0x$vendor); pass -$what <index>" }
    $a.Index
}
$nvidiaIndex = Pick $Nvidia '10de' 'Nvidia'
$amdIndex = Pick $Amd '1002' 'Amd'

# ---------------------------------------------------------------- 2. probe runs
$runs = @(
    @{ Label = 'nvidia-vsr'; Adapter = $nvidiaIndex; Mode = 'nvidia' },
    @{ Label = 'nvidia-std'; Adapter = $nvidiaIndex; Mode = 'standard' },
    @{ Label = 'amd-vsr'; Adapter = $amdIndex; Mode = 'nvidia' }
) | Where-Object { $Only.Count -eq 0 -or $Only -contains $_.Label }

function Sample-3d([int]$probePid) {
    # One sample: summed 3D utilisation per LUID, all processes and the probe alone.
    $rows = Get-CimInstance -ClassName Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine `
        -Filter "Name LIKE '%engtype_3D'" -ErrorAction SilentlyContinue
    $all = @{}; $mine = @{}
    foreach ($r in $rows) {
        if ($r.Name -notmatch '^pid_(\d+)_luid_(0x[0-9a-fA-F]{8}_0x[0-9a-fA-F]{8})_') { continue }
        $luid = $matches[2].ToLower(); $value = [double]$r.UtilizationPercentage
        $all[$luid] = [double]$all[$luid] + $value
        if ([int]$matches[1] -eq $probePid) { $mine[$luid] = [double]$mine[$luid] + $value }
    }
    @{ All = $all; Mine = $mine }
}

function Quote([string]$arg) { if ($arg -match '[\s"]') { '"' + ($arg -replace '"', '\"') + '"' } else { $arg } }

$results = @()
foreach ($run in $runs) {
    $label = $run.Label
    $png = Join-Path $Out "$label.png"
    $logFile = Join-Path $Out "$label.log"
    Remove-Item -ErrorAction SilentlyContinue $png, $logFile, "$logFile.err"
    $probeArgs = @('run', '--adapter', "$($run.Adapter)", '--src', $Ref, '--size', '3840x2160',
        '--hwdec', 'd3d11va', '--vf', "d3d11vpp=scale=2:scaling-mode=$($run.Mode)",
        '--at', "$At", '--warmup', "$Warmup", '--seconds', "$Seconds", '--png', $png, '--log', $Log)
    $commandLine = ($probeArgs | ForEach-Object { Quote $_ }) -join ' '
    Say ''
    Say "== $label : $Probe $commandLine"
    $p = Start-Process -FilePath $Probe -ArgumentList $commandLine -NoNewWindow -PassThru `
        -RedirectStandardOutput $logFile -RedirectStandardError "$logFile.err"
    $null = $p.Handle   # keeps ExitCode readable after exit (Windows PowerShell)
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $deadline = $planned + $OverrunKill
    $samples = @(); $killed = $false
    while (-not $p.HasExited) {
        if ($clock.Elapsed.TotalSeconds -gt $deadline) {
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            $killed = $true
            break
        }
        $samples += , (Sample-3d $p.Id)
        Start-Sleep -Milliseconds 500
    }
    $p.WaitForExit()
    $wall = [math]::Round($clock.Elapsed.TotalSeconds, 1)
    if ($killed) { Say "  KILLED after $wall s (planned $planned s + $OverrunKill s)" }
    else { Say "  exit $($p.ExitCode) after $wall s" }

    $text = if (Test-Path $logFile) { Get-Content $logFile } else { @() }
    # Key lines in log order (each once), up to the filter log section.
    $key = '^D3D11 device:|^\s+(adapter|hwdec-current|vf|video-params|video-out-params) = |^\s+first frame:|^\s+render target |^\s+frames rendered:|^\s+mpv render GPU time|^\s+--at |^\s+wrote |WARNING|^error:'
    foreach ($line in $text) {
        if ($line -like '== mpv filter/decoder log*') { break }
        if ($line -match $key) { Say $line }
    }
    Say '  -- filter/decoder lines (d3d11vpp, Super Resolution, RTX Video, vf [out], warnings/errors):'
    $section = $false
    foreach ($line in $text) {
        if ($line -like '== mpv filter/decoder log*') { $section = $true; continue }
        if (-not $section) { continue }
        if ($line -match 'd3d11vpp|Super Resolution|RTX Video|\[mpv (warn|error|fatal) |\] \[out\] |\] \[d3d11vpp\] |hwdec|Using hardware decoding') {
            Say $line
        }
    }
    if (Test-Path "$logFile.err") { Get-Content "$logFile.err" | Where-Object { $_ } | ForEach-Object { Say "  stderr: $_" } }

    # 3D load per LUID: mean and max over the samples (all processes / probe only).
    foreach ($a in $adapters) {
        $all = @($samples | ForEach-Object { [double]$_.All[$a.Luid] })
        $mine = @($samples | ForEach-Object { [double]$_.Mine[$a.Luid] })
        if ($all.Count -eq 0) { continue }
        $m = ($all | Measure-Object -Average -Maximum); $n = ($mine | Measure-Object -Average -Maximum)
        Say ('  3D load {0,-40} all {1,5:N1}% mean {2,5:N1}% max | probe {3,5:N1}% mean {4,5:N1}% max ({5} samples)' -f `
                $a.Name, $m.Average, $m.Maximum, $n.Average, $n.Maximum, $all.Count)
    }
    $results += , @{ Label = $label; Png = $png }
}

# ---------------------------------------------------------------- 3. reference frames
# The frame at t = At (4K source and 1080p reference have the same timestamps;
# select keeps the first frame from 8 ms before At, i.e. the one at At, as the
# probe's exact seek does).
$select = "select=gte(t\,$($At - 0.008))"
$ref4k = Join-Path $Out 'source-4k.png'
$ref1080 = Join-Path $Out 'ref-1080.png'
Say ''
Say "== reference frames at t=$At"
Invoke-Ffmpeg @('-y', '-i', $Source, '-vf', $select, '-frames:v', '1', '-fps_mode', 'passthrough', $ref4k) | Out-Null
Invoke-Ffmpeg @('-y', '-i', $Ref, '-vf', $select, '-frames:v', '1', '-fps_mode', 'passthrough', $ref1080) | Out-Null
foreach ($flags in @('bicubic', 'lanczos')) {
    $up = Join-Path $Out "ffmpeg-$flags.png"
    Invoke-Ffmpeg @('-y', '-i', $ref1080, '-vf', "scale=3840:2160:flags=$flags", $up) | Out-Null
    $results += , @{ Label = "ffmpeg-$flags (CPU baseline)"; Png = $up }
}

# ---------------------------------------------------------------- 4. SSIM / PSNR
Say ''
Say "== SSIM / PSNR against $ref4k (yuv444p; higher is closer to the 4K source)"
foreach ($r in $results) {
    if (-not (Test-Path $r.Png)) { Say ('  {0,-28} no PNG' -f $r.Label); continue }
    $graph = '[0:v]format=yuv444p,split[a1][a2];[1:v]format=yuv444p,split[b1][b2];[a1][b1]ssim;[a2][b2]psnr'
    $lines = Invoke-Ffmpeg @('-nostats', '-i', $r.Png, '-i', $ref4k, '-filter_complex', $graph, '-f', 'null', '-')
    $ssim = ($lines | Where-Object { $_ -match 'SSIM Y:' } | Select-Object -Last 1) -replace '^.*SSIM ', 'SSIM '
    $psnr = ($lines | Where-Object { $_ -match 'PSNR y:' } | Select-Object -Last 1) -replace '^.*PSNR ', 'PSNR '
    if (-not $ssim) { $ssim = 'SSIM n/a (sizes differ?)' }
    Say ('  {0,-28} {1}' -f $r.Label, $ssim)
    Say ('  {0,-28} {1}' -f '', $psnr)
}
# Do the two NVIDIA runs differ at all? (identical PNG files = VSR did nothing)
$vsr = Join-Path $Out 'nvidia-vsr.png'; $std = Join-Path $Out 'nvidia-std.png'
if ((Test-Path $vsr) -and (Test-Path $std)) {
    $lines = Invoke-Ffmpeg @('-nostats', '-i', $vsr, '-i', $std, '-filter_complex',
        '[0:v]format=yuv444p[a];[1:v]format=yuv444p[b];[a][b]psnr', '-f', 'null', '-')
    $psnr = ($lines | Where-Object { $_ -match 'PSNR y:' } | Select-Object -Last 1) -replace '^.*PSNR ', 'PSNR '
    Say "  nvidia-vsr vs nvidia-std: $psnr (inf = identical pixels)"
}
Say ''
Say "summary: $summary"
