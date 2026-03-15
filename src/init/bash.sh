export AWSWIT_SHELL=bash
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
    while IFS= read -r line; do
        # Split on first '=' only
        key="${line%%=*}"
        value="${line#*=}"
        # If no '=' was found, key equals the whole line and value equals the whole line
        if [ "$key" = "$line" ]; then
            value=""
        fi
        case "$key" in
            AWS_PROFILE|AWS_DEFAULT_PROFILE|AWS_REGION|AWS_DEFAULT_REGION|AWSWIT_PROFILE)
                if [ -n "$value" ]; then
                    export "$key=$value"
                else
                    unset "$key"
                fi
                ;;
            AWSWIT_UNSET)
                unset AWS_PROFILE AWS_DEFAULT_PROFILE
                unset AWS_REGION AWS_DEFAULT_REGION
                unset AWSWIT_PROFILE
                ;;
            *)
                # Print non-variable output
                [ -n "$line" ] && printf '%s\n' "$line"
                ;;
        esac
    done <<< "$output"
}
