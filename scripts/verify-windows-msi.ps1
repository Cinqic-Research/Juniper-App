param(
  [Parameter(Mandatory = $true)][string]$MsiPath,
  [Parameter(Mandatory = $true)][string]$ExpectedAppVersion,
  [Parameter(Mandatory = $true)][string]$ExpectedMsiVersion,
  [Parameter(Mandatory = $true)][string]$EvidencePath,
  [Parameter(Mandatory = $true)][string]$PackageEvidencePath
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $MsiPath -PathType Leaf)) {
  throw "MSI not found: $MsiPath"
}

# msiexec resolves paths against its own working directory, not the caller's, so
# a relative path fails with 1619 (ERROR_INSTALL_PACKAGE_OPEN_FAILED). Resolve
# once here and use the absolute path everywhere below.
$MsiPath = (Resolve-Path -LiteralPath $MsiPath).Path

$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember(
  'OpenDatabase',
  'InvokeMethod',
  $null,
  $installer,
  @($MsiPath, 0)
)
$view = $database.GetType().InvokeMember(
  'OpenView',
  'InvokeMethod',
  $null,
  $database,
  @("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = 'ProductVersion'")
)
$view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
$record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
$productVersion = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
if ($productVersion -ne $ExpectedMsiVersion) {
  throw "MSI ProductVersion $productVersion does not equal $ExpectedMsiVersion"
}

$signature = Get-AuthenticodeSignature -LiteralPath $MsiPath
@(
  "ProductVersion=$productVersion"
  "AppVersion=$ExpectedAppVersion"
  "Architecture=x86_64"
  "AuthenticodeStatus=$($signature.Status)"
) | Set-Content -LiteralPath $EvidencePath -Encoding utf8
@(
  "ProductVersion=$productVersion"
  "AppVersion=$ExpectedAppVersion"
  "Architecture=x86_64"
) | Set-Content -LiteralPath $PackageEvidencePath -Encoding utf8

# 0 is success; 3010 is ERROR_SUCCESS_REBOOT_REQUIRED, which is also a
# successful install and must not fail the smoke test.
$install = Start-Process msiexec.exe -ArgumentList @('/i', $MsiPath, '/qn', '/norestart') -Wait -PassThru
if ($install.ExitCode -notin @(0, 3010)) { throw "MSI install failed with $($install.ExitCode)" }

$exe = Get-ChildItem -Path $env:ProgramFiles, ${env:ProgramFiles(x86)} -Filter Juniper.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $exe) { throw 'Installed Juniper.exe was not found.' }
$runtime = Get-ChildItem -Path $exe.Directory.FullName -Filter llama-server.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $runtime) { throw 'Installed Juniper local runtime was not found.' }
# Read the PE optional header from the installed executable. The subsystem is
# a property of the executable crate, not of its linked Rust library.
function Get-PeSubsystem([string]$Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  if ($bytes.Length -lt 0x100) { throw "Invalid PE executable: $Path" }
  $offset = [BitConverter]::ToInt32($bytes, 0x3c)
  if ($offset -lt 0 -or $bytes.Length -lt ($offset + 0x70) -or
      [System.Text.Encoding]::ASCII.GetString($bytes, $offset, 4) -ne "PE`0`0") {
    throw "Invalid PE executable: $Path"
  }
  $optional = $offset + 24
  $magic = [BitConverter]::ToUInt16($bytes, $optional)
  if ($magic -notin @(0x10b, 0x20b)) { throw "Unknown PE optional header magic: $magic" }
  return [BitConverter]::ToUInt16($bytes, $optional + 0x44)
}
$subsystem = Get-PeSubsystem $exe.FullName
if ($subsystem -ne 2) { throw "Installed Juniper.exe has PE subsystem $subsystem; expected Windows GUI (2)." }
# Negative control: the bundled CLI runtime is a console executable and must
# be rejected if substituted for Juniper.exe.
if ((Get-PeSubsystem $runtime.FullName) -ne 3) { throw 'Console-subsystem negative control was unavailable.' }
Add-Content -LiteralPath $EvidencePath -Value 'PESubsystem=WindowsGUI(2); ConsoleNegativeControl=WindowsCUI(3)'
$license = Get-ChildItem -Path $exe.Directory.FullName -Filter LICENSE -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $license) { throw 'Installed Apache-2.0 LICENSE was not found.' }
$notices = Get-ChildItem -Path $exe.Directory.FullName -Filter THIRD_PARTY_NOTICES.md -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $notices) { throw 'Installed third-party notices were not found.' }
# Launch smoke. Surviving a timer is not evidence of a usable window (rc.31
# rendered nothing on Linux while its process stayed alive), so each launch must
# report "[juniper-startup] frontend ready" on stderr, stay alive through a
# settle period, and produce no fatal startup or frontend report. The second
# launch seeds stored state with an enabled Ollama provider backed by a loopback
# stand-in, the state that blanked rc.31, and requires the discovered model to be
# persisted. The surface is captured and checked before and after settling.
$evidenceDirectory = Join-Path (Split-Path -Parent (Resolve-Path -LiteralPath $PackageEvidencePath).Path) 'windows-smoke'
New-Item -ItemType Directory -Force -Path $evidenceDirectory | Out-Null
$regression = Join-Path $PSScriptRoot '..\tests\linux-probe\ollama-startup-regression.py'
$database = Join-Path $env:APPDATA 'com.cinqic.juniper\juniper.db'
$fatalPattern = '\[juniper-startup\] (frontend fatal|stage [^ ]+ failed|fatal:)|panicked at'

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JuniperWindowProbe {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
  [DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr window);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref Point point);
}
'@

function Get-RenderMetrics([System.Drawing.Bitmap]$Bitmap) {
    $colors = New-Object 'System.Collections.Generic.HashSet[int]'
    $black = 0
    $white = 0
    $samples = 0
    for ($y = 40; $y -lt ($Bitmap.Height - 40); $y += 8) {
      for ($x = 40; $x -lt ($Bitmap.Width - 40); $x += 8) {
        $pixel = $Bitmap.GetPixel($x, $y)
        $r = [int]($pixel.R / 32); $g = [int]($pixel.G / 32); $b = [int]($pixel.B / 32)
        [void]$colors.Add(($r -shl 6) -bor ($g -shl 3) -bor $b)
        if ($pixel.R -lt 24 -and $pixel.G -lt 24 -and $pixel.B -lt 24) { $black++ }
        if ($pixel.R -gt 240 -and $pixel.G -gt 240 -and $pixel.B -gt 240) { $white++ }
        $samples++
      }
    }
    return [pscustomobject]@{ Colors = $colors.Count; Black = $black; White = $white; Samples = $samples }
}

function Test-RenderedPixels([System.Drawing.Bitmap]$Bitmap) {
  $metrics = Get-RenderMetrics $Bitmap
  return ($metrics.Colors -ge 8 -and $metrics.Black -le ($metrics.Samples * 0.95) -and
    $metrics.White -le ($metrics.Samples * 0.95))
}

# Negative controls for the visual gate. These must fail before any MSI launch.
foreach ($color in @([System.Drawing.Color]::Black, [System.Drawing.Color]::White)) {
  $blank = [System.Drawing.Bitmap]::new(640, 480)
  try {
    $paint = [System.Drawing.Graphics]::FromImage($blank)
    try { $paint.Clear($color) } finally { $paint.Dispose() }
    if (Test-RenderedPixels $blank) { throw "Visual gate accepted a $color surface" }
  }
  finally { $blank.Dispose() }
}

function Assert-RenderedWindow([System.Diagnostics.Process]$Process, [string]$Label) {
  $Process.Refresh()
  $window = $Process.MainWindowHandle
  if ($window -eq [IntPtr]::Zero -or -not [JuniperWindowProbe]::IsWindowVisible($window)) {
    throw "[$Label] No visible Juniper top-level window"
  }
  if ([JuniperWindowProbe]::IsHungAppWindow($window)) { throw "[$Label] Juniper window is unresponsive" }
  $rect = New-Object JuniperWindowProbe+Rect
  if (-not [JuniperWindowProbe]::GetClientRect($window, [ref]$rect)) { throw "[$Label] Cannot read client area" }
  $width = $rect.Right - $rect.Left
  $height = $rect.Bottom - $rect.Top
  if ($width -lt 400 -or $height -lt 300) { throw "[$Label] Juniper client area is too small: ${width}x${height}" }
  $origin = New-Object JuniperWindowProbe+Point
  if (-not [JuniperWindowProbe]::ClientToScreen($window, [ref]$origin)) { throw "[$Label] Cannot locate client area" }
  $bounds = [System.Windows.Forms.Screen]::FromHandle($window).Bounds
  if ($origin.X -lt $bounds.Left -or $origin.Y -lt $bounds.Top -or
      ($origin.X + $width) -gt $bounds.Right -or ($origin.Y + $height) -gt $bounds.Bottom) {
    throw "[$Label] Juniper client area is outside the visible screen"
  }
  $bitmap = [System.Drawing.Bitmap]::new($width, $height)
  try {
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try { $graphics.CopyFromScreen($origin.X, $origin.Y, 0, 0, $bitmap.Size) }
    finally { $graphics.Dispose() }
    $screenshot = Join-Path $evidenceDirectory "windows-$Label-render.png"
    $bitmap.Save($screenshot, [System.Drawing.Imaging.ImageFormat]::Png)
    if (-not (Test-RenderedPixels $bitmap)) {
      $metrics = Get-RenderMetrics $bitmap
      throw "[$Label] Window appears blank (color bins=$($metrics.Colors), black=$($metrics.Black)/$($metrics.Samples), white=$($metrics.White)/$($metrics.Samples)); see $screenshot"
    }
    Write-Output "[$Label] visible responsive ${width}x${height} window; screenshot=$screenshot"
  }
  finally { $bitmap.Dispose() }
}

function Invoke-LaunchProbe([string]$Label, [int]$TimeoutSeconds = 180, [int]$SettleSeconds = 15) {
  $stdout = Join-Path $evidenceDirectory "windows-$Label-stdout.log"
  $stderr = Join-Path $evidenceDirectory "windows-$Label-stderr.log"
  $launchedAt = Get-Date
  $process = Start-Process -FilePath $exe.FullName -PassThru `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
  try {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
      $log = if (Test-Path -LiteralPath $stderr) { Get-Content -LiteralPath $stderr -Raw } else { '' }
      if ($log -match $fatalPattern) { throw "[$Label] fatal diagnostic before readiness: $($Matches[0])" }
      if ($log -match '\[juniper-startup\] frontend ready') { break }
      if ($process.HasExited) { throw "[$Label] Juniper exited with $($process.ExitCode) before frontend readiness" }
      if ((Get-Date) -gt $deadline) { throw "[$Label] Juniper did not report frontend readiness within ${TimeoutSeconds}s" }
      Start-Sleep -Milliseconds 500
    }
    if ($log -notmatch [regex]::Escape("[juniper-startup] Juniper $ExpectedAppVersion starting on windows/x86_64")) {
      throw "[$Label] Juniper did not report version $ExpectedAppVersion"
    }
    Assert-RenderedWindow $process $Label
    Start-Sleep -Seconds $SettleSeconds
    if ($process.HasExited) { throw "[$Label] Juniper exited with $($process.ExitCode) during the settle period" }
    $log = Get-Content -LiteralPath $stderr -Raw
    if ($log -match $fatalPattern) { throw "[$Label] fatal diagnostic after readiness: $($Matches[0])" }
    Assert-RenderedWindow $process "$Label-settled"
    Write-Output "[$Label] PASS: frontend ready and visibly rendered for ${SettleSeconds}s with no fatal report"
  }
  finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    $process.WaitForExit(30000) | Out-Null
    "ProcessExitCode=$($process.ExitCode)" | Set-Content -LiteralPath (Join-Path $evidenceDirectory "windows-$Label-exit.txt")
    try {
      Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $launchedAt } -ErrorAction Stop |
        Where-Object { $_.Message -match 'Juniper\.exe|msedgewebview2\.exe' } |
        Select-Object TimeCreated, ProviderName, Id, Message |
        Format-List | Out-File -LiteralPath (Join-Path $evidenceDirectory "windows-$Label-events.txt")
    } catch {
      "Application Event Log unavailable: $($_.Exception.GetType().Name)" |
        Set-Content -LiteralPath (Join-Path $evidenceDirectory "windows-$Label-events.txt")
    }
    if (Test-Path -LiteralPath $stderr) {
      Write-Output "--- [$Label] Juniper stderr ---"
      Get-Content -LiteralPath $stderr
    }
  }
}

Invoke-LaunchProbe 'fresh'
if (-not (Test-Path -LiteralPath $database -PathType Leaf)) { throw "Juniper did not create $database" }

$portFile = Join-Path $evidenceDirectory 'windows-ollama-port.txt'
$requestLog = Join-Path $evidenceDirectory 'windows-ollama-requests.log'
Remove-Item -LiteralPath $portFile, $requestLog -ErrorAction SilentlyContinue
$standIn = Start-Process -FilePath python -ArgumentList @($regression, 'serve', $portFile, $requestLog) -PassThru -NoNewWindow
try {
  $deadline = (Get-Date).AddSeconds(30)
  while (-not ((Test-Path -LiteralPath $portFile) -and (Get-Item -LiteralPath $portFile).Length -gt 0)) {
    if ($standIn.HasExited -or (Get-Date) -gt $deadline) { throw 'Ollama stand-in did not start' }
    Start-Sleep -Milliseconds 200
  }
  & python $regression seed $database (Get-Content -LiteralPath $portFile -Raw).Trim()
  if ($LASTEXITCODE -ne 0) { throw 'Could not seed stored Ollama state' }
  Invoke-LaunchProbe 'ollama'
}
finally {
  if (-not $standIn.HasExited) { Stop-Process -Id $standIn.Id -Force }
}
if (-not (Select-String -LiteralPath $requestLog -SimpleMatch 'GET /api/tags' -Quiet)) {
  throw 'Juniper never queried the Ollama stand-in'
}
& python $regression check $database
if ($LASTEXITCODE -ne 0) { throw 'Discovered Ollama model was not persisted' }

$uninstall = Start-Process msiexec.exe -ArgumentList @('/x', $MsiPath, '/qn', '/norestart') -Wait -PassThru
if ($uninstall.ExitCode -notin @(0, 3010)) { throw "MSI uninstall failed with $($uninstall.ExitCode)" }
if (Test-Path -LiteralPath $exe.FullName) { throw 'Juniper.exe remained after uninstall.' }
