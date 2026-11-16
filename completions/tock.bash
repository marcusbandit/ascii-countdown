# bash completion for tock. Install as
# ~/.local/share/bash-completion/completions/tock

_tock() {
  local cur prev
  cur=${COMP_WORDS[COMP_CWORD]}
  prev=${COMP_WORDS[COMP_CWORD-1]}

  case $prev in
    -t | --title) return 0 ;;
  esac

  if [[ $cur == -* ]]; then
    COMPREPLY=($(compgen -W "--title --up --no-bell --help --version" -- "$cur"))
    return 0
  fi
  # Every word that is not an option is another clock.
  COMPREPLY=($(compgen -W "stopwatch rounds 5m 10m 15m 25m 45m 1h 1h30m 90s" -- "$cur"))
  return 0
}

complete -F _tock tock
