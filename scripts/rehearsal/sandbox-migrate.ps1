# 옛 Electron → Tauri 이전 리허설. **Windows Sandbox 안에서만** 돈다(prepare.ps1 이 만든 .wsb 의 LogonCommand).
# 이 스크립트는 Electron 설치본을 설치하고 Tauri 설치본으로 덮어 설치하므로, 실제 PC 에서 실행하면 설치된 Peekaboo 를 지운다.
# 그래서 샌드박스의 기본 사용자(WDAGUtilityAccount)가 아니면 아무것도 하지 않고 끝낸다.
#
# 순서: (a) Electron 설치 + 시작 시 실행(Run) 값 + 설정 파일 + (선택) 실행 → (b) Tauri 설치본 무인 실행(/S) → (c) 검증
# 시나리오 옵션은 in\migrate-options.json(읽기 전용 매핑)에서 읽는다: 시작 시 실행 값의 상태(현재 이름·옛 이름 v0.1.0 계열·없음)와
# 작업 관리자의 "사용 안 함" 상태. 조용한(/S) 설치만 자동으로 본다. 화면이 있는(비조용) 설치는 사람이 해야 해서 docs/tauri-rehearsal.md 의 수동 절차로 둔다.
# 결과: C:\Rehearsal\out\migrate-result.json, migrate-result.txt (호스트의 매핑 폴더 out\ 에 남는다)
# 안내 문구는 Windows PowerShell 5.1 이 코드 페이지와 무관하게 읽도록 ASCII 로 쓴다.
param(
    [string]$Root = 'C:\Rehearsal',
    # Electron 앱을 설치 직후 한 번 띄워, 실행 중인 옛 앱이 있는 상태에서 Tauri 설치가 끝나는지 본다(계획서 6절 4번)
    [bool]$RunElectron = $true,
    [int]$TauriInstallTimeoutSec = 240
)
$ErrorActionPreference = 'Stop'

if ($env:USERNAME -ne 'WDAGUtilityAccount') {
    Write-Host 'REFUSED: this script uninstalls the Electron Peekaboo by design. Run it only inside Windows Sandbox.'
    exit 3
}

$out = Join-Path $Root 'out'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$logPath = Join-Path $out 'migrate-log.txt'
function Log([string]$m) {
    $line = '{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $m
    Add-Content -LiteralPath $logPath -Value $line
    Write-Host $line
}

# 옛 Electron 설치본의 제거 키(electron-builder 가 appId 로 만든 GUID) — docs/tauri-migration.md 6절
$oldKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\a32366e0-3e8a-5400-a683-97d78e557198'
$newKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Peekaboo'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runName = 'io.github.kang-jacob-gitlb.peekaboo'
$oldDir = Join-Path $env:LOCALAPPDATA 'Programs\Peekaboo'
$newDir = Join-Path $env:LOCALAPPDATA 'Peekaboo'
$settingsPath = Join-Path $env:APPDATA 'Peekaboo\settings.json'
$approvedKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$marker = [guid]::NewGuid().ToString()
# 작업 관리자의 "사용 안 함" 상태(StartupApproved\Run, REG_BINARY). 첫 바이트가 홀수(3)면 사용 안 함이다.
[byte[]]$disabledState = 3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8

# 시나리오 옵션(prepare.ps1 -RunValue / -DisabledInTaskManager 가 in\migrate-options.json 으로 넘긴다)
#   runValue: Current = 현재 이름(v0.10 계열)으로 시작 시 실행 값이 있음, LegacyName = 옛 이름(v0.1.0 계열)으로 있음, None = 원래 없음
$options = [ordered]@{ runValue = 'Current'; legacyRunName = 'electron.app.Peekaboo'; disabledInTaskManager = $false }
$optionsPath = Join-Path $Root 'in\migrate-options.json'
if (Test-Path -LiteralPath $optionsPath) {
    foreach ($p in (Get-Content -LiteralPath $optionsPath -Raw | ConvertFrom-Json).PSObject.Properties) { $options[$p.Name] = $p.Value }
}
$seedName = switch ($options.runValue) { 'Current' { $runName } 'LegacyName' { $options.legacyRunName } default { $null } }

$checks = New-Object System.Collections.Generic.List[object]
function Check([string]$name, [bool]$pass, [string]$detail) {
    $checks.Add([ordered]@{ name = $name; pass = $pass; detail = $detail })
    Log ('{0}: {1} - {2}' -f $(if ($pass) { 'PASS' } else { 'FAIL' }), $name, $detail)
}
function Get-RunValue([string]$name = $runName) { (Get-ItemProperty -LiteralPath $runKey -Name $name -ErrorAction SilentlyContinue).$name }
function Get-Approved([string]$name) { (Get-ItemProperty -LiteralPath $approvedKey -Name $name -ErrorAction SilentlyContinue).$name }
function Same-Bytes($a, $b) { $null -ne $a -and $null -ne $b -and (($a -join ',') -eq ($b -join ',')) }
function Get-Shortcuts {
    $roots = @(
        (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'),
        [Environment]::GetFolderPath('Desktop')
    )
    $shell = New-Object -ComObject WScript.Shell
    foreach ($r in $roots) {
        if (-not (Test-Path -LiteralPath $r)) { continue }
        Get-ChildItem -LiteralPath $r -Filter 'Peekaboo*.lnk' -Recurse -ErrorAction SilentlyContinue | ForEach-Object {
            [pscustomobject]@{ path = $_.FullName; target = $shell.CreateShortcut($_.FullName).TargetPath; startMenu = $_.FullName.StartsWith($roots[0]) }
        }
    }
}
function Get-PeekabooProcesses {
    Get-CimInstance Win32_Process -Filter "Name='Peekaboo.exe' OR Name='peekaboo-helper.exe'" -ErrorAction SilentlyContinue |
        ForEach-Object { [pscustomobject]@{ name = $_.Name; pid = $_.ProcessId; path = $_.ExecutablePath } }
}
function Run-Installer([string]$path, [string[]]$arguments, [int]$timeoutSec) {
    $p = Start-Process -FilePath $path -ArgumentList $arguments -PassThru
    if (-not $p.WaitForExit($timeoutSec * 1000)) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        return [pscustomobject]@{ exitCode = $null; timedOut = $true }
    }
    return [pscustomobject]@{ exitCode = $p.ExitCode; timedOut = $false }
}

$result = [ordered]@{ startedAt = (Get-Date).ToString('o'); marker = $marker; steps = [ordered]@{}; checks = $null; passed = $false }
try {
    $electron = Join-Path $Root 'in\electron-setup.exe'
    $tauri = Join-Path $Root 'in\tauri-setup.exe'
    foreach ($f in $electron, $tauri) { if (-not (Test-Path -LiteralPath $f)) { throw "missing installer: $f" } }

    # (a) 옛 Electron 설치(현재 사용자) + 시작 시 실행 + 설정 파일
    Log 'A: installing Electron Peekaboo (/S /currentuser)'
    $a = Run-Installer $electron @('/S', '/currentuser') 240
    $result.steps.electronInstall = $a
    if ($a.timedOut -or $a.exitCode -ne 0) { throw "Electron install failed: $($a | ConvertTo-Json -Compress)" }
    if (-not (Test-Path -LiteralPath $oldKey)) { throw 'Electron uninstall key not found after install (HKCU GUID key)' }
    $oldExe = Join-Path $oldDir 'Peekaboo.exe'
    if (-not (Test-Path -LiteralPath $oldExe)) { throw "Electron exe not found: $oldExe" }
    if ($seedName) {
        Set-ItemProperty -LiteralPath $runKey -Name $seedName -Value ('"{0}"' -f $oldExe)
        if ($options.disabledInTaskManager) {
            New-Item -Path $approvedKey -Force | Out-Null
            Set-ItemProperty -LiteralPath $approvedKey -Name $seedName -Value $disabledState -Type Binary
        }
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $settingsPath) | Out-Null
    [IO.File]::WriteAllText($settingsPath, ('{{"hotkeyEnabled":false,"rehearsalMarker":"{0}"}}' -f $marker), (New-Object System.Text.UTF8Encoding($false)))
    $result.steps.beforeTauri = [ordered]@{
        oldKeyPresent = [bool](Test-Path -LiteralPath $oldKey)
        scenario = $options
        runValue = if ($seedName) { Get-RunValue $seedName } else { $null }
        shortcuts = @(Get-Shortcuts)
    }
    if ($RunElectron) {
        Log 'A: starting Electron Peekaboo (installer must cope with a running old app)'
        Start-Process -FilePath $oldExe
        Start-Sleep -Seconds 8
        $procs = @(Get-PeekabooProcesses)
        $result.steps.beforeTauri.processes = $procs
        # 옛 앱이 실제로 떠 있어야 "실행 중인 앱이 있어도 설치가 끝난다"는 검증이 의미가 있다
        $runningOld = @($procs | Where-Object { $_.path -and $_.path.StartsWith($oldDir, 'OrdinalIgnoreCase') })
        Check 'old Electron Peekaboo is running right before the Tauri installer' ($runningOld.Count -ge 1) ($procs | ConvertTo-Json -Compress)
    }

    # (b) Tauri 설치본 무인 실행
    Log 'B: running Tauri installer (/S)'
    $b = Run-Installer $tauri @('/S') $TauriInstallTimeoutSec
    $result.steps.tauriInstall = $b
    Start-Sleep -Seconds 3
    Check 'tauri installer finished in time with exit code 0' ((-not $b.timedOut) -and ($b.exitCode -eq 0)) ($b | ConvertTo-Json -Compress)

    # (c) 검증
    Check 'old Electron uninstall key is gone' (-not (Test-Path -LiteralPath $oldKey)) $oldKey
    Check 'old install folder is gone' (-not (Test-Path -LiteralPath $oldDir)) $oldDir
    $newExe = Join-Path $newDir 'Peekaboo.exe'
    Check 'new install exists' (Test-Path -LiteralPath $newExe) $newExe
    $newInstallLocation = (Get-ItemProperty -LiteralPath $newKey -ErrorAction SilentlyContinue).InstallLocation
    Check 'new uninstall key points at new folder' ([bool]$newInstallLocation -and ($newInstallLocation.Trim('"') -ieq $newDir)) "InstallLocation=$newInstallLocation"
    $run = Get-RunValue
    if ($seedName) {
        Check 'Run value points at the new exe (current value name)' ($run -ieq ('"{0}"' -f $newExe)) "Run=$run"
        if ($seedName -ne $runName) {
            Check 'old-name Run value is gone' ($null -eq (Get-RunValue $seedName)) "$seedName=$(Get-RunValue $seedName)"
        }
        if ($options.disabledInTaskManager) {
            Check 'Task Manager disabled state carried over to the current value name' (Same-Bytes (Get-Approved $runName) $disabledState) "StartupApproved=$((Get-Approved $runName) -join ',')"
            if ($seedName -ne $runName) {
                Check 'old-name StartupApproved value is gone' ($null -eq (Get-Approved $seedName)) "$seedName=$((Get-Approved $seedName) -join ',')"
            }
        }
    }
    else {
        # 옛 설치본이 시작 시 실행에 등록돼 있지 않았으면 새로 만들지 않는다
        Check 'no Run value was created (there was none before)' ($null -eq $run) "Run=$run"
    }
    $settingsOk = (Test-Path -LiteralPath $settingsPath) -and ((Get-Content -LiteralPath $settingsPath -Raw) -like "*$marker*")
    Check 'settings.json kept' $settingsOk $settingsPath
    $links = @(Get-Shortcuts)
    $menuLinks = @($links | Where-Object { $_.startMenu })
    Check 'exactly one Start Menu shortcut' ($menuLinks.Count -eq 1) ($links | ConvertTo-Json -Compress)
    Check 'no shortcut targets the old exe' (@($links | Where-Object { $_.target -ieq $oldExe }).Count -eq 0) ''
    $procs = @(Get-PeekabooProcesses)
    $result.steps.processesAfter = $procs
    Check 'no process runs from the old folder' (@($procs | Where-Object { $_.path -and $_.path.StartsWith($oldDir, 'OrdinalIgnoreCase') }).Count -eq 0) ($procs | ConvertTo-Json -Compress)
}
catch {
    Log ('ERROR: ' + $_.Exception.Message)
    $checks.Add([ordered]@{ name = 'rehearsal ran to completion'; pass = $false; detail = $_.Exception.Message })
}

$result.checks = $checks
$result.passed = (@($checks | Where-Object { -not $_.pass }).Count -eq 0) -and ($checks.Count -gt 0)
$result.finishedAt = (Get-Date).ToString('o')
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $out 'migrate-result.json') -Encoding UTF8
$lines = @("overall: $(if ($result.passed) { 'PASS' } else { 'FAIL' })") + ($checks | ForEach-Object { '{0}: {1} - {2}' -f $(if ($_.pass) { 'PASS' } else { 'FAIL' }), $_.name, $_.detail })
$lines | Set-Content -LiteralPath (Join-Path $out 'migrate-result.txt') -Encoding UTF8
Log 'done. Results are in the mapped folder out\.'
