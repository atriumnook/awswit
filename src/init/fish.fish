set -gx AWSWIT_SHELL fish

function awswit
    switch $argv[1]
        case exec which doctor init completions prompt help -h --help -v --version -l --list
            command awswit $argv
            return
    end
    set -l _out (command awswit --shell-export $argv)
    set -l _rc $status
    if test $_rc -eq 0; and test -n "$_out"
        echo $_out | source
    end
    return $_rc
end

# Fish tab completion: profile names + subcommands.
function __awswit_profiles
    command awswit -l 2>/dev/null | cut -f1
end

complete -c awswit -f
complete -c awswit -n __fish_use_subcommand -a 'exec which doctor prompt init completions help'
complete -c awswit -n __fish_use_subcommand -a '(__awswit_profiles)'
complete -c awswit -n '__fish_seen_subcommand_from exec' -a '(__awswit_profiles)'
