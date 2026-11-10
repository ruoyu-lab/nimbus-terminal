# Nimbus 终端的 shell 集成（bash）：用 OSC 133 把每条命令的位置告诉终端。
# 在 ~/.bashrc 里加一行：source 这个文件的路径
#
# Nimbus 在 WSL 里自己起 bash 时用 `--init-file` 加载这个文件（设了 NIMBUS_BASH_LOGIN），它代替了平常的启动文件：
# 先照登录 shell 的顺序把系统和用户的配置跑一遍，和 `wsl.exe` 默认起的登录 bash 一样
if [[ -n $NIMBUS_BASH_LOGIN ]]; then
  unset NIMBUS_BASH_LOGIN
  [[ -r /etc/profile ]] && . /etc/profile
  for __nimbus_file in ~/.bash_profile ~/.bash_login ~/.profile; do
    if [[ -r $__nimbus_file ]]; then
      . "$__nimbus_file"
      break
    fi
  done
  unset __nimbus_file
fi
[[ -n $NIMBUS_SHELL_INTEGRATION_LOADED ]] && return 0
[[ $- == *i* ]] || return 0
NIMBUS_SHELL_INTEGRATION_LOADED=1

# WSL 里还要报当前目录（OSC 7）：Windows 那边看不到 Linux 里进程的目录。别处终端自己问得到，不报
if [[ -n $WSL_DISTRO_NAME ]]; then
  __nimbus_cwd() {
    local LC_ALL=C path=$PWD encoded= ch hex i
    for (( i = 0; i < ${#path}; i++ )); do
      ch=${path:i:1}
      case $ch in
        [a-zA-Z0-9/._~-]) encoded+=$ch ;;
        *)
          printf -v hex '%02X' "'$ch"
          encoded+="%${hex: -2}"
          ;;
      esac
    done
    printf '\e]7;file://%s%s\a' "$HOSTNAME" "$encoded"
  }
else
  __nimbus_cwd() { :; }
fi

__nimbus_ran=
__nimbus_armed=
__nimbus_prompt() {
  local ret=$?
  __nimbus_armed=
  if [[ -n $__nimbus_ran ]]; then
    printf '\e]133;D;%s\a' "$ret"
  fi
  __nimbus_ran=
  printf '\e]133;A\a'
  __nimbus_cwd
  return $ret
}
# 排在 PROMPT_COMMAND 的最后：提示符要跑的都跑完了，之后 DEBUG 陷阱碰到的才是用户敲的命令
__nimbus_arm() {
  __nimbus_armed=1
}
__nimbus_preexec() {
  # 只认提示符之后的第一条：一条命令（管道）只在第一段报一次；PROMPT_COMMAND 里别人加的命令
  # （比如 systemd 报 OSC 3008 的那条）跑在 __nimbus_arm 之前，不算
  [[ -n $__nimbus_armed && $BASH_COMMAND != __nimbus_prompt* ]] || return 0
  __nimbus_armed=
  __nimbus_ran=1
  printf '\e]133;C\a'
}
# bash 5.1 起 PROMPT_COMMAND 可以是数组（一个元素一条命令）：是数组就前后各加一个元素，不然拼成一串
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
  PROMPT_COMMAND=(__nimbus_prompt "${PROMPT_COMMAND[@]}" __nimbus_arm)
else
  PROMPT_COMMAND="__nimbus_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND};__nimbus_arm"
fi
[[ $PS1 == *'133;B'* ]] || PS1="${PS1}\[\e]133;B\a\]"
trap '__nimbus_preexec' DEBUG
