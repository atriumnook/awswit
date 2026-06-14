export AWSWIT_SHELL=zsh

# True iff `$@` contains a token that means "the binary's own output is for
# the user, not for `eval`". Scans every arg so flag order doesn't matter:
# `awswit -l --json` and `awswit --json -l` both bypass the eval path.
_awswit_is_info() {
    local a
    for a in "$@"; do
        case "$a" in
            exec|pick|which|doctor|init|completions|prompt|help|\
            -h|--help|-v|--version|-l|--list|--json|--names-only|\
            -s|--shell-export)
                return 0
                ;;
        esac
    done
    return 1
}

awswit() {
    if _awswit_is_info "$@"; then
        command awswit "$@"
        return
    fi
    local _out _rc
    _out=$(command awswit --shell-export "$@")
    _rc=$?
    [ $_rc -eq 0 ] && [ -n "$_out" ] && eval "$_out"
    return $_rc
}

# Zsh tab completion: profile names + subcommands.
_awswit() {
    local -a subs profiles
    subs=(exec pick which doctor prompt init completions help)
    profiles=(${(f)"$(command awswit -l --names-only 2>/dev/null)"})

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
