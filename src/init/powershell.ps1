$env:AWSWIT_SHELL = 'powershell'

function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    $binary = (Get-Command awswit -CommandType Application).Source
    if ($Arguments.Count -ge 1) {
        switch -Regex ($Arguments[0]) {
            '^(exec|pick|which|doctor|init|completions|prompt|help|-h|--help|-v|--version|-l|--list)$' {
                & $binary @Arguments
                return
            }
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
Register-ArgumentCompleter -Native -CommandName awswit -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $tokens = $commandAst.CommandElements | ForEach-Object { $_.ToString() }
    $position = $tokens.Count
    if ($wordToComplete) { $position -= 1 }

    $subs = @('exec', 'pick', 'which', 'doctor', 'prompt', 'init', 'completions', 'help')
    $profiles = @()
    try {
        $binary = (Get-Command awswit -CommandType Application).Source
        $profiles = (& $binary -l 2>$null) | ForEach-Object { ($_ -split "`t")[0] }
    } catch {}

    if ($position -eq 1) {
        $candidates = $subs + $profiles
    } elseif ($position -eq 2 -and $tokens[1] -eq 'exec') {
        $candidates = $profiles
    } else {
        return
    }
    $candidates | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
