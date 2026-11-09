# Nimbus 终端的 shell 集成（bash）：用 OSC 133 把每条命令的位置告诉终端。
# 在 ~/.bashrc 里加一行：source 这个文件的路径
[[ -n $NIMBUS_SHELL_INTEGRATION_LOADED ]] && return 0
[[ $- == *i* ]] || return 0
NIMBUS_SHELL_INTEGRATION_LOADED=1

__nimbus_ran=
__nimbus_prompt() {
  local ret=$?
  if [[ -n $__nimbus_ran ]]; then
    printf '\e]133;D;%s\a' "$ret"
  fi
  __nimbus_ran=
  printf '\e]133;A\a'
  return $ret
}
__nimbus_preexec() {
  # 一条命令（管道）只在第一段报一次；提示符函数自己不算
  if [[ -z $__nimbus_ran && $BASH_COMMAND != __nimbus_prompt* ]]; then
    __nimbus_ran=1
    printf '\e]133;C\a'
  fi
}
PROMPT_COMMAND="__nimbus_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
[[ $PS1 == *'133;B'* ]] || PS1="${PS1}\[\e]133;B\a\]"
trap '__nimbus_preexec' DEBUG
