# awswit shell wrapper for PowerShell
# Add to your PowerShell profile: . /path/to/awswit.ps1

function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    $output = & awswit.exe @Arguments
    $exitCode = $LASTEXITCODE

    if ($exitCode -ne 0) {
        Write-Error $output
        return
    }

    foreach ($line in $output -split "`n") {
        if ($line -match '^([^=]+)=(.*)$') {
            $key = $Matches[1]
            $value = $Matches[2]

            switch ($key) {
                'AWS_ACCESS_KEY_ID' { if ($value) { $env:AWS_ACCESS_KEY_ID = $value } else { Remove-Item Env:AWS_ACCESS_KEY_ID -ErrorAction SilentlyContinue } }
                'AWS_SECRET_ACCESS_KEY' { if ($value) { $env:AWS_SECRET_ACCESS_KEY = $value } else { Remove-Item Env:AWS_SECRET_ACCESS_KEY -ErrorAction SilentlyContinue } }
                'AWS_SESSION_TOKEN' { if ($value) { $env:AWS_SESSION_TOKEN = $value } else { Remove-Item Env:AWS_SESSION_TOKEN -ErrorAction SilentlyContinue } }
                'AWS_SECURITY_TOKEN' { if ($value) { $env:AWS_SECURITY_TOKEN = $value } else { Remove-Item Env:AWS_SECURITY_TOKEN -ErrorAction SilentlyContinue } }
                'AWS_REGION' { if ($value) { $env:AWS_REGION = $value } else { Remove-Item Env:AWS_REGION -ErrorAction SilentlyContinue } }
                'AWS_DEFAULT_REGION' { if ($value) { $env:AWS_DEFAULT_REGION = $value } else { Remove-Item Env:AWS_DEFAULT_REGION -ErrorAction SilentlyContinue } }
                'AWS_PROFILE' { if ($value) { $env:AWS_PROFILE = $value } else { Remove-Item Env:AWS_PROFILE -ErrorAction SilentlyContinue } }
                'AWS_DEFAULT_PROFILE' { if ($value) { $env:AWS_DEFAULT_PROFILE = $value } else { Remove-Item Env:AWS_DEFAULT_PROFILE -ErrorAction SilentlyContinue } }
                'AWSWIT_PROFILE' { if ($value) { $env:AWSWIT_PROFILE = $value } else { Remove-Item Env:AWSWIT_PROFILE -ErrorAction SilentlyContinue } }
                'AWSWIT_EXPIRATION' { if ($value) { $env:AWSWIT_EXPIRATION = $value } else { Remove-Item Env:AWSWIT_EXPIRATION -ErrorAction SilentlyContinue } }
                'AWSWIT_UNSET' {
                    Remove-Item Env:AWS_ACCESS_KEY_ID -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_SECRET_ACCESS_KEY -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_SESSION_TOKEN -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_SECURITY_TOKEN -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_REGION -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_DEFAULT_REGION -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_PROFILE -ErrorAction SilentlyContinue
                    Remove-Item Env:AWS_DEFAULT_PROFILE -ErrorAction SilentlyContinue
                    Remove-Item Env:AWSWIT_PROFILE -ErrorAction SilentlyContinue
                    Remove-Item Env:AWSWIT_EXPIRATION -ErrorAction SilentlyContinue
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
