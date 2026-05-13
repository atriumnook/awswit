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
