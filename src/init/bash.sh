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
