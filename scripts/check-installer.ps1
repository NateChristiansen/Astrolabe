# Runs the real installer on a CI machine: the app goes apart from the data folder, and no
# install, reinstall or uninstall touches what is in that folder, even on the old layout.
param([Parameter(Mandatory)][string]$Installer)
$ErrorActionPreference = 'Stop'
$data = Join-Path $env:LOCALAPPDATA 'Astrolabe'
$app = Join-Path $env:LOCALAPPDATA 'AstrolabeApp'
$kept = 'astrolabe.db', 'delivered.jsonl', 'guard-denials.log', 'reports\one.json'

function Fail($why) { Write-Error $why; exit 1 }
function Run($file, $arguments) {
  $run = Start-Process -FilePath $file -ArgumentList $arguments -Wait -PassThru
  if ($run.ExitCode -ne 0) { Fail "$file $arguments exited $($run.ExitCode)" }
}
function CheckKept($when) {
  foreach ($name in $kept) {
    $file = Join-Path $data $name
    if (-not (Test-Path -LiteralPath $file)) { Fail "$when removed $name" }
    if ((Get-Content -Raw -LiteralPath $file).Trim() -ne $name) { Fail "$when changed $name" }
  }
  Write-Output "kept after: $when"
}
function Uninstall($from) {
  Run (Join-Path $from 'uninstall.exe') @('/S', "_?=$from")
  foreach ($exe in 'astrolabe.exe', 'astrolabe-desktop.exe') {
    if (Test-Path -LiteralPath (Join-Path $from $exe)) { Fail "uninstall left $exe in $from" }
  }
}

foreach ($name in $kept) {
  $file = Join-Path $data $name
  New-Item -ItemType Directory -Force -Path (Split-Path $file) | Out-Null
  Set-Content -LiteralPath $file -Value $name
}

Run $Installer @('/S')
foreach ($exe in 'astrolabe.exe', 'astrolabe-desktop.exe') {
  if (-not (Test-Path -LiteralPath (Join-Path $app $exe))) { Fail "$exe is not in $app" }
  if (Test-Path -LiteralPath (Join-Path $data $exe)) { Fail "$exe went into the data folder" }
}
CheckKept 'install'
Run $Installer @('/S', '/UPDATE')
CheckKept 'update'
Uninstall $app
CheckKept 'uninstall'

# The old layout: the same uninstaller, with the app in the data folder.
Run $Installer @('/S', "/D=$data")
CheckKept 'install on the old layout'
Uninstall $data
CheckKept 'uninstall on the old layout'
