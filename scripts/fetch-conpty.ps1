# 取微软开源的新版 ConPTY（conpty.dll 和它起的 OpenConsole.exe），放进 vendor\conpty\
#
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-conpty.ps1              # x64
#   powershell -ExecutionPolicy Bypass -File scripts\fetch-conpty.ps1 -Arch arm64
#
# 系统自带的 ConPTY 会把 Kitty 图片（APC）、Sixel（DCS）整段吃掉，这一份原样转发（见 docs/windows.md）。
# 打包脚本（bundle-windows.ps1）会先跑它，装好的 Nimbus 在程序旁边的 conpty\ 里找它们；
# 调试版的 Nimbus（cargo run、cargo test）直接用这里的。没有这两个文件时用系统自带的，照样能用。
#
# 来源是 nuget.org 上的 Microsoft.Windows.Console.ConPTY（MIT 许可，源码在 github.com/microsoft/terminal）。
# 版本固定：包的 SHA-256 对得上、两个文件都有微软的有效签名才放进来。换版本时改下面两行，
# 两个文件要来自同一个包（它们之间的协议没有版本检查，不配套会出怪问题）
param(
    [ValidateSet('x64', 'arm64')]
    [string]$Arch = 'x64'
)
$Version = '1.25.260930003'
$Sha256 = '02B07B349AF66D801159BDF9E440D4A1CE78BB951F37FC8609731665AFDAE7EE'

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)
$target = 'vendor\conpty'
$stamp = Join-Path $target 'VERSION'
if ((Test-Path $stamp) -and ((Get-Content $stamp -Raw).Trim() -eq "$Version $Arch")) {
    Write-Host "ConPTY 已经是 $Version（$Arch）"
    exit 0
}

# Windows PowerShell 5.1 默认不一定开 TLS 1.2
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$work = Join-Path ([IO.Path]::GetTempPath()) ('nimbus-conpty-' + [Guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
try {
    # 扩展名写成 .zip：5.1 的 Expand-Archive 只认它
    $package = Join-Path $work 'conpty.zip'
    $url = "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/$Version/microsoft.windows.console.conpty.$Version.nupkg"
    Write-Host "下载 ConPTY $Version ……"
    Invoke-WebRequest $url -OutFile $package -UseBasicParsing
    $hash = (Get-FileHash $package -Algorithm SHA256).Hash
    if ($hash -ne $Sha256) {
        throw "包的 SHA-256 对不上（$hash），不用它"
    }
    $extracted = Join-Path $work 'package'
    Expand-Archive $package -DestinationPath $extracted
    $files = [ordered]@{
        'conpty.dll'      = "runtimes\win-$Arch\native\conpty.dll"
        'OpenConsole.exe' = "build\native\runtimes\$Arch\OpenConsole.exe"
    }
    foreach ($name in $files.Keys) {
        $source = Join-Path $extracted $files[$name]
        $signature = Get-AuthenticodeSignature $source
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
            throw "$name 的签名不对（$($signature.Status)），不用它"
        }
    }
    if (Test-Path $target) {
        Remove-Item -Recurse -Force $target
    }
    New-Item -ItemType Directory -Force $target | Out-Null
    foreach ($name in $files.Keys) {
        Copy-Item (Join-Path $extracted $files[$name]) (Join-Path $target $name)
    }
    Copy-Item 'assets\windows\conpty-LICENSE.txt' (Join-Path $target 'LICENSE.txt')
    Set-Content $stamp "$Version $Arch"
    Write-Host "ConPTY $Version（$Arch）放进了 $target"
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
