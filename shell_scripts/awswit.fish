# awswit shell wrapper for Fish
# Add to ~/.config/fish/functions/awswit.fish

function awswit
    set -l output (command awswit $argv)
    set -l exit_code $status

    if test $exit_code -ne 0
        echo $output >&2
        return $exit_code
    end

    for line in $output
        set -l parts (string split '=' $line)
        set -l key $parts[1]
        set -l value $parts[2]

        switch $key
            case AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN AWS_REGION AWS_DEFAULT_REGION AWSWIT_PROFILE AWSWIT_EXPIRATION
                if test -n "$value"
                    set -gx $key $value
                else
                    set -e $key
                end
            case AWSWIT_UNSET
                set -e AWS_ACCESS_KEY_ID
                set -e AWS_SECRET_ACCESS_KEY
                set -e AWS_SESSION_TOKEN
                set -e AWS_REGION
                set -e AWS_DEFAULT_REGION
                set -e AWSWIT_PROFILE
                set -e AWSWIT_EXPIRATION
                set -e AWS_PROFILE
                set -e AWS_DEFAULT_PROFILE
            case '*'
                if test -n "$key"
                    echo $line
                end
        end
    end
end
