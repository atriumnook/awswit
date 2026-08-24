# awswit shell integration for Bash.
# Generated profile data is parsed as data; it is never passed to eval.
_awswit_patch_error() {
    printf '%s\n' 'awswit: rejected an invalid activation response; environment unchanged' >&2
    return 1
}

_awswit_variable_safe() {
    local declaration
    declaration=$(declare -p "$1" 2>/dev/null) || return 0
    case "$declaration" in
        'declare -- '*|'declare -x '*) return 0 ;;
        *) return 1 ;;
    esac
}

_awswit_apply_patch() {
    local frame=$1
    local line state kind record key value seen record_count
    local profile profile_default profile_awswit
    local region_op region_value default_region_op default_region_value

    [ -n "$frame" ] || { _awswit_patch_error; return 1; }
    state=header
    kind=
    seen='|'
    record_count=0
    profile=
    profile_default=
    profile_awswit=
    region_op=
    region_value=
    default_region_op=
    default_region_value=

    while IFS= read -r line || [ -n "$line" ]; do
        case "$state" in
            header)
                case "$line" in
                    'AWSWIT-PATCH 1 ACTIVATE') kind=activate ;;
                    'AWSWIT-PATCH 1 UNSET') kind=unset ;;
                    *) _awswit_patch_error; return 1 ;;
                esac
                state=body
                continue
                ;;
            committed)
                _awswit_patch_error
                return 1
                ;;
        esac

        if [ "$line" = 'AWSWIT-COMMIT' ]; then
            state=committed
            continue
        fi

        case "$line" in
            'SET '*=*)
                [ "$kind" = activate ] || { _awswit_patch_error; return 1; }
                record=${line#SET }
                key=${record%%=*}
                value=${record#*=}
                [ -n "$value" ] || { _awswit_patch_error; return 1; }
                case "$value" in
                    *$'\001'*|*$'\002'*|*$'\003'*|*$'\004'*|*$'\005'*|*$'\006'*|*$'\007'*|*$'\010'*|*$'\011'*|*$'\013'*|*$'\014'*|*$'\015'*|*$'\016'*|*$'\017'*|*$'\020'*|*$'\021'*|*$'\022'*|*$'\023'*|*$'\024'*|*$'\025'*|*$'\026'*|*$'\027'*|*$'\030'*|*$'\031'*|*$'\032'*|*$'\033'*|*$'\034'*|*$'\035'*|*$'\036'*|*$'\037'*|*$'\177'*|*$'\302\200'*|*$'\302\201'*|*$'\302\202'*|*$'\302\203'*|*$'\302\204'*|*$'\302\205'*|*$'\302\206'*|*$'\302\207'*|*$'\302\210'*|*$'\302\211'*|*$'\302\212'*|*$'\302\213'*|*$'\302\214'*|*$'\302\215'*|*$'\302\216'*|*$'\302\217'*|*$'\302\220'*|*$'\302\221'*|*$'\302\222'*|*$'\302\223'*|*$'\302\224'*|*$'\302\225'*|*$'\302\226'*|*$'\302\227'*|*$'\302\230'*|*$'\302\231'*|*$'\302\232'*|*$'\302\233'*|*$'\302\234'*|*$'\302\235'*|*$'\302\236'*|*$'\302\237'*|*$'\330\234'*|*$'\342\200\216'*|*$'\342\200\217'*|*$'\342\200\250'*|*$'\342\200\251'*|*$'\342\200\252'*|*$'\342\200\253'*|*$'\342\200\254'*|*$'\342\200\255'*|*$'\342\200\256'*|*$'\342\201\246'*|*$'\342\201\247'*|*$'\342\201\250'*|*$'\342\201\251'*) _awswit_patch_error; return 1 ;;
                esac
                case "$key" in
                    AWS_PROFILE|AWS_DEFAULT_PROFILE|AWSWIT_PROFILE|AWS_REGION|AWS_DEFAULT_REGION|AWS_CONFIG_FILE|AWS_SHARED_CREDENTIALS_FILE) ;;
                    *) _awswit_patch_error; return 1 ;;
                esac
                ;;
            'UNSET '*)
                key=${line#UNSET }
                value=
                case "$key" in
                    AWS_PROFILE|AWS_DEFAULT_PROFILE|AWSWIT_PROFILE|AWS_REGION|AWS_DEFAULT_REGION|AWS_CONFIG_FILE|AWS_SHARED_CREDENTIALS_FILE|AWS_ACCESS_KEY_ID|AWS_ACCESS_KEY|AMAZON_ACCESS_KEY_ID|AWS_SECRET_ACCESS_KEY|AWS_SECRET_KEY|AMAZON_SECRET_ACCESS_KEY|AWS_SESSION_TOKEN|AWS_SECURITY_TOKEN|AMAZON_SESSION_TOKEN|AWS_WEB_IDENTITY_TOKEN_FILE|AWS_ROLE_ARN|AWS_ROLE_SESSION_NAME|AWS_CONTAINER_CREDENTIALS_RELATIVE_URI|AWS_CONTAINER_CREDENTIALS_FULL_URI|AWS_CONTAINER_AUTHORIZATION_TOKEN|AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE|AWS_EC2_METADATA_SERVICE_ENDPOINT|AWS_LOGIN_CACHE_DIRECTORY|AWS_CREDENTIAL_PROFILES_FILE|AWS_BEARER_TOKEN_BEDROCK) ;;
                    *) _awswit_patch_error; return 1 ;;
                esac
                if [ "$kind" = activate ]; then
                    case "$key" in
                        AWS_REGION|AWS_DEFAULT_REGION|AWS_ACCESS_KEY_ID|AWS_ACCESS_KEY|AMAZON_ACCESS_KEY_ID|AWS_SECRET_ACCESS_KEY|AWS_SECRET_KEY|AMAZON_SECRET_ACCESS_KEY|AWS_SESSION_TOKEN|AWS_SECURITY_TOKEN|AMAZON_SESSION_TOKEN|AWS_WEB_IDENTITY_TOKEN_FILE|AWS_ROLE_ARN|AWS_ROLE_SESSION_NAME|AWS_CONTAINER_CREDENTIALS_RELATIVE_URI|AWS_CONTAINER_CREDENTIALS_FULL_URI|AWS_CONTAINER_AUTHORIZATION_TOKEN|AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE|AWS_EC2_METADATA_SERVICE_ENDPOINT|AWS_LOGIN_CACHE_DIRECTORY|AWS_CREDENTIAL_PROFILES_FILE|AWS_BEARER_TOKEN_BEDROCK) ;;
                        *) _awswit_patch_error; return 1 ;;
                    esac
                fi
                if [ "$kind" = unset ]; then
                    case "$key" in
                        AWS_PROFILE|AWS_DEFAULT_PROFILE|AWSWIT_PROFILE|AWS_REGION|AWS_DEFAULT_REGION) ;;
                        *) _awswit_patch_error; return 1 ;;
                    esac
                fi
                ;;
            *)
                _awswit_patch_error
                return 1
                ;;
        esac

        case "$seen" in
            *"|$key|"*) _awswit_patch_error; return 1 ;;
        esac
        seen="${seen}${key}|"
        record_count=$((record_count + 1))

        case "$line" in
            'SET AWS_PROFILE='*) profile=$value ;;
            'SET AWS_DEFAULT_PROFILE='*) profile_default=$value ;;
            'SET AWSWIT_PROFILE='*) profile_awswit=$value ;;
            'SET AWS_REGION='*) region_op=set; region_value=$value ;;
            'UNSET AWS_REGION') region_op=unset ;;
            'SET AWS_DEFAULT_REGION='*) default_region_op=set; default_region_value=$value ;;
            'UNSET AWS_DEFAULT_REGION') default_region_op=unset ;;
        esac
    done <<< "$frame"

    [ "$state" = committed ] || { _awswit_patch_error; return 1; }
    if [ "$kind" = activate ]; then
        [ -n "$profile" ] && [ "$profile" = "$profile_default" ] && [ "$profile" = "$profile_awswit" ] || {
            _awswit_patch_error
            return 1
        }
        [ -n "$region_op" ] && [ "$region_op" = "$default_region_op" ] || {
            _awswit_patch_error
            return 1
        }
        if [ "$region_op" = set ] && [ "$region_value" != "$default_region_value" ]; then
            _awswit_patch_error
            return 1
        fi
    else
        [ "$record_count" -eq 5 ] || { _awswit_patch_error; return 1; }
        for key in AWS_PROFILE AWS_DEFAULT_PROFILE AWSWIT_PROFILE AWS_REGION AWS_DEFAULT_REGION; do
            case "$seen" in
                *"|$key|"*) ;;
                *) _awswit_patch_error; return 1 ;;
            esac
        done
    fi

    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            'SET '*=*) record=${line#SET }; key=${record%%=*} ;;
            'UNSET '*) key=${line#UNSET } ;;
            *) continue ;;
        esac
        if ! _awswit_variable_safe "$key"; then
            _awswit_patch_error
            return 1
        fi
    done <<< "$frame"

    # Validation is complete.  Only now may the parent environment change.
    state=header
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            'SET '*=*)
                record=${line#SET }
                key=${record%%=*}
                value=${record#*=}
                export "$key=$value"
                ;;
            'UNSET '*)
                key=${line#UNSET }
                unset "$key"
                ;;
        esac
    done <<< "$frame"

    if [ "$kind" = activate ]; then
        printf '%s\n' 'awswit: AWS profile activated' >&2
    else
        printf '%s\n' 'awswit: AWS profile cleared' >&2
    fi
}

awswit() {
    local output exit_code argument before_separator=1

    for argument in "$@"; do
        if [ "$argument" = -- ]; then
            before_separator=0
            continue
        fi
        if [ "$before_separator" -eq 0 ]; then
            continue
        fi
        case "$argument" in
            --help|-h|--version|-V)
                command awswit "$@"
                return $?
                ;;
        esac
    done

    if [ "$#" -eq 0 ]; then
        if output=$(AWSWIT_HOOK=1 AWSWIT_SHELL=bash command awswit activate); then :; else
            exit_code=$?
            return "$exit_code"
        fi
    else
        case "$1" in
            activate|unset)
                if output=$(AWSWIT_HOOK=1 AWSWIT_SHELL=bash command awswit "$@"); then :; else
                    exit_code=$?
                    return "$exit_code"
                fi
                ;;
            doctor)
                AWSWIT_HOOK=1 AWSWIT_SHELL=bash command awswit "$@"
                return $?
                ;;
            exec|list|init|completions|help|--help|-h|--version|-V)
                command awswit "$@"
                return $?
                ;;
            *)
                if output=$(AWSWIT_HOOK=1 AWSWIT_SHELL=bash command awswit activate "$@"); then :; else
                    exit_code=$?
                    return "$exit_code"
                fi
                ;;
        esac
    fi

    _awswit_apply_patch "$output"
}

_awswit_complete() {
    local current previous first candidate completion root_position=0
    current=${COMP_WORDS[COMP_CWORD]-}
    previous=${COMP_WORDS[COMP_CWORD-1]-}
    first=${COMP_WORDS[1]-}

    if declare -F _awswit >/dev/null 2>&1; then
        _awswit awswit "$current" "$previous"
    else
        COMPREPLY=()
    fi

    if [ "$COMP_CWORD" -eq 1 ]; then
        root_position=1
    elif ! { [ "$COMP_CWORD" -eq 2 ] && { [ "$first" = activate ] || [ "$first" = exec ]; }; }; then
        return 0
    fi

    while IFS= read -r candidate; do
        [ -n "$candidate" ] || continue
        if [ "$root_position" -eq 1 ]; then
            case "$candidate" in
                activate|exec|list|doctor|unset|init|completions|help) continue ;;
            esac
        fi
        case "$candidate" in
            -*) completion="--profile=$candidate" ;;
            *) completion=$candidate ;;
        esac
        if [[ "$completion" == "$current"* || "$candidate" == "$current"* ]]; then
            COMPREPLY+=("$completion")
        fi
    done < <(command awswit list --format completion 2>/dev/null)
}

if type complete >/dev/null 2>&1; then
    complete -o default -F _awswit_complete awswit
fi
