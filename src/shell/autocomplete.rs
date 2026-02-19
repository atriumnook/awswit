use crate::shell::export::ShellType;

/// Generate shell completion script
pub fn generate_completion(shell: &ShellType) -> String {
    match shell {
        ShellType::Bash => generate_bash_completion(),
        ShellType::Zsh => generate_zsh_completion(),
        ShellType::Fish => generate_fish_completion(),
        ShellType::PowerShell => generate_powershell_completion(),
    }
}

fn generate_bash_completion() -> String {
    r#"_awswit_completions() {
    local cur prev opts profiles
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    opts="-r --refresh -s --show-commands -u --unset -a --auto-refresh -k --kill-refresher -l --list-profiles -n --no-interactive --role-arn --source-profile --external-id --mfa-token --region --session-name --role-duration --config-file --credentials-file -v --version -h --help --debug --info --favorite --completion --credential-process"

    if [[ ${cur} == -* ]]; then
        COMPREPLY=( $(compgen -W "${opts}" -- ${cur}) )
        return 0
    fi

    # Complete profile names
    if command -v awswit &>/dev/null; then
        profiles=$(command awswit --list-profiles 2>/dev/null | grep -v '^$')
        COMPREPLY=( $(compgen -W "${profiles}" -- ${cur}) )
    fi
}

complete -F _awswit_completions awswit
"#
    .to_string()
}

fn generate_zsh_completion() -> String {
    r#"#compdef awswit

_awswit() {
    local -a profiles
    local -a options

    options=(
        '-r[Force credential refresh]'
        '--refresh[Force credential refresh]'
        '-s[Show export commands]'
        '--show-commands[Show export commands]'
        '-u[Unset AWS environment variables]'
        '--unset[Unset AWS environment variables]'
        '-a[Enable auto-refresh]'
        '--auto-refresh[Enable auto-refresh]'
        '-k[Kill auto-refresh daemon]'
        '--kill-refresher[Kill auto-refresh daemon]'
        '-l[List profiles]'
        '--list-profiles[List profiles]'
        '-n[Disable interactive mode]'
        '--no-interactive[Disable interactive mode]'
        '--role-arn[Role ARN]:arn:'
        '--source-profile[Source profile]:profile:'
        '--external-id[External ID]:id:'
        '--mfa-token[MFA token]:token:'
        '--region[AWS region]:region:'
        '--session-name[Session name]:name:'
        '--role-duration[Role duration (seconds)]:duration:'
        '--config-file[Config file path]:file:_files'
        '--credentials-file[Credentials file path]:file:_files'
        '--debug[Enable debug logging]'
        '--info[Enable info logging]'
        '--favorite[Toggle favorite]:profile:'
        '--completion[Generate completion script]:shell:(bash zsh fish powershell)'
        '--credential-process[Output for credential_process]'
        '-v[Show version]'
        '--version[Show version]'
        '-h[Show help]'
        '--help[Show help]'
    )

    if command -v awswit &>/dev/null; then
        profiles=(${(f)"$(command awswit --list-profiles 2>/dev/null)"})
    fi

    _arguments -s $options '*:profile:compadd -a profiles'
}

_awswit "$@"
"#
    .to_string()
}

fn generate_fish_completion() -> String {
    r#"# Fish completions for awswit

complete -c awswit -s r -l refresh -d 'Force credential refresh'
complete -c awswit -s s -l show-commands -d 'Show export commands'
complete -c awswit -s u -l unset -d 'Unset AWS environment variables'
complete -c awswit -s a -l auto-refresh -d 'Enable auto-refresh'
complete -c awswit -s k -l kill-refresher -d 'Kill auto-refresh daemon'
complete -c awswit -s l -l list-profiles -d 'List profiles'
complete -c awswit -s n -l no-interactive -d 'Disable interactive mode'
complete -c awswit -l role-arn -d 'Role ARN' -x
complete -c awswit -l source-profile -d 'Source profile' -x
complete -c awswit -l external-id -d 'External ID' -x
complete -c awswit -l mfa-token -d 'MFA token' -x
complete -c awswit -l region -d 'AWS region' -x
complete -c awswit -l session-name -d 'Session name' -x
complete -c awswit -l role-duration -d 'Role duration (seconds)' -x
complete -c awswit -l config-file -d 'Config file path' -rF
complete -c awswit -l credentials-file -d 'Credentials file path' -rF
complete -c awswit -l debug -d 'Enable debug logging'
complete -c awswit -l info -d 'Enable info logging'
complete -c awswit -l favorite -d 'Toggle favorite' -x
complete -c awswit -l completion -d 'Generate completion script' -xa 'bash zsh fish powershell'
complete -c awswit -l credential-process -d 'Output for credential_process'
complete -c awswit -s v -l version -d 'Show version'
complete -c awswit -s h -l help -d 'Show help'

# Profile name completions
complete -c awswit -f -a "(command awswit --list-profiles 2>/dev/null)"
"#
    .to_string()
}

fn generate_powershell_completion() -> String {
    r#"Register-ArgumentCompleter -CommandName awswit -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $options = @(
        [CompletionResult]::new('-r', '-r', 'ParameterName', 'Force credential refresh')
        [CompletionResult]::new('--refresh', '--refresh', 'ParameterName', 'Force credential refresh')
        [CompletionResult]::new('-s', '-s', 'ParameterName', 'Show export commands')
        [CompletionResult]::new('--show-commands', '--show-commands', 'ParameterName', 'Show export commands')
        [CompletionResult]::new('-u', '-u', 'ParameterName', 'Unset AWS environment variables')
        [CompletionResult]::new('--unset', '--unset', 'ParameterName', 'Unset AWS environment variables')
        [CompletionResult]::new('-a', '-a', 'ParameterName', 'Enable auto-refresh')
        [CompletionResult]::new('--auto-refresh', '--auto-refresh', 'ParameterName', 'Enable auto-refresh')
        [CompletionResult]::new('-k', '-k', 'ParameterName', 'Kill auto-refresh daemon')
        [CompletionResult]::new('--kill-refresher', '--kill-refresher', 'ParameterName', 'Kill auto-refresh daemon')
        [CompletionResult]::new('-l', '-l', 'ParameterName', 'List profiles')
        [CompletionResult]::new('-n', '-n', 'ParameterName', 'Disable interactive mode')
        [CompletionResult]::new('--debug', '--debug', 'ParameterName', 'Enable debug logging')
        [CompletionResult]::new('--info', '--info', 'ParameterName', 'Enable info logging')
    )

    # Get profile names
    try {
        $profiles = & awswit --list-profiles 2>$null
        foreach ($profile in $profiles) {
            if ($profile -and $profile.Trim()) {
                $options += [CompletionResult]::new($profile.Trim(), $profile.Trim(), 'ParameterValue', "AWS Profile: $($profile.Trim())")
            }
        }
    } catch {}

    $options | Where-Object { $_.CompletionText -like "$wordToComplete*" }
}
"#
    .to_string()
}
