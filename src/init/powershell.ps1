$env:AWSWIT_SHELL = 'powershell'
function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    $output = & (Get-Command awswit -CommandType Application).Source @Arguments
    $exitCode = $LASTEXITCODE

    if ($exitCode -ne 0) {
        Write-Error $output
        $global:LASTEXITCODE = $exitCode
        return
    }

    foreach ($line in $output -split "`n") {
        if ($line -match '^([^=]+)=(.*)$') {
            $key = $Matches[1]
            $value = $Matches[2]

            switch ($key) {
                'AWS_PROFILE' { if ($value) { $env:AWS_PROFILE = $value } else { Remove-Item Env:AWS_PROFILE -ErrorAction SilentlyContinue } }
                'AWS_DEFAULT_PROFILE' { if ($value) { $env:AWS_DEFAULT_PROFILE = $value } else { Remove-Item Env:AWS_DEFAULT_PROFILE -ErrorAction SilentlyContinue } }
                'AWS_REGION' { if ($value) { $env:AWS_REGION = $value } else { Remove-Item Env:AWS_REGION -ErrorAction SilentlyContinue } }
                'AWS_DEFAULT_REGION' { if ($value) { $env:AWS_DEFAULT_REGION = $value } else { Remove-Item Env:AWS_DEFAULT_REGION -ErrorAction SilentlyContinue } }
                'AWSWIT_PROFILE' { if ($value) { $env:AWSWIT_PROFILE = $value } else { Remove-Item Env:AWSWIT_PROFILE -ErrorAction SilentlyContinue } }
                'AWSWIT_UNSET' {
                    Remove-Item Env:AWS_PROFILE -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_DEFAULT_PROFILE -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_REGION -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_DEFAULT_REGION -ErrorAction SilentlyContinue
                    Remove-Item Env:AWSWIT_PROFILE -ErrorAction SilentlyContinue
                }
                default {
                    if ($line) { Write-Output $line }
                }
            }
        } else {
            if ($line) { Write-Output $line }
        }
    }
}
