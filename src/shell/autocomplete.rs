/// Generate autocomplete script for different shells
pub fn generate_autocomplete_script(shell: &str) -> String {
    match shell.to_lowercase().as_str() {
        "bash" => generate_bash_autocomplete(),
        "zsh" => generate_zsh_autocomplete(),
        "fish" => generate_fish_autocomplete(),
        "powershell" | "pwsh" => generate_powershell_autocomplete(),
        _ => String::new(),
    }
}

fn generate_bash_autocomplete() -> String {
    r#"
_awswit_rs() {
    local cur prev opts
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    
    # Get profile names from awswit-autocomplete
    opts=$(awswit-autocomplete 2>/dev/null)
    
    # Add common flags
    flags="-h --help -v --version -r --refresh -s --show-commands -u --unset -a --auto-refresh -k --kill -l --list-profiles --role-arn --source-profile --external-id --mfa-token --region --session-name --role-duration --credentials-file --config-file --info --debug"
    
    case "${prev}" in
        --role-arn|--source-profile|--external-id|--mfa-token|--region|--session-name|--role-duration|--credentials-file|--config-file)
            # These flags expect a value, don't complete
            return 0
            ;;
    esac
    
    if [[ ${cur} == -* ]]; then
        COMPREPLY=( $(compgen -W "${flags}" -- ${cur}) )
    else
        COMPREPLY=( $(compgen -W "${opts}" -- ${cur}) )
    fi
    
    return 0
}

complete -F _awswit_rs awswit
"#.to_string()
}

fn generate_zsh_autocomplete() -> String {
    r#"
#compdef awswit

_awswit_rs() {
    local -a profiles flags
    
    # Get profiles
    profiles=($(awswit-autocomplete 2>/dev/null))
    
    # Define flags
    flags=(
        '-h[Show help]'
        '--help[Show help]'
        '-v[Show version]'
        '--version[Show version]'
        '-r[Force refresh credentials]'
        '--refresh[Force refresh credentials]'
        '-s[Show commands to set credentials]'
        '--show-commands[Show commands to set credentials]'
        '-u[Unset AWS environment variables]'
        '--unset[Unset AWS environment variables]'
        '-a[Auto-refresh credentials]'
        '--auto-refresh[Auto-refresh credentials]'
        '-k[Kill auto-refresher]'
        '--kill[Kill auto-refresher]'
        '-l[List profiles]'
        '--list-profiles[List profiles]'
        '--role-arn[Role ARN to assume]:arn:'
        '--source-profile[Source profile]:profile:($profiles)'
        '--external-id[External ID]:id:'
        '--mfa-token[MFA token]:token:'
        '--region[AWS region]:region:'
        '--session-name[Session name]:name:'
        '--role-duration[Role duration in seconds]:seconds:'
        '--credentials-file[Credentials file path]:file:_files'
        '--config-file[Config file path]:file:_files'
        '--info[Show INFO logs]'
        '--debug[Show DEBUG logs]'
    )
    
    _arguments -s $flags '*:profile:($profiles)'
}

compdef _awswit_rs awswit
_awswit_rs "$@"
"#
    .to_string()
}

fn generate_fish_autocomplete() -> String {
    r#"
# Fish completion for awswit

function __fish_awswit_profiles
    awswit-autocomplete 2>/dev/null
end

# Profile completions
complete -c awswit -f -a "(__fish_awswit_profiles)"

# Flag completions
complete -c awswit -s h -l help -d "Show help"
complete -c awswit -s v -l version -d "Show version"
complete -c awswit -s r -l refresh -d "Force refresh credentials"
complete -c awswit -s s -l show-commands -d "Show export commands"
complete -c awswit -s u -l unset -d "Unset AWS environment variables"
complete -c awswit -s a -l auto-refresh -d "Auto-refresh credentials"
complete -c awswit -s k -l kill -d "Kill auto-refresher"
complete -c awswit -s l -l list-profiles -d "List profiles"
complete -c awswit -l role-arn -d "Role ARN to assume"
complete -c awswit -l source-profile -d "Source profile" -xa "(__fish_awswit_profiles)"
complete -c awswit -l external-id -d "External ID"
complete -c awswit -l mfa-token -d "MFA token"
complete -c awswit -l region -d "AWS region"
complete -c awswit -l session-name -d "Session name"
complete -c awswit -l role-duration -d "Role duration in seconds"
complete -c awswit -l info -d "Show INFO logs"
complete -c awswit -l debug -d "Show DEBUG logs"
"#
    .to_string()
}

fn generate_powershell_autocomplete() -> String {
    r#"
Register-ArgumentCompleter -Native -CommandName awswit,awswit -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    
    $profiles = $(awswit-autocomplete 2>$null)
    $flags = @(
        '-h', '--help',
        '-v', '--version',
        '-r', '--refresh',
        '-s', '--show-commands',
        '-u', '--unset',
        '-a', '--auto-refresh',
        '-k', '--kill',
        '-l', '--list-profiles',
        '--role-arn',
        '--source-profile',
        '--external-id',
        '--mfa-token',
        '--region',
        '--session-name',
        '--role-duration',
        '--credentials-file',
        '--config-file',
        '--info',
        '--debug'
    )
    
    $completions = @()
    
    if ($wordToComplete -like '-*') {
        $completions = $flags | Where-Object { $_ -like "$wordToComplete*" }
    } else {
        $completions = $profiles | Where-Object { $_ -like "$wordToComplete*" }
    }
    
    $completions | Sort-Object | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
"#
    .to_string()
}
