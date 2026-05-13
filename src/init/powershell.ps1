$env:AWSWIT_SHELL = 'powershell'
function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    $out = & (Get-Command awswit -CommandType Application).Source --shell-export @Arguments
    $rc = $LASTEXITCODE
    if ($rc -eq 0 -and $out) {
        Invoke-Expression ($out -join "`n")
    }
    $global:LASTEXITCODE = $rc
}
