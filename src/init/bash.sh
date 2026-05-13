export AWSWIT_SHELL=bash

# True iff `$@` contains a token that means "the binary's own output is for
# the user, not for `eval`" — any subcommand, list/help/version flag, or
# completion-fast-path flag. Scans every arg so flag order doesn't matter:
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

# Bash tab completion: profile names + subcommands.
#
# IMPORTANT: profile names are read into an array and quoted via `printf %q`
# before being handed to `compgen -W`. `compgen -W "$WORDLIST"` interprets
# its argument as a shell wordlist and performs `$(...)`, `` `...` ``, and
# variable expansion on it. Although the binary already filters out
# shell-unsafe profile names (see fast_profile_names() in src/config/),
# we re-quote here as defense in depth: a future change to that filter
# must not silently turn into a shell-injection hole through a hostile
# ~/.aws/config.
_awswit_complete() {
    local cur prev subs
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    subs="exec pick which doctor prompt init completions"

    local -a _awswit_profiles=()
    while IFS= read -r line; do
        _awswit_profiles+=("$line")
    done < <(command awswit -l --names-only 2>/dev/null)
    local quoted=""
    if [ ${#_awswit_profiles[@]} -gt 0 ]; then
        printf -v quoted '%q ' "${_awswit_profiles[@]}"
    fi

    if [ "$COMP_CWORD" -eq 1 ]; then
        COMPREPLY=( $(compgen -W "${subs} ${quoted}" -- "${cur}") )
    elif [ "$COMP_CWORD" -eq 2 ] && [ "${prev}" = "exec" ]; then
        COMPREPLY=( $(compgen -W "${quoted}" -- "${cur}") )
    else
        COMPREPLY=()
    fi
    return 0
}
complete -F _awswit_complete awswit
