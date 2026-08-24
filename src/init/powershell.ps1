# awswit shell integration for PowerShell.
# Generated profile data is parsed as data; it is never evaluated.
$script:AwswitExecutable = (Get-Command awswit -CommandType Application).Source

function Write-AwswitPatchError {
    [Console]::Error.WriteLine('awswit: rejected an invalid activation response; environment unchanged')
}

function Test-AwswitStoredSessionCredentials {
    # AWS Tools for PowerShell resolves this public session variable before
    # environment and shared-profile providers. Inspect only the PSVariable
    # container and whether its value is null; never inspect credential fields
    # or convert the credential object to text.
    $storedVariable = Microsoft.PowerShell.Utility\Get-Variable `
        -Name StoredAWSCredentials -ErrorAction SilentlyContinue
    if ($null -eq $storedVariable) {
        return $false
    }
    return -not [object]::ReferenceEquals($null, $storedVariable.Value)
}

function Write-AwswitStoredSessionCredentialError {
    [Console]::Error.WriteLine(
        'awswit: PowerShell session credentials in $StoredAWSCredentials can override the selected profile; run Clear-AWSCredential, then retry'
    )
}

function Write-AwswitPowerShellExecSeparatorError {
    [Console]::Error.WriteLine(
        "awswit: PowerShell cannot safely infer the exec command boundary; quote '--' and retry"
    )
}

function Find-AwswitPowerShellExecCommandIndex {
    param([string[]]$Arguments)

    # PowerShell consumes an unquoted -- before a function receives argv. Walk
    # only awswit's closed exec-option grammar. Once the required profile has
    # been seen, the first non-option token is the command. An option-shaped
    # command is inherently ambiguous and must use a quoted '--' delimiter.
    $positionalProfileSeen = $false
    $namedProfileSeen = $false
    $index = 1
    while ($index -lt $Arguments.Count) {
        $argument = $Arguments[$index]

        if ($argument -ceq '-h' -or $argument -ceq '--help' -or
            $argument -ceq '-V' -or $argument -ceq '--version') {
            if ($positionalProfileSeen -or $namedProfileSeen) {
                return -1
            }
            return -2
        }
        if ($argument -ceq '--clear-credential-overrides') {
            $index++
            continue
        }
        if (@('--profile', '--region', '--config-file', '--credentials-file') -ccontains $argument) {
            if ($index + 1 -ge $Arguments.Count) {
                return -1
            }
            if ($argument -ceq '--profile') {
                $namedProfileSeen = $true
            }
            $index += 2
            continue
        }
        if ($null -ne $argument -and
            $argument.StartsWith('--profile=', [System.StringComparison]::Ordinal)) {
            $namedProfileSeen = $true
            $index++
            continue
        }
        if ($null -ne $argument -and (
            $argument.StartsWith('--region=', [System.StringComparison]::Ordinal) -or
            $argument.StartsWith('--config-file=', [System.StringComparison]::Ordinal) -or
            $argument.StartsWith('--credentials-file=', [System.StringComparison]::Ordinal)
        )) {
            $index++
            continue
        }
        if ($null -ne $argument -and
            $argument.StartsWith('-', [System.StringComparison]::Ordinal)) {
            return -1
        }
        if (-not $positionalProfileSeen -and -not $namedProfileSeen) {
            $positionalProfileSeen = $true
            $index++
            continue
        }
        return $index
    }
    return -1
}

function Remove-AwswitProcessEnvironmentVariable {
    param([string]$Key)

    # On Unix, Environment.SetEnvironmentVariable(name, $null, Process) can
    # leave NAME= in PowerShell's process environment. Remove the provider item
    # so child processes inherit a genuinely absent key.
    if ($null -ne [Environment]::GetEnvironmentVariable($Key, 'Process')) {
        Microsoft.PowerShell.Management\Remove-Item `
            -LiteralPath "Env:$Key" -ErrorAction Stop
    }
}

function Invoke-AwswitUtf8Native {
    param([string[]]$Arguments)

    # Rust's textual machine interfaces are UTF-8. Windows PowerShell and a
    # customized PowerShell 7 session may otherwise decode native stdout with a
    # legacy console code page and silently turn an exact profile into another
    # string. Scope the decoder change to this one native invocation.
    $previousOutputEncoding = [Console]::OutputEncoding
    $nativeExitCode = $null
    try {
        [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
        & $script:AwswitExecutable @Arguments
        $nativeExitCode = $LASTEXITCODE
    }
    finally {
        [Console]::OutputEncoding = $previousOutputEncoding
    }
    if ($null -ne $nativeExitCode) {
        $global:LASTEXITCODE = $nativeExitCode
    }
}

function Invoke-AwswitPatch {
    param([string[]]$Lines)

    if ($null -eq $Lines -or $Lines.Count -eq 0) {
        Write-AwswitPatchError
        return $false
    }

    $kind = switch -CaseSensitive ($Lines[0]) {
        'AWSWIT-PATCH 1 ACTIVATE' { 'activate'; break }
        'AWSWIT-PATCH 1 UNSET' { 'unset'; break }
        default { $null }
    }
    if ($null -eq $kind) {
        Write-AwswitPatchError
        return $false
    }

    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $records = [System.Collections.Generic.List[object]]::new()
    $committed = $false
    $profile = $null
    $profileDefault = $null
    $profileAwswit = $null
    $regionOperation = $null
    $regionValue = $null
    $defaultRegionOperation = $null
    $defaultRegionValue = $null

    for ($index = 1; $index -lt $Lines.Count; $index++) {
        $line = $Lines[$index]
        if ($committed) {
            Write-AwswitPatchError
            return $false
        }
        if ($line -ceq 'AWSWIT-COMMIT') {
            $committed = $true
            continue
        }

        $operation = $null
        $key = $null
        $value = $null
        if ($line.StartsWith('SET ', [System.StringComparison]::Ordinal)) {
            if ($kind -cne 'activate') {
                Write-AwswitPatchError
                return $false
            }
            $record = $line.Substring(4)
            $separator = $record.IndexOf('=')
            if ($separator -le 0) {
                Write-AwswitPatchError
                return $false
            }
            $operation = 'set'
            $key = $record.Substring(0, $separator)
            $value = $record.Substring($separator + 1)
            if ([string]::IsNullOrEmpty($value) -or $value -cmatch '[\x00-\x1f\x7f-\x9f\u061c\u200e\u200f\u2028-\u202e\u2066-\u2069]') {
                Write-AwswitPatchError
                return $false
            }
            if ($key -cnotin @(
                'AWS_PROFILE', 'AWS_DEFAULT_PROFILE', 'AWSWIT_PROFILE',
                'AWS_REGION', 'AWS_DEFAULT_REGION', 'AWS_CONFIG_FILE',
                'AWS_SHARED_CREDENTIALS_FILE'
            )) {
                Write-AwswitPatchError
                return $false
            }
        }
        elseif ($line.StartsWith('UNSET ', [System.StringComparison]::Ordinal)) {
            $operation = 'unset'
            $key = $line.Substring(6)
            if ($key -cnotin @(
                'AWS_PROFILE', 'AWS_DEFAULT_PROFILE', 'AWSWIT_PROFILE',
                'AWS_REGION', 'AWS_DEFAULT_REGION', 'AWS_CONFIG_FILE',
                'AWS_SHARED_CREDENTIALS_FILE', 'AWS_ACCESS_KEY_ID',
                'AWS_ACCESS_KEY', 'AMAZON_ACCESS_KEY_ID',
                'AWS_SECRET_ACCESS_KEY', 'AWS_SECRET_KEY',
                'AMAZON_SECRET_ACCESS_KEY', 'AWS_SESSION_TOKEN',
                'AWS_SECURITY_TOKEN', 'AMAZON_SESSION_TOKEN',
                'AWS_WEB_IDENTITY_TOKEN_FILE', 'AWS_ROLE_ARN', 'AWS_ROLE_SESSION_NAME',
                'AWS_CONTAINER_CREDENTIALS_RELATIVE_URI',
                'AWS_CONTAINER_CREDENTIALS_FULL_URI',
                'AWS_CONTAINER_AUTHORIZATION_TOKEN',
                'AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE',
                'AWS_EC2_METADATA_SERVICE_ENDPOINT', 'AWS_LOGIN_CACHE_DIRECTORY',
                'AWS_CREDENTIAL_PROFILES_FILE',
                'AWS_BEARER_TOKEN_BEDROCK'
            )) {
                Write-AwswitPatchError
                return $false
            }
            if ($kind -ceq 'activate' -and $key -cnotin @(
                'AWS_REGION', 'AWS_DEFAULT_REGION', 'AWS_ACCESS_KEY_ID',
                'AWS_ACCESS_KEY', 'AMAZON_ACCESS_KEY_ID',
                'AWS_SECRET_ACCESS_KEY', 'AWS_SECRET_KEY',
                'AMAZON_SECRET_ACCESS_KEY', 'AWS_SESSION_TOKEN',
                'AWS_SECURITY_TOKEN', 'AMAZON_SESSION_TOKEN',
                'AWS_WEB_IDENTITY_TOKEN_FILE', 'AWS_ROLE_ARN', 'AWS_ROLE_SESSION_NAME',
                'AWS_CONTAINER_CREDENTIALS_RELATIVE_URI',
                'AWS_CONTAINER_CREDENTIALS_FULL_URI',
                'AWS_CONTAINER_AUTHORIZATION_TOKEN',
                'AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE',
                'AWS_EC2_METADATA_SERVICE_ENDPOINT', 'AWS_LOGIN_CACHE_DIRECTORY',
                'AWS_CREDENTIAL_PROFILES_FILE',
                'AWS_BEARER_TOKEN_BEDROCK'
            )) {
                Write-AwswitPatchError
                return $false
            }
            if ($kind -ceq 'unset' -and $key -cnotin @(
                'AWS_PROFILE', 'AWS_DEFAULT_PROFILE', 'AWSWIT_PROFILE',
                'AWS_REGION', 'AWS_DEFAULT_REGION'
            )) {
                Write-AwswitPatchError
                return $false
            }
        }
        else {
            Write-AwswitPatchError
            return $false
        }

        if (-not $seen.Add($key)) {
            Write-AwswitPatchError
            return $false
        }
        $records.Add([pscustomobject]@{ Operation = $operation; Key = $key; Value = $value })

        if ($operation -ceq 'set') {
            switch -CaseSensitive ($key) {
                'AWS_PROFILE' { $profile = $value }
                'AWS_DEFAULT_PROFILE' { $profileDefault = $value }
                'AWSWIT_PROFILE' { $profileAwswit = $value }
                'AWS_REGION' { $regionOperation = 'set'; $regionValue = $value }
                'AWS_DEFAULT_REGION' { $defaultRegionOperation = 'set'; $defaultRegionValue = $value }
            }
        }
        else {
            switch -CaseSensitive ($key) {
                'AWS_REGION' { $regionOperation = 'unset' }
                'AWS_DEFAULT_REGION' { $defaultRegionOperation = 'unset' }
            }
        }
    }

    if (-not $committed) {
        Write-AwswitPatchError
        return $false
    }
    if ($kind -ceq 'activate') {
        if ([string]::IsNullOrEmpty($profile) -or $profile -cne $profileDefault -or $profile -cne $profileAwswit) {
            Write-AwswitPatchError
            return $false
        }
        if ($null -eq $regionOperation -or $regionOperation -cne $defaultRegionOperation) {
            Write-AwswitPatchError
            return $false
        }
        if ($regionOperation -ceq 'set' -and $regionValue -cne $defaultRegionValue) {
            Write-AwswitPatchError
            return $false
        }
    }
    else {
        if ($records.Count -ne 5) {
            Write-AwswitPatchError
            return $false
        }
        foreach ($required in @(
            'AWS_PROFILE', 'AWS_DEFAULT_PROFILE', 'AWSWIT_PROFILE',
            'AWS_REGION', 'AWS_DEFAULT_REGION'
        )) {
            if (-not $seen.Contains($required)) {
                Write-AwswitPatchError
                return $false
            }
        }
    }

    # The complete frame is valid.  Apply the allow-listed records now.
    $previous = @{}
    foreach ($record in $records) {
        $previous[$record.Key] = [Environment]::GetEnvironmentVariable($record.Key, 'Process')
    }
    try {
        foreach ($record in $records) {
            if ($record.Operation -ceq 'set') {
                [Environment]::SetEnvironmentVariable(
                    $record.Key, $record.Value, 'Process'
                )
            }
            else {
                Remove-AwswitProcessEnvironmentVariable -Key $record.Key
            }
        }
    }
    catch {
        foreach ($record in $records) {
            if ($null -eq $previous[$record.Key]) {
                Remove-AwswitProcessEnvironmentVariable -Key $record.Key
            }
            else {
                [Environment]::SetEnvironmentVariable(
                    $record.Key, $previous[$record.Key], 'Process'
                )
            }
        }
        Write-AwswitPatchError
        return $false
    }

    if ($kind -eq 'activate') {
        [Console]::Error.WriteLine('awswit: AWS profile activated')
    }
    else {
        [Console]::Error.WriteLine('awswit: AWS profile cleared')
    }
    return $true
}

function awswit {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    if ($Arguments.Count -gt 0 -and $Arguments[0] -ceq 'exec' -and
        $Arguments -cnotcontains '--') {
        $commandIndex = Find-AwswitPowerShellExecCommandIndex -Arguments $Arguments
        if ($commandIndex -eq -2) {
            # A display option belongs to the awswit prefix. The ordinary
            # display path below forwards it unchanged.
        }
        elseif ($commandIndex -lt 0) {
            Write-AwswitPowerShellExecSeparatorError
            $global:LASTEXITCODE = 2
            return
        }
        else {
            $Arguments = @(
                $Arguments[0..($commandIndex - 1)] +
                '--' +
                $Arguments[$commandIndex..($Arguments.Count - 1)]
            )
        }
    }

    $displayRequest = $false
    foreach ($argument in $Arguments) {
        if ($argument -ceq '--') {
            break
        }
        if ($argument -cin @('--help', '-h', '--version', '-V')) {
            $displayRequest = $true
            break
        }
    }
    if ($displayRequest) {
        & $script:AwswitExecutable @Arguments
        $global:LASTEXITCODE = $LASTEXITCODE
        return
    }

    $activationRequest = $false
    if ($Arguments.Count -eq 0) {
        $activationRequest = $true
        $binaryArguments = @('activate')
    }
    elseif ($Arguments[0] -cin @('activate', 'unset')) {
        $activationRequest = $Arguments[0] -ceq 'activate'
        $binaryArguments = $Arguments
    }
    elseif ($Arguments[0] -ceq 'doctor') {
        $previousHook = $env:AWSWIT_HOOK
        $previousShell = $env:AWSWIT_SHELL
        try {
            $env:AWSWIT_HOOK = '1'
            $env:AWSWIT_SHELL = 'powershell'
            Invoke-AwswitUtf8Native -Arguments $Arguments
            $exitCode = $LASTEXITCODE
        }
        finally {
            $env:AWSWIT_HOOK = $previousHook
            $env:AWSWIT_SHELL = $previousShell
        }
        $global:LASTEXITCODE = $exitCode
        return
    }
    elseif ($Arguments[0] -ceq 'exec') {
        # Child stdout has the child's encoding, not awswit's UTF-8 machine
        # protocol. Preserve the caller's decoder for this pass-through path.
        & $script:AwswitExecutable @Arguments
        $global:LASTEXITCODE = $LASTEXITCODE
        return
    }
    elseif ($Arguments[0] -cin @('list', 'init', 'completions')) {
        Invoke-AwswitUtf8Native -Arguments $Arguments
        $global:LASTEXITCODE = $LASTEXITCODE
        return
    }
    elseif ($Arguments[0] -cin @('help', '--help', '-h', '--version', '-V')) {
        & $script:AwswitExecutable @Arguments
        $global:LASTEXITCODE = $LASTEXITCODE
        return
    }
    else {
        $activationRequest = $true
        $binaryArguments = @('activate') + $Arguments
    }

    if ($activationRequest -and (Test-AwswitStoredSessionCredentials)) {
        Write-AwswitStoredSessionCredentialError
        $global:LASTEXITCODE = 1
        return
    }

    $previousHook = $env:AWSWIT_HOOK
    $previousShell = $env:AWSWIT_SHELL
    try {
        $env:AWSWIT_HOOK = '1'
        $env:AWSWIT_SHELL = 'powershell'
        $output = @(Invoke-AwswitUtf8Native -Arguments $binaryArguments)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $env:AWSWIT_HOOK = $previousHook
        $env:AWSWIT_SHELL = $previousShell
    }
    if ($exitCode -cne 0) {
        $global:LASTEXITCODE = $exitCode
        return
    }

    if (Invoke-AwswitPatch -Lines $output) {
        $global:LASTEXITCODE = 0
    }
    else {
        $global:LASTEXITCODE = 1
    }
}

Register-ArgumentCompleter -CommandName awswit -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $elements = @($commandAst.CommandElements)
    $first = if ($elements.Count -ge 2) { $elements[1].Extent.Text } else { $null }
    $previous = if ($elements.Count -ge 2) { $elements[$elements.Count - 2].Extent.Text } else { $null }
    $rootArgument = $elements.Count -le 1 -or (
        $elements.Count -eq 2 -and $first -ceq $wordToComplete
    )
    $profileArgument = $elements.Count -le 3 -and $first -cin @('activate', 'exec')
    $commands = @('activate', 'exec', 'list', 'doctor', 'unset', 'init', 'completions', 'help')
    $candidates = [System.Collections.Generic.List[System.Management.Automation.CompletionResult]]::new()

    $staticValues = if ($rootArgument) {
        @('-h', '--help', '-V', '--version') + $commands
    }
    elseif ($first -cin @('activate', 'exec')) {
        @('-h', '--help', '--profile', '--region', '--clear-credential-overrides', '--config-file', '--credentials-file')
    }
    elseif ($first -cin @('list', 'doctor')) {
        if ($previous -ceq '--format') {
            if ($first -ceq 'list') { @('human', 'names', 'json') } else { @('human', 'json') }
        }
        else {
            @('-h', '--help', '--format', '--config-file', '--credentials-file')
        }
    }
    elseif ($first -cin @('init', 'completions')) {
        @('-h', '--help', 'bash', 'zsh', 'fish', 'powershell')
    }
    elseif ($first -ceq 'unset') {
        @('-h', '--help')
    }
    elseif ($first -ceq 'help') {
        $commands
    }
    else {
        @()
    }
    foreach ($value in $staticValues) {
        if ($value.StartsWith($wordToComplete, [System.StringComparison]::OrdinalIgnoreCase)) {
            $type = if ($value.StartsWith('-')) { 'ParameterName' } else { 'ParameterValue' }
            $candidates.Add([System.Management.Automation.CompletionResult]::new($value, $value, $type, $value))
        }
    }

    # Completion is observational: its internal list process must not change
    # the exit status that the interactive caller may still be inspecting.
    $previousNativeExitCode = $global:LASTEXITCODE
    try {
        if ($rootArgument -or $profileArgument) {
            foreach ($name in @(
                Invoke-AwswitUtf8Native -Arguments @('list', '--format', 'completion') 2>$null
            )) {
                if ([string]::IsNullOrEmpty($name) -or ($rootArgument -and $name -cin $commands)) {
                    continue
                }
                $completionValue = if ($name.StartsWith('-')) { "--profile=$name" } else { $name }
                if (-not $completionValue.StartsWith($wordToComplete, [System.StringComparison]::OrdinalIgnoreCase) -and
                    -not $name.StartsWith($wordToComplete, [System.StringComparison]::OrdinalIgnoreCase)) {
                    continue
                }
                $completionText = "'" + $completionValue.Replace("'", "''") + "'"
                $candidates.Add([System.Management.Automation.CompletionResult]::new(
                    $completionText, $name, 'ParameterValue', 'AWS profile'
                ))
            }
        }
    }
    finally {
        $global:LASTEXITCODE = $previousNativeExitCode
    }
    $candidates
}
