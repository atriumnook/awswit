export AWSWIT_SHELL=zsh
awswit() {
    local output
    local exit_code

    # Run awswit binary and capture output
    output=$(command awswit "$@")
    exit_code=$?

    if [ $exit_code -ne 0 ]; then
        printf '%s\n' "$output" >&2
        return $exit_code
    fi

    # Parse and export variables from output
    while IFS='=' read -r key value; do
        case "$key" in
            AWS_ACCESS_KEY_ID|AWS_SECRET_ACCESS_KEY|AWS_SESSION_TOKEN|AWS_REGION|AWS_DEFAULT_REGION|AWS_PROFILE|AWS_DEFAULT_PROFILE|AWSWIT_PROFILE|AWSWIT_EXPIRATION)
                if [ -n "$value" ]; then
                    export "$key=$value"
                else
                    unset "$key"
                fi
                ;;
            AWSWIT_UNSET)
                unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN
                unset AWS_REGION AWS_DEFAULT_REGION
                unset AWS_PROFILE AWS_DEFAULT_PROFILE
                unset AWSWIT_PROFILE AWSWIT_EXPIRATION
                ;;
            *)
                # Print non-variable output
                [ -n "$key" ] && printf '%s\n' "$key${value:+=$value}"
                ;;
        esac
    done <<< "$output"
}

# Register zsh completion for the awswit function
if type compdef &>/dev/null; then
    compdef _awswit_rs awswit
fi
