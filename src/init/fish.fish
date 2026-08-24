# awswit shell integration for Fish.
# Generated profile data is parsed as data; it is never evaluated.
function __awswit_patch_error
    echo 'awswit: rejected an invalid activation response; environment unchanged' >&2
    return 1
end

function __awswit_apply_patch
    set -l lines $argv
    if test (count $lines) -eq 0
        __awswit_patch_error
        return 1
    end

    set -l kind
    switch $lines[1]
        case 'AWSWIT-PATCH 1 ACTIVATE'
            set kind activate
        case 'AWSWIT-PATCH 1 UNSET'
            set kind unset
        case '*'
            __awswit_patch_error
            return 1
    end

    set -l seen
    set -l record_count 0
    set -l committed 0
    set -l profile profile_default profile_awswit
    set -l region_op region_value default_region_op default_region_value

    for line in $lines[2..-1]
        if test $committed -eq 1
            __awswit_patch_error
            return 1
        end
        if test "$line" = AWSWIT-COMMIT
            set committed 1
            continue
        end

        set -l key
        set -l value
        if string match -q 'SET *=*' -- "$line"
            if test "$kind" != activate
                __awswit_patch_error
                return 1
            end
            set -l record (string replace 'SET ' '' -- "$line")
            set -l pair (string split -m 1 '=' -- "$record")
            if test (count $pair) -ne 2
                __awswit_patch_error
                return 1
            end
            set key "$pair[1]"
            set value "$pair[2]"
            if test -z "$value"; or string match -qr '[\x{0000}-\x{001f}\x{007f}-\x{009f}\x{061c}\x{200e}\x{200f}\x{2028}-\x{202e}\x{2066}-\x{2069}]' -- "$value"
                __awswit_patch_error
                return 1
            end
            switch $key
                case AWS_PROFILE AWS_DEFAULT_PROFILE AWSWIT_PROFILE AWS_REGION AWS_DEFAULT_REGION AWS_CONFIG_FILE AWS_SHARED_CREDENTIALS_FILE
                case '*'
                    __awswit_patch_error
                    return 1
            end
        else if string match -q 'UNSET *' -- "$line"
            set key (string replace 'UNSET ' '' -- "$line")
            switch $key
                case AWS_PROFILE AWS_DEFAULT_PROFILE AWSWIT_PROFILE AWS_REGION AWS_DEFAULT_REGION AWS_CONFIG_FILE AWS_SHARED_CREDENTIALS_FILE AWS_ACCESS_KEY_ID AWS_ACCESS_KEY AMAZON_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SECRET_KEY AMAZON_SECRET_ACCESS_KEY AWS_SESSION_TOKEN AWS_SECURITY_TOKEN AMAZON_SESSION_TOKEN AWS_WEB_IDENTITY_TOKEN_FILE AWS_ROLE_ARN AWS_ROLE_SESSION_NAME AWS_CONTAINER_CREDENTIALS_RELATIVE_URI AWS_CONTAINER_CREDENTIALS_FULL_URI AWS_CONTAINER_AUTHORIZATION_TOKEN AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE AWS_EC2_METADATA_SERVICE_ENDPOINT AWS_LOGIN_CACHE_DIRECTORY AWS_CREDENTIAL_PROFILES_FILE AWS_BEARER_TOKEN_BEDROCK
                case '*'
                    __awswit_patch_error
                    return 1
            end
            if test "$kind" = activate
                switch $key
                    case AWS_REGION AWS_DEFAULT_REGION AWS_ACCESS_KEY_ID AWS_ACCESS_KEY AMAZON_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SECRET_KEY AMAZON_SECRET_ACCESS_KEY AWS_SESSION_TOKEN AWS_SECURITY_TOKEN AMAZON_SESSION_TOKEN AWS_WEB_IDENTITY_TOKEN_FILE AWS_ROLE_ARN AWS_ROLE_SESSION_NAME AWS_CONTAINER_CREDENTIALS_RELATIVE_URI AWS_CONTAINER_CREDENTIALS_FULL_URI AWS_CONTAINER_AUTHORIZATION_TOKEN AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE AWS_EC2_METADATA_SERVICE_ENDPOINT AWS_LOGIN_CACHE_DIRECTORY AWS_CREDENTIAL_PROFILES_FILE AWS_BEARER_TOKEN_BEDROCK
                    case '*'
                        __awswit_patch_error
                        return 1
                end
            end
            if test "$kind" = unset
                switch $key
                    case AWS_PROFILE AWS_DEFAULT_PROFILE AWSWIT_PROFILE AWS_REGION AWS_DEFAULT_REGION
                    case '*'
                        __awswit_patch_error
                        return 1
                end
            end
        else
            __awswit_patch_error
            return 1
        end

        if contains -- "$key" $seen
            __awswit_patch_error
            return 1
        end
        set -a seen "$key"
        set record_count (math $record_count + 1)

        switch $line
            case 'SET AWS_PROFILE=*'
                set profile "$value"
            case 'SET AWS_DEFAULT_PROFILE=*'
                set profile_default "$value"
            case 'SET AWSWIT_PROFILE=*'
                set profile_awswit "$value"
            case 'SET AWS_REGION=*'
                set region_op set
                set region_value "$value"
            case 'UNSET AWS_REGION'
                set region_op unset
            case 'SET AWS_DEFAULT_REGION=*'
                set default_region_op set
                set default_region_value "$value"
            case 'UNSET AWS_DEFAULT_REGION'
                set default_region_op unset
        end
    end

    if test $committed -ne 1
        __awswit_patch_error
        return 1
    end
    if test "$kind" = activate
        if test -z "$profile"; or test "$profile" != "$profile_default"; or test "$profile" != "$profile_awswit"
            __awswit_patch_error
            return 1
        end
        if test -z "$region_op"; or test "$region_op" != "$default_region_op"
            __awswit_patch_error
            return 1
        end
        if test "$region_op" = set; and test "$region_value" != "$default_region_value"
            __awswit_patch_error
            return 1
        end
    else
        if test $record_count -ne 5
            __awswit_patch_error
            return 1
        end
        for key in AWS_PROFILE AWS_DEFAULT_PROFILE AWSWIT_PROFILE AWS_REGION AWS_DEFAULT_REGION
            if not contains -- "$key" $seen
                __awswit_patch_error
                return 1
            end
        end
    end

    # The complete frame is valid.  Apply the allow-listed records now.
    for line in $lines[2..-1]
        if string match -q 'SET *=*' -- "$line"
            set -l record (string replace 'SET ' '' -- "$line")
            set -l pair (string split -m 1 '=' -- "$record")
            set -l key "$pair[1]"
            set -l value "$pair[2]"
            set -gx "$key" "$value"
        else if string match -q 'UNSET *' -- "$line"
            set -l key (string replace 'UNSET ' '' -- "$line")
            # Never erase a persistent universal variable.  A zero-element,
            # unexported global shadows it in this shell so child processes
            # still observe the requested UNSET operation.
            set -eg "$key"
            set -g "$key"
        end
    end

    if test "$kind" = activate
        echo 'awswit: AWS profile activated' >&2
    else
        echo 'awswit: AWS profile cleared' >&2
    end
end

function awswit
    set -l output
    set -l exit_code
    set -l before_separator 1

    for argument in $argv
        if test "$argument" = --
            set before_separator 0
            continue
        end
        if test $before_separator -eq 0
            continue
        end
        switch $argument
            case --help -h --version -V
                command awswit $argv
                return $status
        end
    end

    if test (count $argv) -eq 0
        set output (env AWSWIT_HOOK=1 AWSWIT_SHELL=fish awswit activate)
        set exit_code $status
    else
        switch $argv[1]
            case activate unset
                set output (env AWSWIT_HOOK=1 AWSWIT_SHELL=fish awswit $argv)
                set exit_code $status
            case doctor
                env AWSWIT_HOOK=1 AWSWIT_SHELL=fish awswit $argv
                return $status
            case exec list init completions help --help -h --version -V
                command awswit $argv
                return $status
            case '*'
                set output (env AWSWIT_HOOK=1 AWSWIT_SHELL=fish awswit activate $argv)
                set exit_code $status
        end
    end

    if test $exit_code -ne 0
        return $exit_code
    end
    __awswit_apply_patch $output
end

function __awswit_profile_candidates --argument-names location
    for profile in (command awswit list --format completion 2>/dev/null)
        if test "$location" = root; and contains -- "$profile" activate exec list doctor unset init completions help
            continue
        end
        if string match -q -- '-*' "$profile"
            echo -- "--profile=$profile"
        else
            echo -- "$profile"
        end
    end
end

function __awswit_at_root_argument
    test (count (commandline -opc)) -eq 1
end

function __awswit_at_profile_argument
    set -l tokens (commandline -opc)
    test (count $tokens) -eq 2; and contains -- $tokens[2] activate exec
end

complete -c awswit -f -n __awswit_at_root_argument \
    -a '(__awswit_profile_candidates root)' -d 'AWS profile'
complete -c awswit -f -n __awswit_at_profile_argument \
    -a '(__awswit_profile_candidates profile)' -d 'AWS profile'
