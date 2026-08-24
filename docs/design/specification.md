# awswit 詳細仕様

状態: 現行 pre-1.0 実装契約
更新日: 2026-08-25

## 1. Scope

awswit は AWS 共有設定から profile を選び、次のいずれかの scope に標準 AWS 環境変数を適用する。

- **current shell** — 生成済み shell hook を介する `activate`
- **one command** — 親環境を変えない `exec`

awswit は credential manager ではない。設定された provider が有効か、login 済みか、最終的にどの
AWS principal になるかは検証しない。AWS CLI / SDK が選択後に credential provider chain を解決する。

## 2. 対応範囲

### 2.1 Release target

release workflow が対象とする executable target は次の5つである。

| OS | Architecture | Target |
|---|---|---|
| Linux | x86-64 | `x86_64-unknown-linux-gnu` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` |
| macOS | Intel | `x86_64-apple-darwin` |
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| Windows | x86-64 | `x86_64-pc-windows-msvc` |

生成可能な hook は Bash、Zsh、Fish、PowerShell である。prebuilt executable は Python、Node、AWS CLI、
fzf、Nerd Font を要求しない。Linux GNU build は OS の通常の system runtime まで静的に内包するという
意味ではない。

### 2.2 AWS configuration

catalog は次をnamed profile recordとして統合し、local/transitive validationを通ったrecordだけをselectableにする。

- config: `[default]`、`[profile NAME]`
- credentials: `[default]`、`[NAME]`

次は metadata section であり selectable ではない。

- `[sso-session NAME]`
- `[services NAME]` と未知の非 profile section

modern SSO session、legacy inline SSO、AWS Loginの`login_session`、role / `source_profile`、
`credential_source`、`credential_process`、web identity、static credential key の**設定上の存在**を識別する。
認証は実行しない。

## 3. CLI contract

### 3.1 Executable grammar

```text
awswit
awswit activate [PROFILE] [--region REGION]
                [--clear-credential-overrides]
                [--config-file PATH] [--credentials-file PATH]
awswit exec PROFILE [--region REGION]
             [--clear-credential-overrides]
             [--config-file PATH] [--credentials-file PATH]
             -- COMMAND [ARG...]
awswit list [--format human|names|json]
            [--config-file PATH] [--credentials-file PATH]
awswit doctor [--format human|json]
              [--config-file PATH] [--credentials-file PATH]
awswit unset
awswit init bash|zsh|fish|powershell
awswit completions bash|zsh|fish|powershell
```

bare executable invocation is interactive activation and therefore requires the hook marker. Direct executable `unset`も
parent shellを変更できないため同じmarkerなしでは`HOOK_REQUIRED`となり、protocol frameを表示しない。The executable
itself does not accept `awswit PROFILE`; arbitrary positional values are rejected as an invalid subcommand.

### 3.2 Shell-hook convenience grammar

After loading `awswit init SHELL`, the shell function dispatches as follows.

| User input | Executable invocation |
|---|---|
| `awswit` | `command awswit activate` |
| `awswit PROFILE ...` | `command awswit activate PROFILE ...` |
| `awswit activate ...` | same arguments, patch captured |
| `awswit unset` | same arguments, patch captured |
| `exec`, `list`, `doctor`, `init`, `completions`, help/version | pass-through |

`awswit PROFILE` is therefore a shell Interface, not a binary Interface. Profile resolution remains exact and
case-sensitive. `--help`, `-h`, `--version`, or `-V` before an argument separator in a hook invocation is passed through
without applying a patch. A profile whose name collides with a subcommand is selected with explicit
`awswit activate PROFILE`. On Bash/Zsh/Fish, a profile name beginning with `-` is placed after `--`, for example
`awswit -- -h` or `awswit activate -- -h`. PowerShell consumes an unquoted `--` before a function receives its arguments;
therefore the preferred PowerShell form is `awswit activate --profile=-h`, or the separator must be quoted as a literal:
`awswit activate '--' -h` (and `awswit '--' -h` for the shortcut).

The same PowerShell parser behavior affects `exec`. For the ordinary
`awswit exec PROFILE -- COMMAND...` form, the hook reconstructs a consumed delimiter only when its closed exec-option grammar
finds one unique boundary. If the executable itself begins with `-`, or the boundary is otherwise ambiguous, the hook returns
2 with a fixed diagnostic and executes nothing. Quoting the delimiter—`awswit exec PROFILE '--' -option-shaped-command`—
preserves the explicit boundary. Arguments after an already identified command are not parsed as awswit options.

The generated function scopes `AWSWIT_HOOK=1` and `AWSWIT_SHELL` (`bash`, `zsh`, `fish`, or `powershell`) to captured
activation/unset and doctor subprocesses only. They are not exported into unrelated child shells. The executable checks the
first marker before current-shell activation or unset; `doctor` reports a wrapper-scoped marker and does not prove
hook/executable version equality.

### 3.3 Output channels

- stdout is machine output: activation frame, list/doctor data, generated hook, generated completion, or child stdout.
- TUI rendering, status, warning, and diagnostic use stderr.
- a broken stdout pipe is a normal success outcome and MUST NOT panic.
- raw argv, access/secret/session-token environment/file values, and `credential_process` command text MUST NOT be logged.

`list --format names` is the narrowest scripting Interface: one exact profile name per line, sorted by Unicode/Rust string
order. Human formats are for people and MAY evolve. JSON is versioned machine data, but its field compatibility is not a
stable 1.0 promise while the package remains pre-1.0.

Because warnings use stderr, a command may return success with a non-empty stderr—for example when best-effort history is
unavailable. Automation determines success from exit status and consumes stdout only as the selected machine format.

`completions SHELL` emits static clap completion for commands/options. Each `init SHELL` artifact composes that grammar with a
runtime completer that obtains exact profile candidates from the internal `command awswit list --format completion` feed. At the hook root it offers
subcommands and non-colliding profile shortcuts; a profile whose name equals a command remains available after `activate`.
After `activate` or `exec` it offers all profiles. Discovery failure yields no dynamic names rather than evaluating an error.
Bash consumes each profile as one literal line; Zsh/Fish use their completion value interfaces; PowerShell emits quoted
`CompletionResult` values. Shell APIs without a separate trustworthy display label cannot safely distinguish an exact name
containing a zero-cell scalar, so the completion feed omits those names. They remain available through exact activation,
the raw `names`/JSON interfaces, and the TUI, where zero-cell scalars are visibly escaped. Completion is convenience only and
never changes exact resolution. PowerShell captures awswit's UTF-8 machine output in an explicitly scoped UTF-8 decoder and
restores the caller's `Console.OutputEncoding` on success and failure. Its completer also restores the pre-completion
`$LASTEXITCODE`, so merely pressing Tab does not overwrite the status of the caller's last native command; `exec` remains a
direct child-output pass-through.
Because the runtime query is a bare `list`, candidates use environment/default source paths at completion time; the completer
does not interpret prospective command-local `--config-file` / `--credentials-file` options.

## 4. Source path resolution

Each source is resolved independently with this precedence:

```text
--config-file                    > AWS_CONFIG_FILE                 > ~/.aws/config
--credentials-file               > AWS_SHARED_CREDENTIALS_FILE     > ~/.aws/credentials
```

Rules:

1. Command-line paths and environment paths are intentional. Missing, unreadable, directory, or otherwise unopenable
   explicit sources are hard errors.
2. A missing OS-default source is an empty source, not an error.
3. Every selected path is converted lexically to an absolute path at invocation time without canonicalizing or following
   symlinks. This pins relative paths to the directory in which the catalog was actually loaded.
4. `activate` and `exec` propagate command-line paths. They also propagate an environment-selected path when its original
   value was relative, replacing it with the resolved absolute path. An absolute environment path is already inherited and a
   default path needs no variable, so neither is added to the patch.
5. A non-default path that is not representable as Unicode can be used by `exec` on platforms that support such paths,
   because it stays an `OsString`. An absolute non-Unicode environment path also needs no activation record and remains
   inherited. Any non-Unicode path that must cross the text shell protocol fails before patch application.
6. Path selection, original relativity, and propagation intent are snapshotted before catalog load. Parsing does not re-read
   path environment variables midway.

## 5. Profile catalog

### 5.1 Parsing and limits

The parser streams both files, bounds each source to 16 MiB, and bounds a physical line to 64 KiB. Profile/session names are
bounded to 1,024 bytes; retained metadata values are bounded to 4 KiB; one parse is bounded to 1,024 issues and 16,384
profile/SSO-session records. An oversized line or malformed section content becomes a safe issue and the affected section is
quarantined until the next section header. Exceeding a whole-source, issue, or entry budget is a hard `CONFIG_READ` failure
rather than returning a misleading partial catalog. The opened target must be a regular file. Unix deliberately follows a
final symlink, opens non-blocking, and validates the target type after open, so normal dotfile symlinks work while a direct
FIFO/device or symlink to one cannot stall the process. Windows rejects reparse-point sources and direct device/custom
namespaces (including named pipes) because the available open semantics do not provide the same pre-open non-blocking
guarantee. Ordinary drive/UNC paths and their extended-length forms remain supported.

Profile names and retained metadata reject terminal control characters and Unicode bidi control characters. Human output
also renders zero-cell scalars as visible `\u{...}` text without changing their exact machine value. Unknown keys and
non-profile sections are ignored for forward compatibility. Duplicate sections/metadata are diagnosed without making
iteration nondeterministic.

### 5.2 Credential-blind model

The credentials reader records only access-key, secret-key, and session-token presence; AWS defines other provider settings
in the config file, not the credentials file. `credential_process` command text, `login_session` value, MFA serial value,
web-identity token-file path, and static credential values are not stored in the catalog, rendered, serialized, or placed in
an error. Only the presence of `login_session` is retained. Input bytes are necessarily scanned to parse the file;
“credential-blind” does not mean the file is never read. SSO start URL/region/scopes, account/role, region, role ARN, source
names, and source paths are retained configured metadata. Presence-only static credential values are bounded by the 64 KiB
physical-line budget rather than the 4 KiB retained-metadata budget because AWS defines no fixed maximum session-token size.

Configured account IDs, profile names, paths, regions, role ARNs, and SSO URLs are not credentials, but they can still be
organizationally sensitive. Operators SHOULD review them before sharing doctor or JSON output.

### 5.3 Merge and ordering

The union of config and credentials section names forms the catalog. Same-name sections become one profile; a valid
credentials-only profile remains selectable. Metadata and known-key presence can be combined for that profile, but a static
access/secret tuple is validated independently within each physical config or credentials source. A partial tuple in either
source is an issue even when the other source contains the missing key: AWS consumers do not uniformly repair such a split,
so awswit does not manufacture a portable signing provider from the union. A `BTreeMap`-equivalent total ordering makes names
reproducible regardless of locale or hash seed.

### 5.4 Issues and provider graph

The catalog can report, without raw values:

- invalid encoding, oversized line, malformed section/property, property outside a section;
- unsafe profile/session name, invalid or duplicate metadata/section;
- missing or cyclic `source_profile`;
- missing or incomplete SSO metadata;
- mutually invalid role credential-source combinations and conflicting top-level credential providers, including AWS Login;
- partial static access-key tuple, identified against its config or credentials source rather than repaired cross-file.

Issues describe configured metadata; `doctor` is the diagnostic Interface. The catalog does not contact AWS and cannot
confirm credential validity, permission, account identity, token expiration, or `credential_process` behavior.

A modern SSO reference requires the named session plus its start URL and SSO region. Account ID and role name are accepted
only as a pair: both absent is the bearer-token form and both present is the credential form; a one-sided pair is incomplete.
Legacy inline SSO is considered complete only with start URL, SSO region, account ID, and role name. These are local
completeness checks, not login or token validation.

Profile resolution tracks two different facts: whether a record is locally activatable and whether it can provide signing
credentials to an upstream role. Complete static keys, `credential_process`, AWS Login, credential-form SSO, a valid web-
identity role, and a valid non-self role chain are source-capable. Config-only/region-only records and bearer-only modern SSO
remain locally activatable but are not source-capable. Every standalone `credential_source` without `role_arn` is
non-activatable with `credential_source_without_role_arn`: current SDKs reject or ignore it, so treating fallback credentials
as the requested source would be unsafe. A role which keeps complete
static keys in the same section and names itself as `source_profile` follows AWS CLI's top-level-only exception: it is locally
activatable, but another profile cannot use it as a source. An upstream edge to any locally selectable but non-source-capable
record makes the upstream profile invalid and emits `source_profile_not_credential_capable`.

A profile combining `role_arn` with `credential_source = Environment` is a separate awswit-specific invalid shape and emits
`environment_credential_source_cannot_be_selected_safely`. With generic environment-driven profile selection, retaining its
direct keys can bypass the role while clearing them removes its required source. An SDK-specific explicit-profile API may
behave differently, but awswit cannot prove that property for an arbitrary child.

AWS's assume-role guide shows a standalone `[profile B] credential_source=Ec2InstanceMetadata` referenced by profile A, but
the current AWS CLI/botocore source-profile builder does not resolve that shape and Go SDK v2 rejects it without a role.
awswit keeps B in doctor but excludes B and A from selection, prioritizing observed consumers over a misleading compatibility
claim. These classifications prove only local provider shape; they do not prove runtime credential validity.

The catalog retains parsed records and issues for diagnosis, but `list`, dynamic completion, and the TUI expose only profiles
whose complete transitive configuration is activatable. A named exact request still resolves against all parsed profile
records: if its own configuration or transitive `source_profile` chain is ambiguous, incomplete, missing, cyclic, or has a
provider conflict, it fails with `PROFILE_CONFIG_INVALID` rather than being reported as absent. Unsafe or unparseable section
names are not catalog records.

## 6. Selection TUI

### 6.1 Terminal contract

Interactive selection requires both stdin and stderr to be terminals. stdout may be captured by the hook because the TUI
never uses it. Before returning, `TerminalSession` attempts to restore every state it acquired: cursor visibility,
alternate screen, then raw mode.

On Unix, SIGINT, SIGTERM, and SIGHUP are converted into an ordinary restoration path, after which the default signal action
is re-established. Panic/unwind and I/O-error paths also run RAII cleanup. SIGKILL, process abort, kernel/terminal loss, and
power failure are outside the recoverable contract.

### 6.2 Initial selection and ranking

- Current-name precedence is a Unicode `AWS_PROFILE` value, otherwise a Unicode `AWS_DEFAULT_PROFILE` value. If that chosen
  name matches a catalog row, the row receives initial focus. An empty/unknown `AWS_PROFILE` does not fall through to
  `AWS_DEFAULT_PROFILE`, matching the precedence rule rather than guessing user intent.
- Without a current profile, no row is initially armed; bare Enter does nothing.
- Default order is favorite first, then store-local most-recent sequence, then profile name. A monotonic `USED#N` label makes
  the logical order visible; larger numbers are more recent and no wall-clock timestamp is exposed.
- A query performs case-insensitive fuzzy filtering/ranking only. Confirmation always returns the exact name of a visible
  catalog row.
- No match cannot be confirmed. Enter is also disabled when the terminal is too narrow to render the focused full profile
  name or too short to show a list row. Favorite mutation is disabled under the same condition. Compact mode asks the user
  to resize, and row actions become available after a sufficient resize.

### 6.3 Keys

| Key | Action |
|---|---|
| text | insert into fuzzy filter |
| Left / Right | move query cursor |
| Backspace / Delete | edit query |
| `Ctrl+U` | clear query |
| Up / Down, `Ctrl+K` / `Ctrl+J` | move focus |
| PageUp / PageDown | move by 10 rows |
| Home / End | first / last row |
| Enter | confirm focused row |
| `*` or `Ctrl+F` | toggle favorite |
| `Ctrl+P` | toggle preview |
| Esc | cancel |
| `Ctrl+C` | interrupt |

The query is bounded to 4 KiB. Optional preview is shown only when terminal dimensions permit. Text labels remain usable
with `NO_COLOR`, `TERM=dumb`, and without special fonts.

Configured provider type (including the presence-only `AWS Login` label), region, account ID, source, role ARN, and SSO
session are display hints, not verified identity.

## 7. Credential-override safety

Before activation or execution, awswit snapshots the **presence** of these variables:

```text
AWS_ACCESS_KEY_ID
AWS_ACCESS_KEY
AMAZON_ACCESS_KEY_ID
AWS_SECRET_ACCESS_KEY
AWS_SECRET_KEY
AMAZON_SECRET_ACCESS_KEY
AWS_SESSION_TOKEN
AWS_SECURITY_TOKEN
AMAZON_SESSION_TOKEN
AWS_WEB_IDENTITY_TOKEN_FILE
AWS_ROLE_ARN
AWS_ROLE_SESSION_NAME
AWS_CONTAINER_CREDENTIALS_RELATIVE_URI
AWS_CONTAINER_CREDENTIALS_FULL_URI
AWS_CONTAINER_AUTHORIZATION_TOKEN
AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE
AWS_EC2_METADATA_SERVICE_ENDPOINT
AWS_LOGIN_CACHE_DIRECTORY
AWS_CREDENTIAL_PROFILES_FILE
AWS_BEARER_TOKEN_BEDROCK
```

Their values are not retained or printed. The decision table is:

| Selected profile proof | Present variables | Default result |
|---|---|---|
| any selected profile | any standard/legacy/`AMAZON_*` direct access, secret, or session credential name | reject and name every present direct-key conflict; environment precedence can bypass role assumption |
| complete, acyclic chain explicitly using `EcsContainer` | container URI/authorization variables | allow the container-provider variables |
| same EcsContainer chain | direct, web-identity, or role variables | reject those conflicts |
| complete, acyclic chain explicitly using `Ec2InstanceMetadata` | `AWS_EC2_METADATA_SERVICE_ENDPOINT` | allow that endpoint setting |
| same Ec2InstanceMetadata chain | direct, web-identity, role, or container variables | reject those conflicts |
| complete, acyclic chain ending in an explicit `login_session` provider | `AWS_LOGIN_CACHE_DIRECTORY` | allow that cache-location setting |
| same AWS Login chain | direct, web-identity, role, container, or IMDS variables | reject those conflicts |
| no proven matching ECS / IMDS / AWS Login provider | any listed non-direct provider variable | reject and name conflicts |
| any chain | `AWS_CREDENTIAL_PROFILES_FILE` | reject; its legacy credential source was not catalogued |
| any chain | `AWS_BEARER_TOKEN_BEDROCK` | reject; it is a service-specific credential independent of profile proof |
| missing/cyclic/invalid provider chain | any listed variable | no provider exemption; fail closed |

`--clear-credential-overrides` changes a non-empty conflict set into `UNSET` operations for every detected name, including
partial or duplicate direct-key names. With `activate`, conflict removal affects the current shell; with `exec`, only the
command environment. awswit never clears a credential variable merely because a new profile was selected.

Alias inputs are fail-closed: every present direct-key alias is rejected, while relative with full container credential URI
or inline with file-based container authorization token are each rejected when both forms are present. Provider precedence
is not guessed from an ambiguous set.

This preflight reduces known precedence mismatches; it does not validate the IMDS endpoint or login-cache path/value, model
every language SDK, application-level override, CLI `--profile`, custom provider, or credential already cached inside a
long-lived process.

Java SDK v1's `AWS_CREDENTIAL_PROFILES_FILE` is detected and can be explicitly cleared, but that SDK does not consume the
standard `AWS_SHARED_CREDENTIALS_FILE`. Therefore awswit does not claim Java v1 non-default path compatibility: configure the
consumer's credential provider explicitly, or use the default shared path. This legacy limitation does not change the
standard source precedence in section 4.

## 8. Environment patch

### 8.1 Managed activation values

Successful selection builds one coherent patch:

- set `AWS_PROFILE`, `AWS_DEFAULT_PROFILE`, and `AWSWIT_PROFILE` to the same exact selection;
- set `AWS_REGION` and `AWS_DEFAULT_REGION` to `--region`, otherwise configured profile region;
- if neither region exists, unset both region variables;
- set `AWS_CONFIG_FILE` / `AWS_SHARED_CREDENTIALS_FILE` for the corresponding command-line source, and for an originally
  relative environment source after pinning it to its lexical absolute path;
- unset only credential conflicts explicitly approved with `--clear-credential-overrides`.

`awswit unset` unsets exactly `AWS_PROFILE`, `AWS_DEFAULT_PROFILE`, `AWSWIT_PROFILE`, `AWS_REGION`, and
`AWS_DEFAULT_REGION`. It deliberately does not clear credential variables or source-path variables.
It also does not set or clear consumer behavior flags such as `AWS_SDK_LOAD_CONFIG`; legacy Go SDK v1 applications must
enable shared-config loading explicitly or migrate to v2.

### 8.2 Protocol grammar

stdout carries a complete version-1 frame:

```text
AWSWIT-PATCH 1 ACTIVATE\n
(SET NAME=VALUE\n | UNSET NAME\n)+
AWSWIT-COMMIT\n
```

or:

```text
AWSWIT-PATCH 1 UNSET\n
UNSET AWS_PROFILE\n
UNSET AWS_DEFAULT_PROFILE\n
UNSET AWSWIT_PROFILE\n
UNSET AWS_REGION\n
UNSET AWS_DEFAULT_REGION\n
AWSWIT-COMMIT\n
```

For `ACTIVATE`, settable names are the three profile variables, two region variables, and two source-path variables.
Credential-provider names are unsettable only. For `UNSET`, all five exact owned names MUST appear once as `UNSET`.

The three profile values MUST be equal. The two region operations MUST both be `SET` with equal values or both `UNSET`.
Names are closed allow-lists; duplicate, unknown, empty, truncated, and post-commit records are invalid. The executable's
encoder/decoder also rejects carriage return, NUL, control characters, and bidi controls. `=` and ordinary shell
metacharacters are part of the value after the first separator.

### 8.3 Shell transaction

Each generated hook captures the full frame, validates its header, record shape/name allow-list, uniqueness, semantic
coherence, basic control-character safety, and final commit marker, then applies allow-listed records. Fish preserves each
`SET` record's value after the first `=` as one data value. Bash and Zsh
additionally preflight every target's variable attributes: an absent, plain scalar, or exported scalar is allowed; readonly,
integer, array, case-transforming, tied, or other special attributes are rejected before any operation. No profile- or path-
derived value is passed to `eval`, reparsed as command text, or interpolated into generated shell source. Hooks are paired
with the same-version executable; they are not a trust wrapper for an already compromised replacement binary.

Loading the static Bash/Zsh hook with `eval "$(awswit init SHELL)"` or the static PowerShell hook with
`Invoke-Expression ((awswit init powershell) -join [Environment]::NewLine)` evaluates code generated by the installed
executable once; activation responses themselves are data frames and are not evaluated. PowerShell must join native stdout
lines before that one evaluation; piping them directly to `Invoke-Expression` does not define the multiline function
reliably.

PowerShell temporarily sets `Console.OutputEncoding` to strict UTF-8 only while capturing awswit's protocol, doctor, or
dynamic-completion stdout and restores the prior encoding in `finally`. This prevents legacy console code pages from
changing an exact Unicode profile while keeping arbitrary `exec` child output under the caller/child encoding contract.

PowerShell's parser removes an unquoted `--` before calling a function. The hook uses the closed CLI grammar to restore the
ordinary `exec PROFILE -- COMMAND` boundary only when unique; it rejects an option-shaped executable or any other ambiguous
case before invoking the binary. A quoted literal `'--'` remains authoritative. Activation of a leading-hyphen profile uses
`--profile=NAME` or the same quoted-literal form; a consumed delimiter is never guessed for activation.

Before invoking any current-shell activation path—including bare TUI, shortcut, and explicit `activate`—the PowerShell hook
checks whether a visible `$StoredAWSCredentials` variable has a non-null value. If so it emits a fixed remediation naming
`Clear-AWSCredential`, does not call the executable, and leaves the environment unchanged. It checks presence only and never
reads credential fields or calls `ToString`. A variable declared `Private` in the caller scope is not visible to the hook
function, and a non-AWS variable with the same name produces a safe false positive. `exec` is not blocked because an external
process cannot inherit the PowerShell object; application-level providers remain a consumer responsibility.

Any hook/protocol validation failure leaves the parent environment unchanged. A hook from an incompatible future protocol
version rejects the frame rather than partially applying it.

Fish `UNSET` removes an exported global value and creates a zero-element unexported global shadow. It does not erase a
persistent universal variable; the shadow prevents that universal value from reaching child processes for the rest of the
current Fish session. Operators must manage any persistent universal variable separately.

## 9. `exec` semantics

`exec PROFILE -- COMMAND...` requires an exact profile, performs the same source and override checks, and constructs the
command from an executable plus argument vector. A leading-hyphen profile uses
`exec --profile=NAME -- COMMAND...`; the named escape hatch avoids overloading the command separator. It never invokes a
shell to reinterpret command text.

In PowerShell, an unquoted delimiter normally disappears at the function-call boundary. The generated hook restores it for
the unambiguous common form, including a named leading-hyphen profile, but fails with status 2 if the child executable begins
with `-` or another unique boundary cannot be proven. Use a quoted literal delimiter in that case, for example
`awswit exec --profile=-h '--' -option-shaped-command`.

- Unix: `exec(2)`-style process replacement preserves inherited streams and native child exit/signal semantics.
- Unix resolves a bare executable name through inherited `PATH`, constructs an explicit environment vector, and calls
  `execve` directly. It does not use the historical `ENOEXEC` fallback that asks a shell to reinterpret an unrecognized
  file; scripts therefore need a valid shebang and executable permission.
- If `PATH` is absent, a bare executable name returns 127 rather than searching an implicit default or the current directory.
  An explicitly present empty `PATH` retains POSIX empty-component behavior and can search the current directory.
- Windows: starts the child with inherited console streams, waits, and returns its numeric status; statuses outside the
  portable 0–255 range or without a numeric code become 1, and signal parity is not promised.
- Windows rejects a `.bat` or `.cmd` executable with 126 because the standard process API would implicitly reinterpret it
  through `cmd.exe`. Callers who intentionally need batch syntax must pass `cmd.exe` explicitly as the executable and accept
  its shell semantics.
- executable not found returns 127; permission denied/not executable returns 126.

Only variables named by the validated patch differ in the child. The parent process environment is unchanged.

## 10. Preference history

History contains profile name, favorite flag, saturating use count, a store-local logical recency sequence, and an
informational timestamp. It contains no AWS credential or provider output and is not an audit log.

- history schema version: 1;
- maximum retained entries: 512;
- maximum accepted history file: 1 MiB;
- ranking: favorite, latest logical sequence, use count in storage ranking, then name;
- entries with neither favorite nor use are pruned; ranked entries are bounded to 512.
- history and lock paths must resolve to regular files; Unix opens use non-blocking/no-follow flags before post-open checks
  so a FIFO or lock symlink cannot stall or redirect the best-effort store.

One invocation's catalog is not proof that a profile was globally deleted: users can switch between explicit config files.
Therefore a commit does not delete used/favorite entries merely because they are absent from its current catalog. The TUI
joins history onto current catalog profiles only, so retained names from another catalog never become selectable by history.

Store commits acquire an exclusive sibling lock, re-read the latest state, merge the invocation delta, write a unique
same-directory temporary file, synchronize it, and atomically replace the target. Unix history/lock/backup files are forced
to mode `0600`; the parent directory is synchronized where the platform supports it. Other platforms synchronize the file
but have no portable directory-fsync guarantee.

Malformed or oversized history is preserved as a unique `history.json.corrupt-*` sibling before resetting when preservation
succeeds; if it does not, the original is left for recovery and a warning is emitted. A schema newer than the executable
understands is left untouched. Legacy schema is migrated best-effort. Read/write/lock failures produce
sanitized `HISTORY_*` warnings and MUST NOT weaken credential safety. Preference storage is non-authoritative: selection and
execution remain valid without it.

Favorite changes are persisted on selection and ordinary Esc cancellation. Signal termination does not promise to persist
the in-flight favorite delta.

## 11. `list` and `doctor`

`list` loads the local catalog and emits only its activatable subset in one of:

- `human`: tabular configured metadata;
- `names`: deterministic exact names only;
- `json`: structured catalog/profile metadata for machine use.

`doctor` is explicitly offline. It reports selected source paths and origins, total parsed profile-record count (including
records excluded from `list`), the same `AWS_PROFILE`-then-
`AWS_DEFAULT_PROFILE` current-name observation used by the TUI,
hook-marker presence, names of credential overrides, and safe catalog issues. It does **not**:

- call STS, IAM, SSO, EC2/ECS metadata, or any AWS API;
- verify that the current shell function is the same version as the executable;
- execute `credential_process`;
- verify authentication, permissions, account identity, token expiration, network, or clock skew.

Human doctor output includes one location plus compact, value-blind JSON descriptor for every safe catalog issue; JSON is
the full structured automation form. A zero exit means local inspection completed, not that AWS authentication will succeed.

`list`, `doctor`, activation discovery, and the TUI do not initiate network calls. A command explicitly supplied to `exec`
is outside this guarantee and may use the network normally.

### 11.1 Current JSON envelopes

Both JSON commands emit `schema_version: 1`. The current `list` envelope is:

```json
{
  "schema_version": 1,
  "profiles": [
    {
      "name": "production",
      "region": "ap-northeast-1",
      "role_arn": null,
      "source_profile": null,
      "credential_source": null,
      "has_mfa_serial": false,
      "has_credential_process": false,
      "has_login_session": false,
      "has_web_identity_token_file": false,
      "static_credentials": {
        "access_key_id": false,
        "secret_access_key": false,
        "session_token": false
      },
      "has_config_section": true,
      "has_credentials_section": false,
      "sso": null
    }
  ]
}
```

The booleans describe key/configuration presence, never credential values or the `login_session` identity. SSO is `null` or an object tagged by
`"mode": "modern" | "legacy"` with configured optional metadata.

The current doctor envelope is:

```json
{
  "schema_version": 1,
  "offline": true,
  "sources": {
    "config": {
      "path": "/home/example/.aws/config",
      "path_is_unicode": true,
      "origin": "default"
    },
    "credentials": {
      "path": "/home/example/.aws/credentials",
      "path_is_unicode": true,
      "origin": "default"
    }
  },
  "profile_count": 1,
  "current_profile": null,
  "hook_detected": false,
  "credential_overrides": [],
  "issues": []
}
```

Each issue contains `source`, lossy display `path`, `path_is_unicode`, optional `line`, and a nested tagged `kind` with a
fixed `code` plus safe profile/session/enum details where applicable. Consumers MUST check `schema_version`; pre-1.0 minor
versions may add or revise fields.

## 12. Exit and diagnostic contract

Application runtime errors use `awswit[CODE]: message` on stderr, optionally followed by `hint:`. Untrusted strings routed
through that diagnostic layer are escaped before terminal output. Clap owns grammar and successful help/version rendering.
On a syntax failure, awswit deliberately replaces clap's argument-echoing error with fixed `CLI_INVALID` text and a help
hint, so untrusted argv cannot forge terminal output. Generated hooks have a separate fixed, un-coded rejection message for
an invalid captured frame; they do not print frame contents. Principal outcomes are:

| Outcome | Process result | stdout |
|---|---:|---|
| ordinary success | 0 | command-specific data |
| broken consumer pipe | 0 | possibly partial |
| Esc cancellation | 130 | empty activation output |
| Unix SIGINT/SIGTERM/SIGHUP in TUI | native default signal result after restoration | empty activation output |
| validation/config/safety/terminal failure | 1 | empty activation output |
| CLI syntax failure | 2 | empty; fixed `CLI_INVALID` + help hint on stderr |
| command not found / not executable | 127 / 126 | child/process dependent |
| successful Unix `exec` | command-native | command-native |
| successful Windows `exec` | child status 0..255; otherwise 1 | command-native |

No guarantee is made that arbitrary child output is secret-free; `exec` deliberately gives the command its normal streams.

## 13. Explicit limitations

- no credential storage, retrieval, refresh, rotation, or secure keystore;
- no STS, MFA prompt, SSO login, or token-cache management;
- no AWS identity or permission verification;
- no daemon, plugin runtime, external fzf, or awswit-specific config file;
- no noninteractive fuzzy profile substitution;
- no automatic insertion of awswit hook/completion lines into shell rc files; executable installers may manage PATH only as documented;
- no recovery from SIGKILL or host/terminal failure;
- no promise that setting `AWS_PROFILE` changes credentials already cached in an existing long-lived process;
- no implicit `AWS_SDK_LOAD_CONFIG` mutation for EOL Go SDK v1; that consumer must enable shared config explicitly;
- no credential-blind enumeration of the Windows .NET SDK Store or proof against application-level credential providers;
- `completions SHELL` is static; dynamic profile names require loading the matching `init SHELL` hook.

## 14. Verification mapping

The closest executable acceptance evidence lives in `tests/cli_contract.rs`:

| Contract | Evidence |
|---|---|
| exact-only activation | `exact_activation_never_substitutes_a_typo` |
| file-local static credential completeness | `static_credential_tuples_must_be_complete_within_each_source_file` |
| override fail-closed and explicit clear | `credential_overrides_fail_closed_without_reading_their_values` |
| offline doctor/value non-disclosure | `doctor_is_offline_and_reports_only_override_names` |
| child-only argv-safe execution | `exec_changes_only_the_child_and_preserves_argv`, `exec_does_not_reinterpret_shell_metacharacters` |
| transactional shell data handling | `bash_hook_treats_profile_metacharacters_as_literal_data`, truncated-frame tests |
| Bash/Zsh variable-attribute preflight | `bash_rejects_special_variable_attributes_without_partial_changes`, Zsh counterpart |
| static/dynamic completion composition and literal handling | combined artifacts plus Bash/Zsh/Fish/PowerShell runtime option, collision, literal-space, leading-hyphen, Unicode capture, zero-cell omission, and PowerShell `$LASTEXITCODE` preservation checks |
| PowerShell exec boundary | runtime checks for unquoted common-form reconstruction, quoted delimiter forwarding, and fail-closed option-shaped executables |
| source artifact identity | exact canonical member/directory set plus byte identity for every release policy, document, source, and contract fixture |
| broken-pipe handling | `list_broken_pipe_never_panics` |
| captured stdout + terminal restoration | Linux PTY Escape and signal tests |

Parser, protocol, history concurrency/corruption, TUI state, small-terminal rendering, and terminal signal mapping have
module-level tests next to their implementations. See [product requirements](../requirements/product-requirements.md) for
items still requiring release-level evidence.
