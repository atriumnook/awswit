$env:AWSWIT_SHELL = 'powershell'

function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    $binary = (Get-Command awswit -CommandType Application).Source

    # Scan every arg so flag order doesn't matter — `awswit -l --json` and
    # `awswit --json -l` both bypass the Invoke-Expression path.
    $infoTokens = @(
        'exec', 'pick', 'which', 'doctor', 'init', 'completions', 'prompt',
        'help', '-h', '--help', '-v', '--version',
        '-l', '--list', '--json', '--names-only',
        '-s', '--shell-export'
    )
    foreach ($arg in $Arguments) {
        if ($infoTokens -contains $arg) {
            & $binary @Arguments
            return
        }
    }

    $out = & $binary --shell-export @Arguments
    $rc = $LASTEXITCODE
    if ($rc -eq 0 -and $out) {
        Invoke-Expression ($out -join "`n")
    }
    $global:LASTEXITCODE = $rc
}

# PowerShell tab completion: profile names + subcommands.
#
# Notes:
# - Match with `StartsWith` (ordinal, case-insensitive) instead of `-like`
#   so profile names containing PowerShell wildcard characters (`*`, `?`,
#   `[`, `]`) match literally rather than being interpreted as patterns.
# - Names captured from the binary are TrimEnd'd of `\r` because on Windows
#   the parent process may CRLF-translate stdout.
Register-ArgumentCompleter -Native -CommandName awswit -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $tokens = $commandAst.CommandElements | ForEach-Object { $_.ToString() }
    $position = $tokens.Count
    if ($wordToComplete) { $position -= 1 }

    $subs = @('exec', 'pick', 'which', 'doctor', 'prompt', 'init', 'completions', 'help')
    $profiles = @()
    try {
        $binary = (Get-Command awswit -CommandType Application).Source
        $profiles = (& $binary -l --names-only 2>$null) |
            ForEach-Object { $_.TrimEnd("`r") } |
            Where-Object { $_ -ne '' }
    } catch {}

    if ($position -eq 1) {
        $candidates = $subs + $profiles
    } elseif ($position -eq 2 -and $tokens[1] -eq 'exec') {
        $candidates = $profiles
    } else {
        return
    }
    $candidates | Where-Object {
        $_.StartsWith($wordToComplete, [System.StringComparison]::OrdinalIgnoreCase)
    } | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
