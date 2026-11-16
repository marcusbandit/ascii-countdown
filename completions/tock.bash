# bash completion for tock. Install as
# ~/.local/share/bash-completion/completions/tock

_tock() {
  local cur prev i word target=0 up=0
  cur=${COMP_WORDS[COMP_CWORD]}
  prev=${COMP_WORDS[COMP_CWORD-1]}

  case $prev in
    -t | --title) return 0 ;;
  esac

  # Look at what is already on the line: a mode, a duration, or --up.
  for ((i = 1; i < COMP_CWORD; i++)); do
    word=${COMP_WORDS[i]}
    case $word in
      -t | --title) ((i++)) ;;
      -u | --up) up=1 ;;
      -*) ;;
      *) target=1 ;;
    esac
  done

  if [[ $cur == -* ]]; then
    local opts="--title --no-bell --help --version"
    ((target || up)) || opts="$opts --up"
    COMPREPLY=($(compgen -W "$opts" -- "$cur"))
    return 0
  fi

  if ((!target && !up)); then
    COMPREPLY=($(compgen -W "stopwatch 5m 10m 15m 25m 45m 1h 1h30m 90s" -- "$cur"))
  fi
  return 0
}

complete -F _tock tock
