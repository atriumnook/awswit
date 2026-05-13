export AWSWIT_SHELL=zsh

awswit() {
    case "$1" in
        exec|which|doctor|init|completions|prompt|help|-h|--help|-v|--version|-l|--list)
            command awswit "$@"
            return
            ;;
    esac
    local _out _rc
    _out=$(command awswit --shell-export "$@")
    _rc=$?
    [ $_rc -eq 0 ] && [ -n "$_out" ] && eval "$_out"
    return $_rc
}

# Zsh tab completion: profile names + subcommands.
_awswit() {
    local -a subs profiles
    subs=(exec which doctor prompt init completions help)
    profiles=(${(f)"$(command awswit -l 2>/dev/null | cut -f1)"})

    if (( CURRENT == 2 )); then
        _alternative "subcommand:subcommand:(${subs})" "profile:profile:(${profiles})"
        return
    fi
    if (( CURRENT == 3 )) && [[ "$words[2]" == exec ]]; then
        compadd -- "${profiles[@]}"
        return
    fi
}
compdef _awswit awswit
