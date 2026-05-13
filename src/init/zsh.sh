export AWSWIT_SHELL=zsh
awswit() {
    local _out _rc
    _out=$(command awswit --shell-export "$@")
    _rc=$?
    [ $_rc -eq 0 ] && [ -n "$_out" ] && eval "$_out"
    return $_rc
}

# Forward zsh completions registered for the binary to the wrapper.
if type compdef &>/dev/null; then
    compdef _awswit awswit
fi
