export AWSWIT_SHELL=bash

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

# Bash tab completion: profile names + subcommands.
_awswit_complete() {
    local cur prev subs profiles
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    subs="exec which doctor prompt init completions"

    # Lazily fetch profile names (TSV first column).
    profiles=$(command awswit -l 2>/dev/null | cut -f1)

    if [ "$COMP_CWORD" -eq 1 ]; then
        COMPREPLY=( $(compgen -W "${subs} ${profiles}" -- "${cur}") )
    elif [ "$COMP_CWORD" -eq 2 ] && [ "${prev}" = "exec" ]; then
        COMPREPLY=( $(compgen -W "${profiles}" -- "${cur}") )
    else
        COMPREPLY=()
    fi
    return 0
}
complete -F _awswit_complete awswit
