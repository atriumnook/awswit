set -gx AWSWIT_SHELL fish

# True iff `$argv` contains a token that means "the binary's own output is
# for the user, not for `source`". Scans every arg so flag order doesn't
# matter: `awswit -l --json` and `awswit --json -l` both bypass the
# source path.
function __awswit_is_info
    for a in $argv
        switch $a
            case exec pick which doctor init completions prompt help \
                 -h --help -v --version -l --list --json --names-only \
                 -s --shell-export
                return 0
        end
    end
    return 1
end

function awswit
    if __awswit_is_info $argv
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
    command awswit -l --names-only 2>/dev/null
end

complete -c awswit -f
complete -c awswit -n __fish_use_subcommand -a 'exec pick which doctor prompt init completions help'
complete -c awswit -n __fish_use_subcommand -a '(__awswit_profiles)'
complete -c awswit -n '__fish_seen_subcommand_from exec' -a '(__awswit_profiles)'
