$env:AWSWIT_SHELL = 'powershell'
function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    $binary = (Get-Command awswit -CommandType Application).Source
    if ($Arguments.Count -ge 1) {
        switch -Regex ($Arguments[0]) {
            '^(exec|which|doctor|init|completions|prompt|help|-h|--help|-v|--version|-l|--list)$' {
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
