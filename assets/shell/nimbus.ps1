# Nimbus 终端的 shell 集成（PowerShell）：用 OSC 133 把每条命令的位置告诉终端，用 OSC 7 报当前目录。
#   A 提示符开始  B 提示符结束、输入开始  C 命令开始执行  D;退出码 命令结束
# 终端靠它做：跳到上一条/下一条命令、复制上一条命令的输出、命令历史、更准的「命令跑完」通知；
# 记住当前目录（PowerShell 的 cd 不改进程的当前目录，终端问进程问不准）。
# Windows PowerShell 5.1 和 PowerShell 7 都能用（5.1 的字符串里没有 `e，转义字符一律写 [char]27）。
# 终端在 Windows 上起 PowerShell 时，会在配置文件之后自动加载它；也可以手动在 $PROFILE 里加一行：. 这个文件的路径
if ($global:__NimbusShellIntegration) { return }
$global:__NimbusShellIntegration = $true

# 原来的提示符：配置文件里定义的、主题装的，照样显示
$global:__NimbusPrompt = $function:prompt
# 上一个提示符之后跑过命令（发过 C），下一个提示符要先发 D
$global:__NimbusRan = $false
# 包起来之前的 PSConsoleHostReadLine
$global:__NimbusReadLine = $null
# 还没出过提示符：终端用 -Command 加载这个脚本时，那条命令会记进 Get-History，第一个提示符时删掉
$global:__NimbusFirstPrompt = $true

# 上一条命令的退出码：成功是 0；原生程序失败报它自己的退出码；PowerShell 自己的错误（cmdlet 报错、throw）报 1
function global:__NimbusExitCode([bool] $succeeded, $nativeCode) {
    if ($succeeded) { return 0 }
    $last = Get-History -Count 1
    $record = $null
    if ($global:Error.Count -gt 0) { $record = $global:Error[0] }
    # 最近一条错误是这条命令里出的：PowerShell 自己的错误。原生程序退出码不为 0 的那种（7.3 起可以开）不算
    $fromPowerShell = $record -is [System.Management.Automation.ErrorRecord] -and
        $null -ne $last -and $null -ne $record.InvocationInfo -and
        $record.InvocationInfo.HistoryId -eq $last.Id -and
        $record.FullyQualifiedErrorId -notlike 'ProgramExitedWithNonZeroCode*'
    if (-not $fromPowerShell -and $nativeCode -is [int] -and $nativeCode -ne 0) { return $nativeCode }
    return 1
}

# 当前目录写成 OSC 7：file://主机/C:/路径，每一段按 UTF-8 百分号编码。
# 只报文件系统里带盘符的目录：注册表这类位置不是目录，网络路径（\\服务器\共享）不报
function global:__NimbusLocation {
    $location = $ExecutionContext.SessionState.Path.CurrentLocation
    if ($location.Provider.Name -ne 'FileSystem') { return '' }
    $path = $location.ProviderPath
    if ($path -notmatch '^[A-Za-z]:') { return '' }
    $segments = $path.TrimEnd('\').Split('\')
    $encoded = @()
    for ($index = 1; $index -lt $segments.Length; $index++) {
        $encoded += [Uri]::EscapeDataString($segments[$index])
    }
    $uri = 'file://' + [Environment]::MachineName + '/' + $segments[0] + '/' + ($encoded -join '/')
    return [string][char]27 + ']7;' + $uri + [char]7
}

# 命令开始执行（C）：包住 PSConsoleHostReadLine（PSReadLine 读一行输入的入口），读到非空的一行、交给 PowerShell 执行之前发。
# 比改回车键好：用户自己绑的回车照样用，没输完的多行输入（回车只是换行）不会误发。
# 没有 PSReadLine（比如开着屏幕阅读器时 5.1 不加载它）就没有 C 和 D，只有提示符
function global:__NimbusHookReadLine {
    if ($null -ne $global:__NimbusReadLine) { return }
    $original = Get-Command PSConsoleHostReadLine -CommandType Function -ErrorAction Ignore
    if ($null -eq $original) { return }
    $global:__NimbusReadLine = $original.ScriptBlock
    function global:PSConsoleHostReadLine {
        # 新一点的 PSReadLine 一进来先读 $?（上一条命令成没成功，提示符变色用）：原样交给它
        $succeeded = $global:?
        Set-StrictMode -Off
        if (-not $succeeded) { Write-Error 'nimbus' -ErrorAction Ignore }
        $line = & $global:__NimbusReadLine
        if (-not [string]::IsNullOrWhiteSpace($line)) {
            $global:__NimbusRan = $true
            [Console]::Write([string][char]27 + ']133;C' + [char]7)
        }
        $line
    }
}

function global:prompt {
    # 第一件事：记下上一条命令成没成功，之后跑的任何东西都会改掉 $?
    $succeeded = $global:?
    $nativeCode = $global:LASTEXITCODE
    Set-StrictMode -Off
    $marks = ''
    try {
        if ($global:__NimbusRan) {
            $marks += [string][char]27 + ']133;D;' + (__NimbusExitCode $succeeded $nativeCode) + [char]7
        }
        $marks += [string][char]27 + ']133;A' + [char]7 + (__NimbusLocation)
        # PSReadLine 在加载这个脚本之后才加载的话，这时候再包
        __NimbusHookReadLine
        if ($global:__NimbusFirstPrompt) {
            $global:__NimbusFirstPrompt = $false
            Get-History | Where-Object { $_.CommandLine -like '*ScriptBlock*nimbus.ps1*' } |
                ForEach-Object { Clear-History -Id $_.Id }
        }
    } catch { }
    $global:__NimbusRan = $false
    # 直接写出去：原来的提示符可能先用 Write-Host 画一部分，A 要在它们前面
    [Console]::Write($marks)
    $text = $null
    try {
        # 原来的提示符看到的 $? 要和刚才一样（有的主题按它变色）
        if (-not $succeeded) { Write-Error 'nimbus' -ErrorAction Ignore }
        $text = & $global:__NimbusPrompt
    } catch { }
    $text = @($text) -join ''
    # 和 PowerShell 自己一样：提示符是空的（或者出错了）就显示 PS>
    if ($text -eq '') { $text = 'PS>' }
    return $text + [char]27 + ']133;B' + [char]7
}

__NimbusHookReadLine
