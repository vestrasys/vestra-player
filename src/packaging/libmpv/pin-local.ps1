# Pins a locally built libmpv (build-windows.sh) into a deps directory for a
# branch: a copy of the current deps (VESTRA_DEPS_DIR layout) with this
# libmpv-2.dll, its MSVC import library mpv.lib (as deps.yml makes it) and
# a note in components.json. Point env.ps1's MPV_LIBRARY_DIR and
# VESTRA_DEPS_DIR at -Out to build and stage the host with it.
#   pin-local.ps1 -Dll <work>\out\libmpv-2.dll [-From C:\vestra\deps] [-Out C:\vestra\deps-t8]
param(
  [Parameter(Mandatory)] [string]$Dll,
  [string]$From = 'C:\vestra\deps',
  [string]$Out = 'C:\vestra\deps-t8'
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $Out | Out-Null
robocopy $From $Out /E /XF libmpv-2.dll mpv.lib components.json /NFL /NDL /NJH /NJS | Out-Null
Copy-Item $Dll "$Out\libmpv-2.dll" -Force
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$version = (Get-Content "$vs\VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt").Trim()
$bin = "$vs\VC\Tools\MSVC\$version\bin\Hostx64\x64"
$exports = & "$bin\dumpbin.exe" /nologo /exports "$Out\libmpv-2.dll"
$names = $exports | ForEach-Object { if ($_ -match '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]{8}\s+(\S+)') { $Matches[1] } }
if (-not $names) { throw 'no exports found in libmpv-2.dll' }
$def = Join-Path $env:TEMP 'mpv-t8.def'
Set-Content -Path $def -Value (@('LIBRARY libmpv-2', 'EXPORTS') + $names)
& "$bin\lib.exe" /nologo /def:$def /machine:x64 /out:"$Out\mpv.lib"
if ($LASTEXITCODE -ne 0) { throw 'lib.exe failed' }
Remove-Item "$Out\mpv.exp" -ErrorAction SilentlyContinue
$sha = (Get-FileHash "$Out\libmpv-2.dll" -Algorithm SHA256).Hash.ToLower()
$components = Get-Content "$From\components.json" -Raw | ConvertFrom-Json
foreach ($c in $components.components) {
  if ($c.name -eq 'mpv') {
    $c.note = "LOCAL BUILD (src/packaging/libmpv/build-windows.sh: FFmpeg 9.1 with fruc_vulkan, mpv $($c.version) + mpv-lavfi-hwdevice.patch), libmpv-2.dll sha256 $sha. Not a deps release."
  }
}
# UTF-8 without a byte order mark (Windows PowerShell's -Encoding utf8 writes one, which JSON.parse refuses).
[System.IO.File]::WriteAllText("$Out\components.json", ($components | ConvertTo-Json -Depth 8), (New-Object System.Text.UTF8Encoding $false))
"pinned $Out\libmpv-2.dll sha256 $sha ($($names.Count) exports)"
