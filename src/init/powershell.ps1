function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    $output = & (Get-Command awswit -CommandType Application).Source @Arguments
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
                'AWS_ACCESS_KEY_ID' { $env:AWS_ACCESS_KEY_ID = $value }
                'AWS_SECRET_ACCESS_KEY' { $env:AWS_SECRET_ACCESS_KEY = $value }
                'AWS_SESSION_TOKEN' { $env:AWS_SESSION_TOKEN = $value }
                'AWS_SECURITY_TOKEN' { $env:AWS_SECURITY_TOKEN = $value }
                'AWS_REGION' { $env:AWS_REGION = $value }
                'AWS_DEFAULT_REGION' { $env:AWS_DEFAULT_REGION = $value }
                'AWS_PROFILE' { $env:AWS_PROFILE = $value }
                'AWS_DEFAULT_PROFILE' { $env:AWS_DEFAULT_PROFILE = $value }
                'AWSWIT_PROFILE' { $env:AWSWIT_PROFILE = $value }
                'AWSWIT_EXPIRATION' { $env:AWSWIT_EXPIRATION = $value }
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
