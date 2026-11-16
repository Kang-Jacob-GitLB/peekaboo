# 리허설 준비(호스트에서 실행). Windows Sandbox 에 매핑할 작업 폴더를 만들고 .wsb 를 생성한다.
# 이 스크립트는 설치본을 실행하지 않고 샌드박스도 띄우지 않는다. 마지막에 출력되는 명령을 사람이 직접 실행한다.
# 자세한 절차: docs/tauri-rehearsal.md
#
# 사용:
#   # 1) 옛 Electron → Tauri 이전 리허설
#   .\prepare.ps1 -Work D:\rehearsal -Scenario Migrate -ElectronInstaller <Peekaboo-Setup-0.10.1-x64.exe> -TauriInstaller <Peekaboo-Setup-X-x64.exe>
#      v0.1.0 계열(옛 이름의 시작 시 실행 값)·값 없음·작업 관리자 "사용 안 함" 상태는 -RunValue LegacyName|None, -DisabledInTaskManager 로 고른다.
#      -Work 는 비어 있거나 이 스크립트가 만든 폴더(표식 파일 .peekaboo-rehearsal)여야 한다. in·out·scripts 를 비우므로 아무 폴더나 주지 마라.
#   # 2) Tauri → Tauri 업데이트 리허설(로컬 서버). UpdateDir 에는 v1·v2 설치본과 latest.json 이 있어야 한다.
#   .\prepare.ps1 -Work D:\rehearsal -Scenario Update -UpdateDir <폴더> -OldVersion 0.1.0 -NewVersion 0.2.0
param(
    [Parameter(Mandatory)] [string]$Work,
    [Parameter(Mandatory)] [ValidateSet('Migrate', 'Update')] [string]$Scenario,
    [string]$ElectronInstaller,
    [string]$TauriInstaller,
    [string]$UpdateDir,
    [string]$OldVersion = '0.1.0',
    [string]$NewVersion = '0.2.0',
    # Migrate 시나리오: 옛 Electron 설치본의 시작 시 실행 값. Current = 현재 이름(v0.10 계열), LegacyName = 옛 이름(v0.1.0 계열, -LegacyRunName), None = 원래 없음
    [ValidateSet('Current', 'LegacyName', 'None')] [string]$RunValue = 'Current',
    [string]$LegacyRunName = 'electron.app.Peekaboo',
    # Migrate 시나리오: 작업 관리자 시작 앱에서 "사용 안 함" 으로 바꿔 둔 상태로 시작한다(StartupApproved)
    [switch]$DisabledInTaskManager,
    # 기본은 네트워크 끔: 옛 Electron 앱이 스스로 업데이트를 받으러 가지 않게 한다. WebView2 런타임을 내려받아야 하면 켠다.
    [switch]$EnableNetwork
)
$ErrorActionPreference = 'Stop'

function Copy-Required([string]$From, [string]$To) {
    if (-not $From -or -not (Test-Path -LiteralPath $From -PathType Leaf)) { throw "file not found: $From" }
    Copy-Item -LiteralPath $From -Destination $To -Force
}

$Work = [IO.Path]::GetFullPath($Work).TrimEnd('\')
# 작업 폴더 보호: 아래에서 in·out·scripts 를 비우므로, 이 스크립트가 만든 폴더(표식 파일이 있음)이거나 비어 있는 폴더만 쓴다.
# 저장소 루트나 아무 폴더를 -Work 로 줘서 내용이 지워지는 사고를 막는다. 저장소를 품은 폴더도 거부한다.
$markerName = '.peekaboo-rehearsal'
$marker = Join-Path $Work $markerName
if ($Work -match '^[A-Za-z]:$') { throw "refusing a drive root as -Work: $Work" }
$repoRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)).TrimEnd('\')
if (($repoRoot + '\').StartsWith($Work + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "refusing -Work that contains this repository: $Work" }
if ((Test-Path -LiteralPath $Work) -and -not (Test-Path -LiteralPath $marker) -and @(Get-ChildItem -LiteralPath $Work -Force).Count -gt 0) {
    throw "refusing a non-empty -Work without the '$markerName' marker (not created by this script): $Work"
}
New-Item -ItemType Directory -Force -Path $Work | Out-Null
Set-Content -LiteralPath $marker -Value 'created by scripts/rehearsal/prepare.ps1; its in, out and scripts folders are emptied on every run' -Encoding ASCII
foreach ($sub in 'in', 'out', 'scripts') {
    $dir = Join-Path $Work $sub
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    # 이전 실행의 설치본·스크립트·결과가 섞이지 않게 비운다
    Get-ChildItem -LiteralPath $dir -Force | Remove-Item -Recurse -Force
}

# 샌드박스 안의 Windows PowerShell 5.1 이 한글 주석을 제대로 읽도록 BOM 이 있는 UTF-8 로 복사한다
$utf8Bom = New-Object System.Text.UTF8Encoding($true)
foreach ($name in 'sandbox-migrate.ps1', 'sandbox-update.ps1', 'serve-feed.ps1') {
    $text = [IO.File]::ReadAllText((Join-Path $PSScriptRoot $name), [Text.Encoding]::UTF8)
    [IO.File]::WriteAllText((Join-Path $Work "scripts\$name"), $text, $utf8Bom)
}

switch ($Scenario) {
    'Migrate' {
        Copy-Required $ElectronInstaller (Join-Path $Work 'in\electron-setup.exe')
        Copy-Required $TauriInstaller (Join-Path $Work 'in\tauri-setup.exe')
        $options = [ordered]@{ runValue = $RunValue; legacyRunName = $LegacyRunName; disabledInTaskManager = [bool]$DisabledInTaskManager }
        [IO.File]::WriteAllText((Join-Path $Work 'in\migrate-options.json'), ($options | ConvertTo-Json), (New-Object System.Text.UTF8Encoding($false)))
        $script = 'sandbox-migrate.ps1'
    }
    'Update' {
        $updateIn = Join-Path $Work 'in\update'
        New-Item -ItemType Directory -Force -Path $updateIn | Out-Null
        Copy-Required (Join-Path $UpdateDir "Peekaboo-Setup-$OldVersion-x64.exe") (Join-Path $updateIn "Peekaboo-Setup-$OldVersion-x64.exe")
        # 서버가 내줄 것: 새 설치본과 latest.json. 구버전 설치본은 서버 폴더에 두지 않는다.
        $serve = Join-Path $updateIn 'serve'
        New-Item -ItemType Directory -Force -Path $serve | Out-Null
        Copy-Required (Join-Path $UpdateDir "Peekaboo-Setup-$NewVersion-x64.exe") (Join-Path $serve "Peekaboo-Setup-$NewVersion-x64.exe")
        Copy-Required (Join-Path $UpdateDir 'latest.json') (Join-Path $serve 'latest.json')
        [IO.File]::WriteAllText((Join-Path $Work 'in\update\versions.txt'), "$OldVersion`n$NewVersion`n")
        $script = 'sandbox-update.ps1'
    }
}

$networking = if ($EnableNetwork) { 'Enable' } else { 'Disable' }
$template = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'rehearsal.wsb.template'), [Text.Encoding]::UTF8)
$hostDir = [Security.SecurityElement]::Escape($Work)
$wsb = $template.Replace('{{HOST_DIR}}', $hostDir).Replace('{{NETWORKING}}', $networking).Replace('{{SCRIPT}}', $script)
$wsbPath = Join-Path $Work "rehearsal-$($Scenario.ToLower()).wsb"
[IO.File]::WriteAllText($wsbPath, $wsb, (New-Object System.Text.UTF8Encoding($false)))

# 출력 문구는 BOM 없는 UTF-8 을 ANSI 로 읽는 Windows PowerShell 5.1 에서도 깨지지 않게 ASCII 로 쓴다
Write-Host "Prepared: $Work"
Write-Host "Start the sandbox with (does not touch the apps installed on this PC):"
Write-Host "  WindowsSandbox.exe `"$wsbPath`""
Write-Host "Results: $Work\out\ (migrate-result.json/.txt or update-*.json)"
