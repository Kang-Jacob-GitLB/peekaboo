# Tauri → Tauri 업데이트 리허설. **Windows Sandbox 안에서만** 돈다(prepare.ps1 -Scenario Update 가 만든 .wsb 의 LogonCommand).
# 앱의 업데이트 엔드포인트는 빌드할 때 http://127.0.0.1:8123/latest.json 으로 넣어 둔 것이다(docs/tauri-rehearsal.md).
#
# 단계 1(Install, LogonCommand 가 자동 실행):
#   구버전 설치본 설치 → 로컬 서버(serve-feed.ps1) 시작 → 앱 실행 → 앱 로그에 새 버전을 받았다는 줄이 나올 때까지 대기 → 결과 기록
# 그 다음 사람이 샌드박스 창에서 트레이 아이콘 우클릭 > "다시 시작하여 업데이트" 를 누른다(GUI 클릭은 자동화하지 않는다).
# 단계 2(Verify, 샌드박스 안 PowerShell 에서 `C:\Rehearsal\scripts\sandbox-update.ps1 -Phase Verify`):
#   설치된 버전·재실행 여부·시작 시 실행 값·설정 파일을 확인해 결과를 기록
# 결과: C:\Rehearsal\out\update-install-result.json / update-verify-result.json (호스트 매핑 폴더의 out\)
param(
    [ValidateSet('Install', 'Verify')] [string]$Phase = 'Install',
    [string]$Root = 'C:\Rehearsal',
    [int]$Port = 8123
)
$ErrorActionPreference = 'Stop'

if ($env:USERNAME -ne 'WDAGUtilityAccount') {
    Write-Host 'REFUSED: run this only inside Windows Sandbox.'
    exit 3
}

$out = Join-Path $Root 'out'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$logPath = Join-Path $out 'update-log.txt'
function Log([string]$m) {
    $line = '{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $m
    Add-Content -LiteralPath $logPath -Value $line
    Write-Host $line
}

$versions = Get-Content -LiteralPath (Join-Path $Root 'in\update\versions.txt')
$oldVersion = $versions[0]
$newVersion = $versions[1]
$newDir = Join-Path $env:LOCALAPPDATA 'Peekaboo'
$newExe = Join-Path $newDir 'Peekaboo.exe'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Peekaboo'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runName = 'io.github.kang-jacob-gitlb.peekaboo'
$appLog = Join-Path $env:APPDATA 'Peekaboo\logs\peekaboo.log'
$settingsPath = Join-Path $env:APPDATA 'Peekaboo\settings.json'

if ($Phase -eq 'Install') {
    $result = [ordered]@{ startedAt = (Get-Date).ToString('o'); oldVersion = $oldVersion; newVersion = $newVersion }
    try {
        $setup = Join-Path $Root "in\update\Peekaboo-Setup-$oldVersion-x64.exe"
        Log "installing old version $oldVersion (/S)"
        $p = Start-Process -FilePath $setup -ArgumentList '/S' -PassThru
        if (-not $p.WaitForExit(240000)) { throw 'old installer timed out' }
        if ($p.ExitCode -ne 0) { throw "old installer exit code $($p.ExitCode)" }
        # 시작 시 실행과 설정 파일이 업데이트를 거쳐 유지되는지 보려고 미리 만들어 둔다
        Set-ItemProperty -LiteralPath $runKey -Name $runName -Value ('"{0}"' -f $newExe)
        New-Item -ItemType Directory -Force -Path (Split-Path $settingsPath) | Out-Null
        $marker = [guid]::NewGuid().ToString()
        [IO.File]::WriteAllText($settingsPath, ('{{"hotkeyEnabled":false,"rehearsalMarker":"{0}"}}' -f $marker), (New-Object System.Text.UTF8Encoding($false)))
        $result.marker = $marker

        Log "starting local feed server on 127.0.0.1:$Port"
        Start-Process -FilePath 'powershell.exe' -WindowStyle Hidden -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $Root 'scripts\serve-feed.ps1'),
            '-Dir', (Join-Path $Root 'in\update\serve'), '-Port', $Port)
        Start-Sleep -Seconds 3
        $feed = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/latest.json"
        $result.feedStatus = $feed.StatusCode

        Log 'starting the app (first update check runs 10 s after start)'
        Start-Process -FilePath $newExe
        $deadline = (Get-Date).AddSeconds(120)
        $downloaded = $false
        while ((Get-Date) -lt $deadline -and -not $downloaded) {
            Start-Sleep -Seconds 3
            if (Test-Path -LiteralPath $appLog) {
                $downloaded = [bool](Select-String -LiteralPath $appLog -Pattern "downloaded $([regex]::Escape($newVersion))" -Quiet)
            }
        }
        $result.downloadedLogLine = $downloaded
        if (Test-Path -LiteralPath $appLog) { $result.appLogTail = @(Get-Content -LiteralPath $appLog -Tail 15) }
        Log "download logged: $downloaded"
    }
    catch {
        Log ('ERROR: ' + $_.Exception.Message)
        $result.error = $_.Exception.Message
    }
    $result.finishedAt = (Get-Date).ToString('o')
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $out 'update-install-result.json') -Encoding UTF8
    Log 'NEXT: tray icon > right click > update (Korean menu item with the new version), then run this script with -Phase Verify.'
    exit 0
}

# Verify
$checks = New-Object System.Collections.Generic.List[object]
function Check([string]$name, [bool]$pass, [string]$detail) {
    $checks.Add([ordered]@{ name = $name; pass = $pass; detail = $detail })
    Log ('{0}: {1} - {2}' -f $(if ($pass) { 'PASS' } else { 'FAIL' }), $name, $detail)
}
$installed = (Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue).DisplayVersion
Check "installed version is $newVersion" ($installed -eq $newVersion) "DisplayVersion=$installed"
$run = (Get-ItemProperty -LiteralPath $runKey -Name $runName -ErrorAction SilentlyContinue).$runName
Check 'Run value still points at the install' ($run -ieq ('"{0}"' -f $newExe)) "Run=$run"
$running = @(Get-CimInstance Win32_Process -Filter "Name='Peekaboo.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -ieq $newExe })
Check 'app was relaunched after the update (installer /R)' ($running.Count -ge 1) "processes=$($running.Count)"
$helper = @(Get-CimInstance Win32_Process -Filter "Name='peekaboo-helper.exe'" -ErrorAction SilentlyContinue)
Check 'exactly one helper process' ($helper.Count -eq 1) "helpers=$($helper.Count)"
Check 'settings.json kept' ((Test-Path -LiteralPath $settingsPath) -and ((Get-Content -LiteralPath $settingsPath -Raw) -like '*rehearsalMarker*')) $settingsPath
$installLog = if (Test-Path -LiteralPath $appLog) { @(Get-Content -LiteralPath $appLog -Tail 40) } else { @() }
$result = [ordered]@{ checks = $checks; passed = (@($checks | Where-Object { -not $_.pass }).Count -eq 0); appLogTail = $installLog }
$result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $out 'update-verify-result.json') -Encoding UTF8
Log "overall: $(if ($result.passed) { 'PASS' } else { 'FAIL' })"
