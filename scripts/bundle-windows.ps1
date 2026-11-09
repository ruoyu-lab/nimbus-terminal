# 打包成 Windows 的安装程序，放在 <target>\release\bundle\ 下面
#
#   powershell -ExecutionPolicy Bypass -File scripts\bundle-windows.ps1                     # NSIS 安装程序（默认）
#   powershell -ExecutionPolicy Bypass -File scripts\bundle-windows.ps1 -Format installer   # 框架自己画的安装程序
#   powershell -ExecutionPolicy Bypass -File scripts\bundle-windows.ps1 -DebugBuild         # 调试版（编得快，带控制台窗口）
#
# 用的是界面框架 Aureole 的打包工具（cargo bundle）。不用另外装：它就在 cargo 拉下来的框架源码里，
# 和 Nimbus 用的是同一个提交，这里从那里编译、运行。它做的事：编译（C 运行库静态编进去，没装 VC++ 运行库的机器也能打开）、
# 往可执行文件的一份拷贝里写图标和包标识符（任务栏分组、系统通知认这个应用标识）、做安装程序。
# 设置在 Cargo.toml 的 [package.metadata.bundle]。
#
# 两种安装程序，都装给当前用户、不要管理员权限，开始菜单的快捷方式带着应用标识（系统通知要它）：
# - nsis：几 MB。要先装 cargo-packager，第一次打包时它还会自己下载 NSIS：
#     cargo install cargo-packager --version 0.11.8 --locked
# - installer：框架画的向导，不用装别的；带着框架，大一些
param(
    [ValidateSet('nsis', 'installer')]
    [string]$Format = 'nsis',
    [switch]$DebugBuild
)

Set-Location (Split-Path -Parent $PSScriptRoot)

function Fail([string]$message) {
    Write-Host "错误：$message" -ForegroundColor Red
    exit 1
}

# 打包工具先编译、后检查 cargo-packager：没装的话要白等一遍发布版的编译，所以这里先看
if ($Format -eq 'nsis') {
    cargo packager --version *> $null
    if ($LASTEXITCODE -ne 0) {
        Fail 'NSIS 安装程序要用 cargo-packager，先装一下：cargo install cargo-packager --version 0.11.8 --locked（或者换 -Format installer）'
    }
}

# 框架源码在哪：依赖图里 aureole 这个包的清单在 crates\aureole 下，打包工具在旁边的 crates\bundle
$metadata = cargo metadata --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    Fail 'cargo metadata 失败'
}
$aureole = $metadata.packages | Where-Object { $_.name -eq 'aureole' } | Select-Object -First 1
if ($null -eq $aureole) {
    Fail '依赖图里没有 aureole'
}
$tool = Join-Path (Split-Path (Split-Path $aureole.manifest_path)) 'bundle\Cargo.toml'
if (-not (Test-Path $tool)) {
    Fail "框架源码里没有打包工具：$tool"
}

$arguments = @('--format', $Format)
if (-not $DebugBuild) {
    $arguments = @('--release') + $arguments
}
# 打包工具编在 target 里单独的目录，不往 cargo 的框架源码目录里写东西；--locked：用框架自己的 Cargo.lock，不改它
cargo run --locked --manifest-path $tool --target-dir (Join-Path $metadata.target_directory 'aureole-bundle') -- @arguments
exit $LASTEXITCODE
