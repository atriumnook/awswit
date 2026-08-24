# awswit architecture

状態: `0.1` redesign の accepted architecture
更新日: 2026-08-24

## Decision

awswit は credential manager ではなく、AWS profile selection workflow を一つの深い
`Application` Module に閉じ込める。外部 Interface は executable の CLI、出力、終了状態と、生成する
shell hook である。Rust crate の内部 Module を汎用 profile-management library として公開しない。

この形を選ぶ理由は、profile catalog、exact selection、credential override policy、history、terminal、
shell transaction、child execution の順序が安全性そのものだからである。各処理を呼び出し側へ露出すると、
preflight や transaction を飛ばす経路が増える。`entry(arguments) -> ExitCode` の小さい Interface の背後に
順序を集中させ、変更と検証の locality を得る。

## Runtime structure

```text
argv
  |
  v
CLI parser ---- static output ----------------------> stdout
  |             (help/init/completions)
  v
Application workflow
  |-- source resolution
  |-- ProfileCatalog ---- local AWS files
  |-- exact lookup OR TUI chooser ---- stdin/stderr TTY
  |-- SafetyPolicy ---- environment variable presence
  |-- PreferenceStore (best-effort) ---- local history
  |-- EnvironmentPatch
  |      |-- versioned frame ---- stdout ---- shell hook ---- parent shell
  |      `-- ExecutionPlan ---- OS process adapter ---- command process
  `-- doctor/list ---- stdout

diagnostic/warning/status ---------------------------> stderr
```

There is no network adapter. AWS CLI / SDK credential resolution happens after awswit returns or after it replaces/starts
the requested command.

## Module responsibilities

| Module | Interface responsibility | Hidden implementation |
|---|---|---|
| `lib` | parse one process invocation and return disposition | diagnostic routing and Unix signal re-termination |
| `cli` | closed command grammar | clap declarations and static completion generation |
| `application` | own workflow order and result | path/catalog/chooser/safety/history/patch orchestration |
| `catalog` | immutable profiles + sanitized issues | bounded INI scan, config/credentials merge, SSO join, graph checks |
| `safety` | classify override conflicts from variable presence | ECS/IMDS/Login intent exceptions and unconditional direct-key/Bedrock/legacy-path rejection |
| `activation` | construct/decode coherent allow-listed patch | protocol grammar, unsafe text checks, variable ownership |
| `tui` | choose one exact profile and return favorite delta | fuzzy filter, layout, terminal RAII, signals |
| `history` | load/commit preference delta at explicit path | locking, latest-state merge, schema migration, atomic replace |
| `process` | execute one already-safe plan | Unix replacement / Windows spawn-wait |
| `output` + `error` | safe channel and diagnostic contract | broken-pipe normalization, terminal-text escaping |
| generated hook | validate and apply one complete frame | shell-specific buffering and assignment-as-data |
| hook completer | offer commands and visually unambiguous current exact names | internal filtered list feed, shell-native completion values |

These Modules are private to the crate. “Interface” here includes invariants, error modes, output channels, and ordering, not
only Rust function signatures.

## Trust and data boundaries

### Untrusted inputs

- argv and inherited environment;
- config/credentials file bytes, including names, paths, and metadata;
- history file bytes;
- terminal events and dimensions;
- child executable and argv;
- activation stdout as received by a shell hook.

All strings crossing a terminal or shell seam are treated as data. Profile names and retained metadata reject terminal/bidi
controls. Diagnostics escape untrusted text. The shell protocol has a closed name allow-list and whole-frame commit.

### Sensitive data boundary

awswit opens AWS credentials files but its domain model records only known-key presence. Access-key, secret-key, session-
token values, `credential_process` command text, and the AWS Login `login_session` identity do not cross the catalog parser
seam. Other retained configured metadata is enumerated in the specification. Environment override policy retains only enum
names; its decisions do not depend on a credential environment value.

This is a minimization guarantee, not a sandbox guarantee: a compromised executable can read anything its OS identity can
read, and an executed child owns its own output. Distribution provenance, file permissions, and OS controls remain part of
the threat model.

### Authoritative vs best-effort state

- AWS shared configuration is authoritative for parsed profile names and configured metadata; catalog validation determines
  the activatable subset exposed by list/completion/TUI.
- exact selected profile plus the current environment snapshot is authoritative for a single workflow.
- preference history is non-authoritative. It may change ordering but MUST NOT make an absent or invalid profile selectable
  or weaken a safety rejection.
- displayed account/provider metadata is descriptive, never proof of current AWS identity.

## Workflow invariants

### Static commands

`init`, `completions`, help, and version are handled without opening AWS or history files. This makes installation and
recovery possible even with broken local configuration.

### Activation

The workflow is ordered so that no failure can partially mutate the parent shell:

1. parse argv without logging it;
2. require the shell-hook marker before config I/O;
3. snapshot source origin/relativity and resolve each path lexically to an absolute path;
4. build one immutable deterministic catalog;
5. obtain an exact selection from exact lookup or TUI;
6. restore terminal state before continuing;
7. validate provider configuration and credential override policy;
8. construct and internally decode the entire activation frame;
9. merge history/favorite delta best-effort;
10. write one committed frame to stdout;
11. shell hook validates the entire frame again and then applies it.

The executable cannot mutate its parent; only step 11 can. An empty, invalid, truncated, duplicate, incoherent, or
unsupported frame leaves the parent unchanged. A history warning does not turn a rejected safety decision into success.

### Execution

`exec` uses the same source, exact lookup, provider, and override decisions. It then turns the validated patch into an
`ExecutionPlan`. Command text is never passed through a shell. Unix resolves `PATH` without an `ENOEXEC` shell fallback and
replaces the awswit process with direct `execve`; Windows inherits the console, waits, and returns a portable numeric status.
PowerShell's function parser can consume an unquoted `--`; its hook reconstructs the exec boundary only when the closed CLI
grammar proves one unique split and rejects ambiguous option-shaped executables before process launch.

### Cancel and signal

Ordinary Esc cancellation returns no patch. Favorite delta may be committed because it is non-authoritative and was an
explicit UI action. On Unix termination signals, the terminal is restored before the default signal action is re-emulated;
in-flight preferences are not part of the signal guarantee.

## ProfileCatalog design

`ProfileCatalog` is deep because callers provide two resolved paths and receive deterministic profiles plus safe issues;
they do not learn the AWS INI variants, recovery state machine, SSO-session join, credentials merge, or graph traversal.

Implementation properties:

- stream input with bounded allocation per line;
- quarantine a malformed section and recover at the next header;
- merge config and credentials profile identity/metadata without materializing secrets, while validating each physical
  source's static access/secret tuple independently so split partial keys cannot masquerade as a portable provider;
- model provider hints as orthogonal metadata rather than a false exclusive identity type;
- join modern SSO session metadata, retain legacy SSO form, and classify AWS Login from `login_session` presence only;
- diagnose incomplete static tuples, source references/cycles, incomplete SSO, conflicting top-level providers, invalid
  role-provider combinations, and a locally selectable node that cannot provide signing credentials to an upstream role;
- expose only transitively activatable profiles to list/completion/TUI while retaining all safe issues for doctor;
- separate `activatable` from `usable_as_source`: config-only and bearer-only SSO records may be selected locally but an
  upstream `source_profile` edge to them is rejected; the AWS CLI-compatible self-static role exception is selectable only
  as its own top-level target;
- reject every standalone `credential_source` without `role_arn`, because current SDKs either reject or ignore that shape and
  can fall through to a different identity;
- reject `role_arn + credential_source = Environment` as an awswit-specific unsafe selection shape: no generic
  `AWS_PROFILE`-driven child contract can both retain its direct source keys and prove that the consumer will assume the role;
- use stable ordered maps/sets so output and selection do not depend on hash iteration.

Path resolution remains outside the Module. `SourcePaths` also carries whether a resolved path must be propagated: every
command-line override and only an originally relative environment path. A catalog load is therefore reproducible from its
input, and a later `cd` cannot retarget a relative AWS path variable to a file other than the one that was inspected.

## SafetyPolicy design

The SafetyPolicy Interface takes only a proof about the selected source chain and a set of present credential-variable
names. It has two results: safe, or a set of names that conflict.

Default behavior is rejection. `--clear-credential-overrides` is an explicit policy choice made by the caller; the safety
Module itself does not mutate anything. Every standard, legacy, or `AMAZON_*` direct access/secret/session credential name is
always a conflict—even for `credential_source = Environment`. In AWS CLI/botocore's environment-driven selection path, the
environment provider precedes assume-role resolution, so treating that tuple as an intended role source could silently skip
the selected role. The catalog therefore excludes that role shape before policy assessment; it is a tool-safety limitation,
not a claim that every explicit SDK session API rejects the AWS configuration. Container URI/authorization variables may be exempted only for a complete chain that explicitly uses
`credential_source = EcsContainer`; an IMDS endpoint override may be exempted only for a complete chain explicitly using
`credential_source = Ec2InstanceMetadata`; an alternate login cache may be exempted only for a complete chain ending in an
explicit `login_session` provider. A Bedrock bearer token is always a conflict because it is a service-specific credential
outside those chain proofs. A legacy SDK-specific credential-file override is also always a conflict because its source was
not the catalog input. The model does not infer a provider from ambient variables and deliberately does not guess that web-
identity or mismatched provider variables belong to the selected profile.

PowerShell has a second, non-environment precedence seam. The generated PowerShell hook checks only whether a visible,
non-null `$StoredAWSCredentials` session variable exists before current-shell activation and refuses to invoke the selector
when it does. It never dereferences or renders the credential object. This guard cannot observe a caller-private variable,
and `exec` does not inherit a PowerShell session object; both limits are documented rather than hidden behind unsafe
reflection. Windows .NET SDK Store discovery remains outside the executable because no stable, dependency-free name-only API
meets the credential-blind boundary.

## Activation protocol design

`EnvironmentPatch` prevents partial construction: callers request a complete profile/region/path/conflict plan and receive
coherent operations. Profile triples and region pairs are generated together. Encoding rejects text that the protocol
cannot represent, while the process adapter retains native `OsString` values.

The shell seam uses a small versioned data protocol instead of generating shell-specific export statements. This gives four
shell adapters the same semantic oracle and removes profile-derived code generation. A final commit marker plus semantic
validation makes application transactional from the parent shell's perspective.

PowerShell additionally has an argv seam before the protocol: the engine removes an unquoted `--` when invoking a function.
The adapter owns a minimal recognizer for the closed `exec` option grammar, restores a missing delimiter only after a unique
profile/command split, and otherwise fails with a fixed message. It does not become a second general CLI parser or infer an
activation separator; quoted literal delimiters and named profile options remain the explicit escape hatches.

## Terminal design

The TUI accepts only display-safe `ViewProfile` values; it cannot read catalog files, inspect credentials, or persist
history. Terminal controls are rejected before the view boundary and zero-cell scalars are rendered as explicit Unicode
escapes while the exact underlying name remains the selection value. Its result contains either an exact visible name,
cancellation, or signal, plus favorite changes.

`TerminalSession` owns raw mode, alternate screen, cursor visibility, and signal lease. Phase flags are set before fallible
mutations so cleanup can safely retry uncertain partial initialization. stderr is the rendering backend, keeping stdout free
for the hook frame.

Fuzzy matching is intentionally inside the chooser: it changes only which existing row is visible/focused. It is absent
from exact CLI resolution.

## PreferenceStore design

The store Interface accepts only an invocation delta. It does not accept one catalog as global deletion evidence and hides
concurrency and durability mechanics:

1. acquire one exclusive lock;
2. re-read the newest file under that lock;
3. merge favorite and one selection event;
4. prune empty entries and enforce the global bound without deleting another explicit catalog's used/favorite names;
5. write and sync a unique same-directory temporary file;
6. atomically replace the target;
7. sync the parent directory where portable.

This latest-state merge prevents independent invocations from overwriting unrelated updates. Corruption is preserved under a
unique sibling name. Newer schemas are not overwritten by an older executable. Unix file mode is `0600`; directory ACL and
non-Unix permission enforcement remain OS/operator responsibilities.

## Deliberate omissions

The architecture excludes:

- credential/key/token ownership, STS, MFA, SSO login, refresh, and cache;
- external `fzf` and multiple chooser implementations;
- noninteractive fuzzy/prefix/edit-distance profile resolution;
- daemon, metadata server, plugin runtime, telemetry, and network identity checks;
- awswit TOML settings for matching, color, or default region;
- shell auto-detection and automatic insertion of awswit hook/completion lines into rc files;
- an external completion daemon or duplicate profile parser (`completions SHELL` is static; hook completion reuses `list`);
- speculative traits for clock, filesystem, environment, parser, or history serialization;
- public exposure of internal Rust Modules.

These are omitted because they enlarge the safety surface without improving the core profile-selection guarantee. A new
seam should be introduced only when a second real adapter exists and the Interface reduces caller knowledge.

## Known guarantee limits

- SIGKILL/abort cannot run terminal cleanup.
- Windows has no Unix signal-equivalence contract and no portable directory fsync.
- a long-lived AWS SDK process may retain credentials acquired before environment switching;
- shell hooks and executable should be version-matched during the `0.0.x` protocol period;
- JSON schema and dynamic completion are not declared stable pre-1.0;
- local inspection cannot prove remote AWS identity, access, or credential freshness.

The exact user-visible contract is specified in [specification.md](specification.md); operational recovery is in the
[runbook](../operations/runbook.md).
