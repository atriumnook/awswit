set -gx AWSWIT_SHELL fish
function awswit
    set -l _out (command awswit --shell-export $argv)
    set -l _rc $status
    if test $_rc -eq 0; and test -n "$_out"
        echo $_out | source
    end
    return $_rc
end
