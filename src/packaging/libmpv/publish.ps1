# Ships a libmpv built by build-windows.sh, the way deps.yml would, without remote CI.
#
#   1. publish.ps1 -Step libmpv -Assets <dir>
#        A pre-release libmpv-ffmpeg<ver>-<date> on this repository with the mpv-dev zip
#        and the MSYS2 package archive; both must match deps.json "upstream.windows-mpv"
#        (url, sha256, msys2_packages, msys2_packages_sha256).
#   2. publish.ps1 -Step deps
#        The Windows half of deps.yml on this machine: collect-windows-deps.mjs (it fetches
#        the pinned libmpv asset and the FFmpeg tools of the pinned deps release, and checks
#        them), mpv.lib from the DLL's exports, vestra-deps-windows-x64.zip; the macOS
#        assets copied unchanged from the
#        release deps.json names (as deps.yml does for a system it does not rebuild); then a
#        pre-release deps-<date>.N with SHA256SUMS, and deps.json's release/assets rewritten.
#        With remote CI, `gh workflow run deps.yml -f platforms=windows` does the same.
#
# Without -Publish it only prints what it would upload. Needs gh, Node 26, VS Build Tools, 7z or tar.
param(
  [Parameter(Mandatory)] [ValidateSet('libmpv', 'deps')] [string]$Step,
  [string]$Assets = '',
  [string]$Tag = '',
  [switch]$Publish
)
$ErrorActionPreference = 'Stop'
$repo = 'vestrasys/vestra-player'
$packaging = Split-Path -Parent $PSScriptRoot
$pins = Get-Content (Join-Path $packaging 'deps.json') -Raw | ConvertFrom-Json
$mpv = $pins.upstream.'windows-mpv'
function Sha($path) { (Get-FileHash $path -Algorithm SHA256).Hash.ToLower() }
function Run($exe, [string[]]$argv) {
  "> $exe $($argv -join ' ')"
  if ($Publish) { & $exe @argv; if ($LASTEXITCODE -ne 0) { throw "$exe failed" } }
}

if ($Step -eq 'libmpv') {
  if (-not $Assets) { throw '-Assets <dir> with the mpv-dev zip and the MSYS2 package tar' }
  $zip = Join-Path $Assets ([IO.Path]::GetFileName(([Uri]$mpv.url).AbsolutePath))
  $tar = Join-Path $Assets ([IO.Path]::GetFileName(([Uri]$mpv.msys2_packages).AbsolutePath))
  if ((Sha $zip) -ne $mpv.sha256) { throw "$zip does not match deps.json's sha256" }
  if ((Sha $tar) -ne $mpv.msys2_packages_sha256) { throw "$tar does not match deps.json's msys2_packages_sha256" }
  $tag = ([Uri]$mpv.url).Segments[-2].TrimEnd('/')
  Run gh @('release', 'create', $tag, '--repo', $repo, '--prerelease', '--title', "libmpv $tag (FFmpeg $($mpv.ffmpeg), built by src/packaging/libmpv)",
    '--notes', "libmpv-2.dll built by src/packaging/libmpv/build-windows.sh (mpv $($mpv.mpv_commit), FFmpeg $($mpv.ffmpeg), mpv-lavfi-hwdevice.patch). libmpv-2.dll sha256 $($mpv.libmpv_sha256). The MSYS2 packages it was built with (msys2-packages.lock) are attached for rebuilds after MSYS2 prunes them.",
    $zip, $tar)
  exit 0
}

# -Step deps
$work = Join-Path ([IO.Path]::GetTempPath()) "vestra-deps-local-$(Get-Date -Format yyyyMMddHHmmss)"
$deps = Join-Path $work 'deps'; $out = Join-Path $work 'out'
New-Item -ItemType Directory -Force $deps, $out | Out-Null
node (Join-Path $packaging 'collect-windows-deps.mjs') --out $deps
if ($LASTEXITCODE -ne 0) { throw 'collect-windows-deps.mjs failed' }
$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$bin = "$vs\VC\Tools\MSVC\$((Get-Content "$vs\VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt").Trim())\bin\Hostx64\x64"
$names = & "$bin\dumpbin.exe" /nologo /exports "$deps\libmpv-2.dll" | ForEach-Object { if ($_ -match '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]{8}\s+(\S+)') { $Matches[1] } }
if (-not $names) { throw 'no exports found in libmpv-2.dll' }
Set-Content -Path "$work\mpv.def" -Value (@('LIBRARY libmpv-2', 'EXPORTS') + $names)
& "$bin\lib.exe" /nologo /def:"$work\mpv.def" /machine:x64 /out:"$deps\mpv.lib"
if ($LASTEXITCODE -ne 0) { throw 'lib.exe failed' }
Remove-Item "$deps\mpv.exp" -ErrorAction SilentlyContinue
Push-Location $deps
tar -a -cf (Join-Path $out 'vestra-deps-windows-x64.zip') *
Pop-Location
Copy-Item "$deps\components.json" "$out\components-windows-x64.json"
# The macOS assets of the release deps.json names, unchanged and checked.
foreach ($key in 'macos-arm64', 'macos-arm64-sources') {
  $asset = $pins.assets.$key
  gh release download $pins.release --repo $repo --pattern $asset.file --dir $out
  if ((Sha (Join-Path $out $asset.file)) -ne $asset.sha256) { throw "$($asset.file) does not match deps.json" }
}
try { gh release download $pins.release --repo $repo --pattern 'components-macos-arm64.json' --dir $out } catch { 'no components-macos-arm64.json in the old release' }
if (-not $Tag) { $Tag = "deps-$(Get-Date -Format yyyyMMdd).1" }
$sums = Get-ChildItem $out -File | ForEach-Object { "$(Sha $_.FullName)  $($_.Name)" }
Set-Content -Path "$out\SHA256SUMS" -Value $sums
# PSCustomObjects, not hashtables: Windows PowerShell 5.1's ConvertTo-Json writes a hashtable
# assigned to an object's property as its type name.
$assets = [pscustomobject][ordered]@{
  'macos-arm64' = $pins.assets.'macos-arm64'
  'windows-x64' = [pscustomobject][ordered]@{ file = 'vestra-deps-windows-x64.zip'; sha256 = (Sha "$out\vestra-deps-windows-x64.zip") }
  'macos-arm64-sources' = $pins.assets.'macos-arm64-sources'
}
$newPins = Get-Content (Join-Path $packaging 'deps.json') -Raw | ConvertFrom-Json
$newPins.release = $Tag
$newPins.assets = $assets
"new deps.json (written to $out\deps.json):"
$json = $newPins | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText("$out\deps.json", $json + "`n", (New-Object Text.UTF8Encoding $false))
# The repository's JSON style (two spaces, as Prettier checks it), and a check that it parsed.
node -e "const f=process.argv[1],fs=require('fs');const p=JSON.parse(fs.readFileSync(f,'utf8'));if(typeof p.assets!=='object')throw new Error('assets');fs.writeFileSync(f,JSON.stringify(p,null,2)+String.fromCharCode(10))" "$out\deps.json"
if ($LASTEXITCODE -ne 0) { throw 'deps.json is not valid' }
Get-Content "$out\deps.json" -Raw
Run gh (@('release', 'create', $Tag, '--repo', $repo, '--prerelease', '--title', "Native dependencies $Tag",
  '--notes', "Native dependencies for Vestra Player packages, made on a local machine by src/packaging/libmpv/publish.ps1 (the Windows half of deps.yml; libmpv from $($mpv.url)); macOS assets unchanged from $($pins.release).") + (Get-ChildItem $out -File | Where-Object Name -ne 'deps.json' | ForEach-Object FullName))
if ($Publish) { Copy-Item "$out\deps.json" (Join-Path $packaging 'deps.json'); "deps.json updated: release $Tag" }
