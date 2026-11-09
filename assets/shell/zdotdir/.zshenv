# Nimbus 终端的 ZDOTDIR 垫片：先把 ZDOTDIR 还给用户，之后 zsh 读的 .zprofile、.zshrc 都是用户自己的；
# 在用户的配置都跑完之后（第一次画提示符之前）再加载 Nimbus 的 shell 集成。
if [[ -n ${NIMBUS_ORIG_ZDOTDIR+x} ]]; then
  ZDOTDIR=$NIMBUS_ORIG_ZDOTDIR
else
  unset ZDOTDIR
fi
unset NIMBUS_ORIG_ZDOTDIR
if [[ -o interactive && -r ${NIMBUS_ZSH_INTEGRATION:-/nonexistent} ]]; then
  __nimbus_load() {
    precmd_functions=(${precmd_functions:#__nimbus_load})
    unfunction __nimbus_load
    source "$NIMBUS_ZSH_INTEGRATION"
    unset NIMBUS_ZSH_INTEGRATION
  }
  typeset -ga precmd_functions
  precmd_functions+=(__nimbus_load)
fi
# 用户自己的 .zshenv
[[ -r ${ZDOTDIR:-$HOME}/.zshenv ]] && source "${ZDOTDIR:-$HOME}/.zshenv"
true
