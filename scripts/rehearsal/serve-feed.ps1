# 로컬 업데이트 리허설용 HTTP 서버. 폴더의 파일을 http://127.0.0.1:<포트>/ 로 내준다(루프백에만 묶는다).
# sandbox-update.ps1 이 샌드박스 안에서 숨김 창으로 띄운다. 끝내려면 프로세스를 종료한다.
param(
    [Parameter(Mandatory)] [string]$Dir,
    [int]$Port = 8123
)
$ErrorActionPreference = 'Stop'
$Dir = [IO.Path]::GetFullPath($Dir)
$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add("http://127.0.0.1:$Port/")
$listener.Start()
while ($listener.IsListening) {
    $ctx = $listener.GetContext()
    try {
        # 폴더 밖을 가리키는 경로(..)는 파일 이름만 남겨 막는다
        $name = [IO.Path]::GetFileName([Uri]::UnescapeDataString($ctx.Request.Url.AbsolutePath))
        $file = Join-Path $Dir $name
        if ($name -and (Test-Path -LiteralPath $file -PathType Leaf)) {
            $bytes = [IO.File]::ReadAllBytes($file)
            $ctx.Response.ContentType = if ($name -like '*.json') { 'application/json' } else { 'application/octet-stream' }
            $ctx.Response.ContentLength64 = $bytes.Length
            $ctx.Response.OutputStream.Write($bytes, 0, $bytes.Length)
        }
        else {
            $ctx.Response.StatusCode = 404
        }
    }
    catch { $ctx.Response.StatusCode = 500 }
    finally { $ctx.Response.Close() }
}
