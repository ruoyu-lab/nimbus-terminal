# Nimbus 终端的 shell 集成（zsh）：用 OSC 133 把每条命令的位置告诉终端。
#   A 提示符开始  B 提示符结束、输入开始  C 命令开始执行  D;退出码 命令结束
# 终端靠它做：跳到上一条/下一条命令、复制上一条命令的输出、命令历史、更准的「命令跑完」通知。
# 终端自己会在本机的 zsh 里加载它；也可以手动在 ~/.zshrc 里加一行：source 这个文件的路径
[[ -n $NIMBUS_SHELL_INTEGRATION_LOADED ]] && return 0
typeset -g NIMBUS_SHELL_INTEGRATION_LOADED=1
autoload -Uz add-zsh-hook

# WSL 里还要报当前目录（OSC 7）：Windows 那边看不到 Linux 里进程的目录。别处终端自己问得到，不报
if [[ -n $WSL_DISTRO_NAME ]]; then
  __nimbus_cwd() {
    emulate -L zsh
    local LC_ALL=C encoded= ch hex
    local -i i
    for (( i = 1; i <= ${#PWD}; i++ )); do
      ch=$PWD[i]
      if [[ $ch == [a-zA-Z0-9/._~-] ]]; then
        encoded+=$ch
      else
        printf -v hex '%02X' "'$ch"
        encoded+="%${hex: -2}"
      fi
    done
    printf '\e]7;file://%s%s\a' "$HOST" "$encoded"
  }
else
  __nimbus_cwd() { :; }
fi

__nimbus_ran=
__nimbus_precmd() {
  local ret=$?
  # 提示符每次重画都可能被主题改掉：确保末尾恰好有一个 B 标记
  if [[ -n $__nimbus_ran ]]; then
    printf '\e]133;D;%s\a' "$ret"
  fi
  __nimbus_ran=
  printf '\e]133;A\a'
  __nimbus_cwd
  if [[ $PS1 != *$'\e]133;B\a'* ]]; then
    PS1="${PS1}%{"$'\e]133;B\a'"%}"
  fi
}
__nimbus_preexec() {
  __nimbus_ran=1
  printf '\e]133;C\a'
}
add-zsh-hook precmd __nimbus_precmd
add-zsh-hook preexec __nimbus_preexec
# 第一个提示符之前就已经在 precmd 里了（我们是在第一次 precmd 时被加载的），补一次
__nimbus_precmd
