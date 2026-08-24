use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::process::{Command, Output};

use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    config: PathBuf,
    credentials: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("create fixture directory");
        let config = root.path().join("config");
        let credentials = root.path().join("credentials");
        fs::write(
            &config,
            r#"[default]
region = us-east-1

[profile dev]
region = ap-northeast-1
role_arn = arn:aws:iam::111122223333:role/Developer
source_profile = default

[profile modern-sso]
sso_session = company
sso_account_id = 444455556666
sso_role_name = ReadOnly

[sso-session company]
sso_start_url = https://example.awsapps.com/start
sso_region = ap-northeast-1
"#,
        )
        .expect("write config fixture");
        fs::write(
            &credentials,
            "[default]\naws_access_key_id = SECRET-SENTINEL\naws_secret_access_key = SECRET-SENTINEL\n",
        )
        .expect("write credentials fixture");
        Self {
            root,
            config,
            credentials,
        }
    }

    fn command(&self) -> Command {
        let mut command = clean_command();
        command
            .env("HOME", self.root.path())
            .env("USERPROFILE", self.root.path())
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_STATE_HOME", self.root.path().join("state"));
        command
    }

    fn source_args(&self) -> [OsString; 4] {
        [
            OsString::from("--config-file"),
            self.config.as_os_str().to_owned(),
            OsString::from("--credentials-file"),
            self.credentials.as_os_str().to_owned(),
        ]
    }

    fn run(&self, arguments: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Output {
        self.command().args(arguments).output().expect("run awswit")
    }
}

fn clean_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_awswit"));
    let path = std::env::var_os("PATH").unwrap_or_default();
    command
        .env_clear()
        .env("PATH", path)
        .env("TERM", "xterm-256color");
    command
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_and_version_are_static_and_successful() {
    let help = clean_command().arg("--help").output().expect("run help");
    assert!(help.status.success());
    let help = text(&help.stdout);
    assert!(help.contains("activate"));
    assert!(help.contains("exec"));
    assert!(help.contains("doctor"));
    assert!(help.contains("awswit init"));

    let version = clean_command()
        .arg("--version")
        .output()
        .expect("run version");
    assert!(version.status.success());
    assert_eq!(
        text(&version.stdout).trim(),
        format!("awswit {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn every_init_artifact_combines_static_options_with_dynamic_profiles() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let output = clean_command()
            .args(["init", shell])
            .output()
            .expect("generate combined init artifact");
        assert!(output.status.success(), "{shell}: {}", text(&output.stderr));
        let artifact = text(&output.stdout);
        let has_region_option = if shell == "fish" {
            artifact.contains("-l region")
        } else {
            artifact.contains("--region")
        };
        assert!(has_region_option, "{shell} lost static options");
        let has_dynamic_profile_completion = if shell == "powershell" {
            artifact.contains("Invoke-AwswitUtf8Native")
                && artifact.contains("@('list', '--format', 'completion')")
        } else {
            artifact.contains("list --format completion")
        };
        assert!(
            has_dynamic_profile_completion,
            "{shell} lost dynamic profile completion"
        );
    }
}

#[test]
fn powershell_hook_guards_activation_from_stored_session_credentials() {
    let output = clean_command()
        .args(["init", "powershell"])
        .output()
        .expect("generate PowerShell init artifact");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let artifact = text(&output.stdout);

    for contract in [
        "Microsoft.PowerShell.Utility\\Get-Variable",
        "-Name StoredAWSCredentials",
        "[object]::ReferenceEquals($null, $storedVariable.Value)",
        "if ($activationRequest -and (Test-AwswitStoredSessionCredentials))",
        "Clear-AWSCredential",
    ] {
        assert!(
            artifact.contains(contract),
            "PowerShell stored-session credential guard lost {contract}"
        );
    }
}

#[test]
fn powershell_hook_reconstructs_only_unambiguous_exec_separators() {
    let output = clean_command()
        .args(["init", "powershell"])
        .output()
        .expect("generate PowerShell init artifact");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let artifact = text(&output.stdout);

    for contract in [
        "Find-AwswitPowerShellExecCommandIndex",
        "$Arguments -cnotcontains '--'",
        "$Arguments[0] -ceq 'exec'",
        "Write-AwswitPowerShellExecSeparatorError",
        "quote '--' and retry",
    ] {
        assert!(
            artifact.contains(contract),
            "PowerShell exec boundary contract lost {contract}"
        );
    }
}

#[test]
fn syntax_errors_never_echo_untrusted_terminal_controls() {
    let forged = "bad\u{1b}[31mFORGED\nLINE";
    let output = clean_command()
        .arg(forged)
        .output()
        .expect("run invalid syntax");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = text(&output.stderr);
    assert_eq!(
        stderr,
        "awswit[CLI_INVALID]: invalid command line\nhint: run `awswit --help` for supported syntax\n"
    );
    assert!(!stderr.contains('\u{1b}'));
    assert!(!stderr.contains("FORGED"));
}

#[test]
fn list_names_is_deterministic_and_credentials_are_never_printed() {
    let fixture = Fixture::new();
    let mut arguments = vec![
        OsString::from("list"),
        OsString::from("--format"),
        OsString::from("names"),
    ];
    arguments.extend(fixture.source_args());
    let output = fixture.run(arguments);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "default\ndev\nmodern-sso\n");
    assert!(!text(&output.stdout).contains("SECRET-SENTINEL"));
    assert!(!text(&output.stderr).contains("SECRET-SENTINEL"));
}

#[test]
fn human_and_json_views_expose_zero_cell_text_without_changing_exact_names() {
    let fixture = Fixture::new();
    let invisible_name = "pro\u{200b}d";
    let invisible_region = "us\u{200d}-east-1";
    fs::write(
        &fixture.config,
        format!("[profile {invisible_name}]\nregion = {invisible_region}\n"),
    )
    .expect("write zero-cell profile fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let names = fixture.run(
        [
            OsString::from("list"),
            OsString::from("--format"),
            OsString::from("names"),
        ]
        .into_iter()
        .chain(fixture.source_args()),
    );
    assert!(names.status.success(), "{}", text(&names.stderr));
    assert_eq!(text(&names.stdout), format!("{invisible_name}\n"));

    let completion = fixture.run(
        [
            OsString::from("list"),
            OsString::from("--format"),
            OsString::from("completion"),
        ]
        .into_iter()
        .chain(fixture.source_args()),
    );
    assert!(completion.status.success(), "{}", text(&completion.stderr));
    assert!(completion.stdout.is_empty());

    let human = fixture.run(
        [
            OsString::from("list"),
            OsString::from("--format"),
            OsString::from("human"),
        ]
        .into_iter()
        .chain(fixture.source_args()),
    );
    assert!(human.status.success(), "{}", text(&human.stderr));
    let human = text(&human.stdout);
    assert!(human.contains("pro\\u{200b}d"));
    assert!(human.contains("us\\u{200d}-east-1"));

    let json = fixture.run(
        [
            OsString::from("list"),
            OsString::from("--format"),
            OsString::from("json"),
        ]
        .into_iter()
        .chain(fixture.source_args()),
    );
    assert!(json.status.success(), "{}", text(&json.stderr));
    assert!(text(&json.stdout).contains("pro\\u200Bd"));
    let report: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("decode escaped list JSON");
    assert_eq!(report["profiles"][0]["name"], invisible_name);
    assert_eq!(report["profiles"][0]["region"], invisible_region);
}

#[test]
fn list_and_completion_source_exclude_profiles_that_cannot_activate() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile healthy]\nregion=us-east-1\n[profile broken]\nrole_arn=arn:aws:iam::111122223333:role/Broken\n",
    )
    .expect("write invalid provider fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let mut list_arguments = vec![
        OsString::from("list"),
        OsString::from("--format"),
        OsString::from("names"),
    ];
    list_arguments.extend(fixture.source_args());
    let listed = fixture.run(list_arguments);
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    assert_eq!(text(&listed.stdout), "healthy\n");

    let mut activate_arguments = vec![OsString::from("activate"), OsString::from("broken")];
    activate_arguments.extend(fixture.source_args());
    let activation = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .args(activate_arguments)
        .output()
        .expect("activate invalid provider");
    assert_eq!(activation.status.code(), Some(1));
    assert!(text(&activation.stderr).contains("PROFILE_CONFIG_INVALID"));
}

#[test]
fn interactive_activation_with_only_invalid_profiles_reports_no_profiles_before_tty() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile broken]\nrole_arn=arn:aws:iam::111122223333:role/Broken\n",
    )
    .expect("write invalid-only catalog fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let output = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .arg("activate")
        .args(fixture.source_args())
        .output()
        .expect("activate an invalid-only catalog");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("NO_PROFILES"));
    assert!(!text(&output.stderr).contains("TTY_REQUIRED"));
}

#[test]
fn exact_activation_in_an_empty_catalog_reports_profile_not_found() {
    let fixture = Fixture::new();
    fs::write(&fixture.config, "").expect("clear config fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let output = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .args(["activate", "missing"])
        .args(fixture.source_args())
        .output()
        .expect("activate a missing exact profile");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("PROFILE_NOT_FOUND"));
    assert!(!text(&output.stderr).contains("NO_PROFILES"));
}

#[test]
fn direct_activation_fails_before_reading_configuration() {
    let output = clean_command()
        .args([
            "activate",
            "prod",
            "--config-file",
            "/definitely/missing/awswit/config",
        ])
        .output()
        .expect("run activation");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("HOOK_REQUIRED"));
    assert!(!text(&output.stderr).contains("CONFIG_READ"));
}

#[test]
fn direct_unset_requires_the_hook_and_never_exposes_a_protocol_frame() {
    let output = clean_command().arg("unset").output().expect("run unset");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("HOOK_REQUIRED"));

    let hooked = clean_command()
        .env("AWSWIT_HOOK", "1")
        .arg("unset")
        .output()
        .expect("run hooked unset");
    assert!(hooked.status.success(), "{}", text(&hooked.stderr));
    assert_eq!(
        text(&hooked.stdout),
        "AWSWIT-PATCH 1 UNSET\nUNSET AWS_PROFILE\nUNSET AWS_DEFAULT_PROFILE\nUNSET AWSWIT_PROFILE\nUNSET AWS_REGION\nUNSET AWS_DEFAULT_REGION\nAWSWIT-COMMIT\n"
    );
}

#[test]
fn exact_activation_never_substitutes_a_typo() {
    let fixture = Fixture::new();
    let mut arguments = vec![OsString::from("activate"), OsString::from("deev")];
    arguments.extend(fixture.source_args());
    let output = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .args(arguments)
        .output()
        .expect("run activation");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("PROFILE_NOT_FOUND"));
    assert!(!text(&output.stderr).contains("activated dev"));
}

#[test]
fn exact_activation_supports_a_complete_static_self_source_role() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile base]\nrole_arn=arn:aws:iam::111122223333:role/Self\nsource_profile=base\n\
         [profile outer]\nrole_arn=arn:aws:iam::111122223333:role/Outer\nsource_profile=base\n",
    )
    .expect("write self-source role fixture");
    fs::write(
        &fixture.credentials,
        "[base]\naws_access_key_id=value\naws_secret_access_key=value\n",
    )
    .expect("write self-source credentials fixture");

    let output = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .args(["activate", "base"])
        .args(fixture.source_args())
        .output()
        .expect("activate a static self-source role");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("SET AWS_PROFILE=base\n"));

    let nested = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .args(["activate", "outer"])
        .args(fixture.source_args())
        .output()
        .expect("reject a nested self-source role");
    assert_eq!(nested.status.code(), Some(1));
    assert!(nested.stdout.is_empty());
    assert!(text(&nested.stderr).contains("PROFILE_CONFIG_INVALID"));
}

#[test]
fn exact_activation_rejects_a_role_whose_source_cannot_sign() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile config-only]\nregion=us-east-1\n\
         [profile outer]\nrole_arn=arn:aws:iam::111122223333:role/Outer\nsource_profile=config-only\n",
    )
    .expect("write non-signing source fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let local = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .args(["activate", "config-only"])
        .args(fixture.source_args())
        .output()
        .expect("activate a top-level config-only profile");
    assert!(local.status.success(), "{}", text(&local.stderr));

    let chained = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .args(["activate", "outer"])
        .args(fixture.source_args())
        .output()
        .expect("reject a non-signing source profile");
    assert_eq!(chained.status.code(), Some(1));
    assert!(chained.stdout.is_empty());
    assert!(text(&chained.stderr).contains("PROFILE_CONFIG_INVALID"));
}

#[test]
fn environment_credential_source_role_is_diagnosed_and_excluded() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile a-safe]\nregion=us-east-1\n\
         [profile environment-role]\nrole_arn=arn:aws:iam::111122223333:role/Environment\ncredential_source=Environment\n\
         [profile ecs-role]\nrole_arn=arn:aws:iam::111122223333:role/Ecs\ncredential_source=EcsContainer\n\
         [profile imds-role]\nrole_arn=arn:aws:iam::111122223333:role/Imds\ncredential_source=Ec2InstanceMetadata\n\
         [profile standalone-environment]\ncredential_source=Environment\n\
         [profile standalone-ecs]\ncredential_source=EcsContainer\n\
         [profile standalone-imds]\ncredential_source=Ec2InstanceMetadata\n\
         [profile standalone-unknown]\ncredential_source=FutureProvider\n\
         [profile from-standalone]\nrole_arn=arn:aws:iam::111122223333:role/Outer\nsource_profile=standalone-ecs\n",
    )
    .expect("write environment credential-source fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let listed = fixture
        .command()
        .args(["list", "--format", "names"])
        .args(fixture.source_args())
        .output()
        .expect("list profiles");
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    assert_eq!(text(&listed.stdout), "a-safe\necs-role\nimds-role\n");

    let doctor = fixture
        .command()
        .args(["doctor", "--format", "json"])
        .args(fixture.source_args())
        .output()
        .expect("diagnose environment credential source");
    assert!(doctor.status.success(), "{}", text(&doctor.stderr));
    let report: serde_json::Value =
        serde_json::from_slice(&doctor.stdout).expect("parse doctor report");
    assert!(report["issues"].as_array().is_some_and(|issues| {
        issues.iter().any(|issue| {
            issue["kind"]["code"] == "invalid_provider_graph"
                && issue["kind"]["profile"] == "environment-role"
                && issue["kind"]["violation"]
                    == "environment_credential_source_cannot_be_selected_safely"
        })
    }));
    for profile in [
        "standalone-ecs",
        "standalone-environment",
        "standalone-imds",
        "standalone-unknown",
    ] {
        assert!(report["issues"].as_array().is_some_and(|issues| {
            issues.iter().any(|issue| {
                issue["kind"]["code"] == "invalid_provider_graph"
                    && issue["kind"]["profile"] == profile
                    && issue["kind"]["violation"] == "credential_source_without_role_arn"
            })
        }));
    }

    for profile in [
        "environment-role",
        "standalone-environment",
        "standalone-ecs",
        "standalone-imds",
        "standalone-unknown",
        "from-standalone",
    ] {
        let activated = fixture
            .command()
            .env("AWSWIT_HOOK", "1")
            .args(["activate", profile])
            .args(fixture.source_args())
            .output()
            .expect("reject unsafe credential source shape");
        assert_eq!(activated.status.code(), Some(1), "{profile}");
        assert!(activated.stdout.is_empty(), "{profile}");
        assert!(
            text(&activated.stderr).contains("PROFILE_CONFIG_INVALID"),
            "{profile}"
        );
    }
}

#[test]
fn exec_named_profile_option_supports_a_leading_hyphen() {
    let fixture = Fixture::new();
    fs::write(&fixture.config, "[profile -h]\nregion=us-east-1\n")
        .expect("write leading-hyphen profile");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");
    let current_test = std::env::current_exe().expect("current test executable");
    let mut arguments = vec![OsString::from("exec"), OsString::from("--profile=-h")];
    arguments.extend(fixture.source_args());
    arguments.extend([
        OsString::from("--"),
        current_test.into_os_string(),
        OsString::from("--exact"),
        OsString::from("hyphen_profile_child_probe"),
        OsString::from("--nocapture"),
    ]);
    let output = fixture
        .command()
        .env("AWSWIT_TEST_HYPHEN_CHILD_PROBE", "1")
        .args(arguments)
        .output()
        .expect("run leading-hyphen profile");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("hyphen profile verified"));
}

#[test]
fn hyphen_profile_child_probe() {
    if std::env::var_os("AWSWIT_TEST_HYPHEN_CHILD_PROBE").is_none() {
        return;
    }
    assert_eq!(std::env::var("AWS_PROFILE").as_deref(), Ok("-h"));
    println!("hyphen profile verified");
}

#[test]
fn credential_overrides_fail_closed_without_reading_their_values() {
    let fixture = Fixture::new();
    let mut arguments = vec![OsString::from("activate"), OsString::from("dev")];
    arguments.extend(fixture.source_args());
    let output = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env("AWS_ACCESS_KEY_ID", "OVERRIDE-SECRET")
        .env("AWS_SECRET_ACCESS_KEY", "OVERRIDE-SECRET")
        .args(&arguments)
        .output()
        .expect("run guarded activation");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = text(&output.stderr);
    assert!(error.contains("CREDENTIAL_OVERRIDE"));
    assert!(error.contains("AWS_ACCESS_KEY_ID"));
    assert!(!error.contains("OVERRIDE-SECRET"));

    arguments.insert(2, OsString::from("--clear-credential-overrides"));
    let cleared = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env("AWS_ACCESS_KEY_ID", "OVERRIDE-SECRET")
        .env("AWS_SECRET_ACCESS_KEY", "OVERRIDE-SECRET")
        .args(arguments)
        .output()
        .expect("run explicit clear");
    assert!(cleared.status.success(), "{}", text(&cleared.stderr));
    let frame = text(&cleared.stdout);
    assert!(frame.contains("UNSET AWS_ACCESS_KEY_ID\n"));
    assert!(frame.contains("UNSET AWS_SECRET_ACCESS_KEY\n"));
    assert!(frame.ends_with("AWSWIT-COMMIT\n"));
}

#[test]
fn ecs_container_source_preserves_its_intended_provider_environment() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile ecs-role]\nrole_arn = arn:aws:iam::111122223333:role/EcsRole\ncredential_source = EcsContainer\n",
    )
    .expect("write ECS provider fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let current_test = std::env::current_exe().expect("current test executable");
    let mut arguments = vec![
        OsString::from("exec"),
        OsString::from("ecs-role"),
        OsString::from("--clear-credential-overrides"),
    ];
    arguments.extend(fixture.source_args());
    arguments.extend([
        OsString::from("--"),
        current_test.into_os_string(),
        OsString::from("--exact"),
        OsString::from("ecs_child_probe"),
        OsString::from("--nocapture"),
    ]);
    let output = fixture
        .command()
        .env("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/v2/credentials")
        .env("AWS_ACCESS_KEY_ID", "CONFLICT")
        .env("AWS_SECRET_ACCESS_KEY", "CONFLICT")
        .env("AWSWIT_TEST_ECS_CHILD_PROBE", "1")
        .args(arguments)
        .output()
        .expect("run ECS child through awswit");
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("ECS environment verified"));
}

#[test]
fn ecs_child_probe() {
    if std::env::var_os("AWSWIT_TEST_ECS_CHILD_PROBE").is_none() {
        return;
    }
    assert_eq!(
        std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").as_deref(),
        Ok("/v2/credentials")
    );
    assert!(std::env::var_os("AWS_ACCESS_KEY_ID").is_none());
    assert!(std::env::var_os("AWS_SECRET_ACCESS_KEY").is_none());
    println!("ECS environment verified");
}

#[test]
fn ambient_endpoint_token_and_legacy_profile_path_fail_closed_and_can_be_cleared() {
    let fixture = Fixture::new();
    let mut arguments = vec![OsString::from("activate"), OsString::from("default")];
    arguments.extend(fixture.source_args());

    let rejected = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env(
            "AWS_EC2_METADATA_SERVICE_ENDPOINT",
            "http://IMDS-ENDPOINT-SENTINEL.invalid",
        )
        .env("AWS_BEARER_TOKEN_BEDROCK", "BEDROCK-TOKEN-SENTINEL")
        .env(
            "AWS_CREDENTIAL_PROFILES_FILE",
            "/alternate/JAVA-PROFILE-SENTINEL",
        )
        .args(&arguments)
        .output()
        .expect("reject ambient credential endpoints");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(rejected.stdout.is_empty());
    let diagnostic = text(&rejected.stderr);
    assert!(diagnostic.contains("AWS_EC2_METADATA_SERVICE_ENDPOINT"));
    assert!(diagnostic.contains("AWS_BEARER_TOKEN_BEDROCK"));
    assert!(diagnostic.contains("AWS_CREDENTIAL_PROFILES_FILE"));
    assert!(!diagnostic.contains("IMDS-ENDPOINT-SENTINEL"));
    assert!(!diagnostic.contains("BEDROCK-TOKEN-SENTINEL"));
    assert!(!diagnostic.contains("JAVA-PROFILE-SENTINEL"));

    arguments.insert(2, OsString::from("--clear-credential-overrides"));
    let cleared = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env(
            "AWS_EC2_METADATA_SERVICE_ENDPOINT",
            "http://IMDS-ENDPOINT-SENTINEL.invalid",
        )
        .env("AWS_BEARER_TOKEN_BEDROCK", "BEDROCK-TOKEN-SENTINEL")
        .env(
            "AWS_CREDENTIAL_PROFILES_FILE",
            "/alternate/JAVA-PROFILE-SENTINEL",
        )
        .args(arguments)
        .output()
        .expect("clear ambient credential endpoints");
    assert!(cleared.status.success(), "{}", text(&cleared.stderr));
    let frame = text(&cleared.stdout);
    assert!(frame.contains("UNSET AWS_EC2_METADATA_SERVICE_ENDPOINT\n"));
    assert!(frame.contains("UNSET AWS_BEARER_TOKEN_BEDROCK\n"));
    assert!(frame.contains("UNSET AWS_CREDENTIAL_PROFILES_FILE\n"));
}

#[test]
fn explicit_ec2_credential_source_proves_its_metadata_endpoint_intent() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile ec2]\nrole_arn=arn:aws:iam::111122223333:role/Ec2\ncredential_source=Ec2InstanceMetadata\n\
         [profile outer]\nrole_arn=arn:aws:iam::111122223333:role/Outer\nsource_profile=ec2\n",
    )
    .expect("write explicit IMDS provider fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    for profile in ["ec2", "outer"] {
        let output = fixture
            .command()
            .env("AWSWIT_HOOK", "1")
            .env("AWS_EC2_METADATA_SERVICE_ENDPOINT", "http://127.0.0.1:9911")
            .args(["activate", profile])
            .args(fixture.source_args())
            .output()
            .expect("activate explicit IMDS provider");
        assert!(
            output.status.success(),
            "{profile}: {}",
            text(&output.stderr)
        );
        assert!(!text(&output.stdout).contains("AWS_EC2_METADATA_SERVICE_ENDPOINT"));
    }
}

#[test]
fn login_cache_override_requires_an_explicit_login_provider_chain() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile login]\nlogin_session=company\n\
         [profile assumed]\nrole_arn=arn:aws:iam::111122223333:role/Assumed\nsource_profile=login\n\
         [profile ordinary]\nregion=us-east-1\n",
    )
    .expect("write login provider fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    for profile in ["login", "assumed"] {
        let output = fixture
            .command()
            .env("AWSWIT_HOOK", "1")
            .env("AWS_LOGIN_CACHE_DIRECTORY", "/alternate/login/cache")
            .args(["activate", profile])
            .args(fixture.source_args())
            .output()
            .expect("activate explicit login provider");
        assert!(
            output.status.success(),
            "{profile}: {}",
            text(&output.stderr)
        );
        assert!(!text(&output.stdout).contains("AWS_LOGIN_CACHE_DIRECTORY"));
    }

    let rejected = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env("AWS_LOGIN_CACHE_DIRECTORY", "/alternate/login/cache")
        .args(["activate", "ordinary"])
        .args(fixture.source_args())
        .output()
        .expect("reject ambient login cache for an ordinary profile");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(text(&rejected.stderr).contains("AWS_LOGIN_CACHE_DIRECTORY"));
    assert!(!text(&rejected.stderr).contains("/alternate/login/cache"));

    let cleared = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env("AWS_LOGIN_CACHE_DIRECTORY", "/alternate/login/cache")
        .args(["activate", "ordinary", "--clear-credential-overrides"])
        .args(fixture.source_args())
        .output()
        .expect("clear ambient login cache");
    assert!(cleared.status.success(), "{}", text(&cleared.stderr));
    assert!(text(&cleared.stdout).contains("UNSET AWS_LOGIN_CACHE_DIRECTORY\n"));
}

#[test]
fn direct_credentials_never_bypass_the_selected_profile() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile process]\ncredential_process=helper\n\
         [profile assumed]\nrole_arn=arn:aws:iam::111122223333:role/Assumed\nsource_profile=process\n\
         [profile ordinary]\nregion=us-east-1\n",
    )
    .expect("write environment provider fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    for profile in ["assumed", "ordinary"] {
        for (access, secret) in [
            ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"),
            ("AWS_ACCESS_KEY", "AWS_SECRET_KEY"),
            ("AMAZON_ACCESS_KEY_ID", "AMAZON_SECRET_ACCESS_KEY"),
        ] {
            let output = fixture
                .command()
                .env("AWSWIT_HOOK", "1")
                .env(access, "DIRECT-ACCESS-SENTINEL")
                .env(secret, "DIRECT-SECRET-SENTINEL")
                .args(["activate", profile])
                .args(fixture.source_args())
                .output()
                .expect("reject direct credentials before profile activation");
            assert_eq!(
                output.status.code(),
                Some(1),
                "{profile}: {access}/{secret}"
            );
            let diagnostic = text(&output.stderr);
            assert!(diagnostic.contains(access));
            assert!(diagnostic.contains(secret));
            assert!(!diagnostic.contains("DIRECT-ACCESS-SENTINEL"));
            assert!(!diagnostic.contains("DIRECT-SECRET-SENTINEL"));
        }
    }

    let cleared = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env("AWS_ACCESS_KEY", "LEGACY-ACCESS-SENTINEL")
        .env("AWS_SECRET_KEY", "LEGACY-SECRET-SENTINEL")
        .env("AMAZON_ACCESS_KEY_ID", "AMAZON-ACCESS-SENTINEL")
        .env("AMAZON_SECRET_ACCESS_KEY", "AMAZON-SECRET-SENTINEL")
        .env("AMAZON_SESSION_TOKEN", "AMAZON-TOKEN-SENTINEL")
        .args(["activate", "ordinary", "--clear-credential-overrides"])
        .args(fixture.source_args())
        .output()
        .expect("clear legacy aliases for an ordinary profile");
    assert!(cleared.status.success(), "{}", text(&cleared.stderr));
    let frame = text(&cleared.stdout);
    assert!(frame.contains("UNSET AWS_ACCESS_KEY\n"));
    assert!(frame.contains("UNSET AWS_SECRET_KEY\n"));
    assert!(frame.contains("UNSET AMAZON_ACCESS_KEY_ID\n"));
    assert!(frame.contains("UNSET AMAZON_SECRET_ACCESS_KEY\n"));
    assert!(frame.contains("UNSET AMAZON_SESSION_TOKEN\n"));

    let partial = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env("AWS_ACCESS_KEY", "PARTIAL-ACCESS-SENTINEL")
        .args(["activate", "ordinary", "--clear-credential-overrides"])
        .args(fixture.source_args())
        .output()
        .expect("clear a partial direct tuple");
    assert!(partial.status.success(), "{}", text(&partial.stderr));
    assert!(text(&partial.stdout).contains("UNSET AWS_ACCESS_KEY\n"));
}

#[test]
fn human_list_identifies_login_provider_without_retaining_its_identity_value() {
    let fixture = Fixture::new();
    let sentinel = "arn:aws:iam::111122223333:user/LOGIN-IDENTITY-SENTINEL";
    fs::write(
        &fixture.config,
        format!("[profile console]\nlogin_session={sentinel}\nregion=us-west-2\n"),
    )
    .expect("write login provider fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let output = fixture.run(
        [
            OsString::from("list"),
            OsString::from("--format"),
            OsString::from("human"),
        ]
        .into_iter()
        .chain(fixture.source_args()),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let human = text(&output.stdout);
    assert!(human.contains("console\tAWS Login\tus-west-2"));
    assert!(!human.contains(sentinel));
}

#[test]
fn doctor_is_offline_and_reports_only_override_names() {
    let fixture = Fixture::new();
    let mut arguments = vec![
        OsString::from("doctor"),
        OsString::from("--format"),
        OsString::from("json"),
    ];
    arguments.extend(fixture.source_args());
    let output = fixture
        .command()
        .env("AWS_ACCESS_KEY_ID", "DOCTOR-SECRET")
        .env("AWS_ACCESS_KEY", "DOCTOR-LEGACY-ACCESS")
        .env("AWS_SECRET_KEY", "DOCTOR-LEGACY-SECRET")
        .env("AMAZON_ACCESS_KEY_ID", "DOCTOR-AMAZON-ACCESS")
        .env("AMAZON_SECRET_ACCESS_KEY", "DOCTOR-AMAZON-SECRET")
        .env("AMAZON_SESSION_TOKEN", "DOCTOR-AMAZON-TOKEN")
        .env("AWS_LOGIN_CACHE_DIRECTORY", "/DOCTOR-LOGIN-CACHE")
        .env("AWS_CREDENTIAL_PROFILES_FILE", "/DOCTOR-JAVA-PROFILE")
        .args(arguments)
        .output()
        .expect("run doctor");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let report = text(&output.stdout);
    assert!(report.contains("\"offline\": true"));
    for name in [
        "AWS_ACCESS_KEY_ID",
        "AWS_ACCESS_KEY",
        "AWS_SECRET_KEY",
        "AMAZON_ACCESS_KEY_ID",
        "AMAZON_SECRET_ACCESS_KEY",
        "AMAZON_SESSION_TOKEN",
        "AWS_LOGIN_CACHE_DIRECTORY",
        "AWS_CREDENTIAL_PROFILES_FILE",
    ] {
        assert!(report.contains(name), "doctor omitted {name}");
    }
    for secret in [
        "DOCTOR-SECRET",
        "DOCTOR-LEGACY-ACCESS",
        "DOCTOR-LEGACY-SECRET",
        "DOCTOR-AMAZON-ACCESS",
        "DOCTOR-AMAZON-SECRET",
        "DOCTOR-AMAZON-TOKEN",
        "DOCTOR-LOGIN-CACHE",
        "DOCTOR-JAVA-PROFILE",
    ] {
        assert!(!report.contains(secret), "doctor leaked {secret}");
    }
}

#[test]
fn human_doctor_lists_actionable_issue_details_without_values() {
    let fixture = Fixture::new();
    let sentinel = "DOCTOR-COMMAND-SECRET";
    fs::write(
        &fixture.config,
        format!(
            "[profile broken]\ncredential_process={sentinel}-one\ncredential_process={sentinel}-two\n"
        ),
    )
    .expect("write duplicate provider fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");

    let output = fixture.run(
        [
            OsString::from("doctor"),
            OsString::from("--format"),
            OsString::from("human"),
        ]
        .into_iter()
        .chain(fixture.source_args()),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let report = text(&output.stdout);
    assert!(report.contains("catalog issues: 1"));
    assert!(report.contains("\"code\":\"duplicate_metadata\""));
    assert!(report.contains("\"field\":\"credential_process\""));
    assert!(!report.contains(sentinel));
}

#[test]
fn doctor_outputs_escape_terminal_controls_without_changing_json_values() {
    let fixture = Fixture::new();
    let terminal_controls = "\u{007f}\u{0085}\u{009b}\u{061c}\u{200b}\u{200e}\u{200f}\u{2028}\u{2029}\u{202a}\u{202b}\u{202c}\u{202d}\u{202e}\u{2066}\u{2067}\u{2068}\u{2069}";
    let config = fixture
        .root
        .path()
        .join(format!("config-{terminal_controls}"));
    fs::copy(&fixture.config, &config).expect("copy config to terminal-control path");
    let current_profile = format!("profile-{terminal_controls}");

    let output = fixture
        .command()
        .env("AWS_PROFILE", &current_profile)
        .args([
            OsString::from("doctor"),
            OsString::from("--format"),
            OsString::from("json"),
            OsString::from("--config-file"),
            config.as_os_str().to_owned(),
            OsString::from("--credentials-file"),
            fixture.credentials.as_os_str().to_owned(),
        ])
        .output()
        .expect("run doctor with terminal controls");
    assert!(output.status.success(), "{}", text(&output.stderr));

    let raw_report = text(&output.stdout);
    for (control, escape) in [
        ('\u{007f}', "\\u007F"),
        ('\u{0085}', "\\u0085"),
        ('\u{009b}', "\\u009B"),
        ('\u{061c}', "\\u061C"),
        ('\u{200b}', "\\u200B"),
        ('\u{200e}', "\\u200E"),
        ('\u{200f}', "\\u200F"),
        ('\u{2028}', "\\u2028"),
        ('\u{2029}', "\\u2029"),
        ('\u{202a}', "\\u202A"),
        ('\u{202b}', "\\u202B"),
        ('\u{202c}', "\\u202C"),
        ('\u{202d}', "\\u202D"),
        ('\u{202e}', "\\u202E"),
        ('\u{2066}', "\\u2066"),
        ('\u{2067}', "\\u2067"),
        ('\u{2068}', "\\u2068"),
        ('\u{2069}', "\\u2069"),
    ] {
        assert!(
            !raw_report.contains(control),
            "raw control U+{:04X}",
            control as u32
        );
        assert!(raw_report.contains(escape), "missing escape {escape}");
    }

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse doctor JSON");
    assert_eq!(report["current_profile"], current_profile);
    assert_eq!(
        report["sources"]["config"]["path"],
        config.to_string_lossy().as_ref()
    );

    let human_output = fixture
        .command()
        .env("AWS_PROFILE", &current_profile)
        .args([
            OsString::from("doctor"),
            OsString::from("--format"),
            OsString::from("human"),
            OsString::from("--config-file"),
            config.as_os_str().to_owned(),
            OsString::from("--credentials-file"),
            fixture.credentials.as_os_str().to_owned(),
        ])
        .output()
        .expect("run human doctor with terminal controls");
    assert!(
        human_output.status.success(),
        "{}",
        text(&human_output.stderr)
    );
    let human_report = text(&human_output.stdout);
    for (control, escape) in [('\u{2028}', "\\u{2028}"), ('\u{2029}', "\\u{2029}")] {
        assert!(!human_report.contains(control));
        assert!(human_report.contains(escape), "missing escape {escape}");
    }
}

#[test]
fn child_probe() {
    if std::env::var_os("AWSWIT_TEST_CHILD_PROBE").is_none() {
        return;
    }
    assert_eq!(std::env::var("AWS_PROFILE").as_deref(), Ok("dev"));
    assert_eq!(std::env::var("AWS_DEFAULT_PROFILE").as_deref(), Ok("dev"));
    assert_eq!(std::env::var("AWS_REGION").as_deref(), Ok("ap-northeast-1"));
    assert!(std::env::var_os("AWS_ACCESS_KEY_ID").is_none());
    println!("child environment verified");
}

#[test]
fn exec_changes_only_the_child_and_preserves_argv() {
    let fixture = Fixture::new();
    let current_test = std::env::current_exe().expect("current test executable");
    let mut arguments = vec![
        OsString::from("exec"),
        OsString::from("dev"),
        OsString::from("--clear-credential-overrides"),
    ];
    arguments.extend(fixture.source_args());
    arguments.extend([
        OsString::from("--"),
        current_test.into_os_string(),
        OsString::from("--exact"),
        OsString::from("child_probe"),
        OsString::from("--nocapture"),
    ]);

    let output = fixture
        .command()
        .env("AWS_ACCESS_KEY_ID", "OVERRIDE")
        .env("AWS_SECRET_ACCESS_KEY", "OVERRIDE")
        .env("AWSWIT_TEST_CHILD_PROBE", "1")
        .args(arguments)
        .output()
        .expect("run child through awswit");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        text(&output.stdout),
        text(&output.stderr)
    );
    assert!(text(&output.stdout).contains("child environment verified"));
    assert!(std::env::var_os("AWS_PROFILE").is_none());
}

#[cfg(unix)]
#[test]
fn exec_does_not_reinterpret_shell_metacharacters() {
    let fixture = Fixture::new();
    let mut arguments = vec![OsString::from("exec"), OsString::from("default")];
    arguments.extend(fixture.source_args());
    arguments.extend([
        OsString::from("--"),
        OsString::from("printf"),
        OsString::from("%s"),
        OsString::from("$HOME; $(touch /tmp/awswit-must-not-run)"),
    ]);
    let output = fixture.run(arguments);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "$HOME; $(touch /tmp/awswit-must-not-run)"
    );
}

#[cfg(unix)]
#[test]
fn exec_preserves_the_caller_supplied_argv_zero() {
    let fixture = Fixture::new();
    let mut arguments = vec![OsString::from("exec"), OsString::from("default")];
    arguments.extend(fixture.source_args());
    arguments.extend([
        OsString::from("--"),
        OsString::from("sh"),
        OsString::from("-c"),
        OsString::from("printf '%s' \"$0\""),
    ]);

    let output = fixture.run(arguments);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "sh");
}

#[cfg(unix)]
#[test]
fn exec_never_falls_back_to_a_shell_for_an_unknown_file_format() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let marker = fixture.root.path().join("enoexec-shell-marker");
    let executable = fixture.root.path().join("executable-text");
    fs::write(&executable, format!("touch {}\n", marker.display()))
        .expect("write executable text fixture");
    let mut permissions = fs::metadata(&executable)
        .expect("stat executable text fixture")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).expect("make text fixture executable");

    let mut arguments = vec![OsString::from("exec"), OsString::from("default")];
    arguments.extend(fixture.source_args());
    arguments.extend([OsString::from("--"), executable.into_os_string()]);
    let output = fixture.run(arguments);

    assert_eq!(output.status.code(), Some(126), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("COMMAND_NOT_EXECUTABLE"));
    assert!(
        !marker.exists(),
        "unknown file format was interpreted as shell code"
    );
}

#[cfg(unix)]
#[test]
fn exec_does_not_search_the_working_directory_when_path_is_unset() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let probe = fixture.root.path().join("untrusted-probe");
    fs::copy("/bin/true", &probe).expect("copy probe executable");
    let mut permissions = fs::metadata(&probe).expect("stat probe").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&probe, permissions).expect("make probe executable");

    let mut arguments = vec![OsString::from("exec"), OsString::from("default")];
    arguments.extend(fixture.source_args());
    arguments.extend([OsString::from("--"), OsString::from("untrusted-probe")]);
    let output = fixture
        .command()
        .env_remove("PATH")
        .current_dir(fixture.root.path())
        .args(arguments)
        .output()
        .expect("run without PATH");

    assert_eq!(output.status.code(), Some(127));
    assert!(text(&output.stderr).contains("COMMAND_NOT_FOUND"));
}

#[cfg(unix)]
#[test]
fn exec_path_search_continues_after_an_inaccessible_candidate() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let denied_directory = fixture.root.path().join("denied-bin");
    let usable_directory = fixture.root.path().join("usable-bin");
    fs::create_dir_all(&denied_directory).expect("create denied bin directory");
    fs::create_dir_all(&usable_directory).expect("create usable bin directory");

    let denied = denied_directory.join("probe");
    fs::copy("/bin/true", &denied).expect("copy inaccessible candidate");
    let mut denied_permissions = fs::metadata(&denied).unwrap().permissions();
    // An execute bit exists, but it is not in the owner class selected for this
    // process, so a mode-bit precheck alone cannot determine effective access.
    denied_permissions.set_mode(0o001);
    fs::set_permissions(&denied, denied_permissions).unwrap();

    let usable = usable_directory.join("probe");
    fs::copy("/bin/true", &usable).expect("copy usable candidate");
    let mut usable_permissions = fs::metadata(&usable).unwrap().permissions();
    usable_permissions.set_mode(0o700);
    fs::set_permissions(&usable, usable_permissions).unwrap();

    let search_path = std::env::join_paths([&denied_directory, &usable_directory]).unwrap();
    let mut arguments = vec![OsString::from("exec"), OsString::from("default")];
    arguments.extend(fixture.source_args());
    arguments.extend([OsString::from("--"), OsString::from("probe")]);
    let output = fixture
        .command()
        .env("PATH", search_path)
        .args(arguments)
        .output()
        .expect("run with multiple PATH candidates");

    assert!(output.status.success(), "{}", text(&output.stderr));
}

#[test]
fn explicit_missing_source_is_a_hard_error_with_empty_stdout() {
    let fixture = Fixture::new();
    let output = fixture.run([
        OsString::from("list"),
        OsString::from("--config-file"),
        fixture.root.path().join("missing").into_os_string(),
        OsString::from("--credentials-file"),
        fixture.credentials.as_os_str().to_owned(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("CONFIG_READ"));
}

#[cfg(unix)]
#[test]
fn regular_configuration_symlinks_are_valid_catalog_sources() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let config_link = fixture.root.path().join("config.link");
    let credentials_link = fixture.root.path().join("credentials.link");
    symlink(&fixture.config, &config_link).expect("link config source");
    symlink(&fixture.credentials, &credentials_link).expect("link credentials source");

    let output = fixture.run([
        OsString::from("list"),
        OsString::from("--format"),
        OsString::from("names"),
        OsString::from("--config-file"),
        config_link.into_os_string(),
        OsString::from("--credentials-file"),
        credentials_link.into_os_string(),
    ]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "default\ndev\nmodern-sso\n");
}

#[test]
fn activation_pins_relative_source_paths_to_the_catalog_snapshot() {
    let fixture = Fixture::new();
    let working = fixture.root.path().join("working");
    let sources = working.join("aws");
    fs::create_dir_all(&sources).expect("create relative source directory");
    fs::write(
        sources.join("config"),
        "[profile relative]\nregion = ap-northeast-1\n",
    )
    .expect("write relative config");
    fs::write(sources.join("credentials"), "").expect("write relative credentials");

    for source_kind in ["command-line", "environment"] {
        let mut command = fixture.command();
        command
            .current_dir(&working)
            .env("AWSWIT_HOOK", "1")
            .args(["activate", "relative"]);
        if source_kind == "command-line" {
            command.args([
                "--config-file",
                "aws/config",
                "--credentials-file",
                "aws/credentials",
            ]);
        } else {
            command
                .env("AWS_CONFIG_FILE", "aws/config")
                .env("AWS_SHARED_CREDENTIALS_FILE", "aws/credentials");
        }

        let output = command.output().expect("activate with relative sources");
        assert!(output.status.success(), "{}", text(&output.stderr));
        let frame = text(&output.stdout);
        assert!(
            frame.contains(&format!(
                "SET AWS_CONFIG_FILE={}\n",
                sources.join("config").display()
            )),
            "{source_kind} frame did not pin the config path: {frame:?}"
        );
        assert!(
            frame.contains(&format!(
                "SET AWS_SHARED_CREDENTIALS_FILE={}\n",
                sources.join("credentials").display()
            )),
            "{source_kind} frame did not pin the credentials path: {frame:?}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn special_configuration_and_history_files_never_block() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let config_fifo = fixture.root.path().join("config.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&config_fifo)
            .status()
            .expect("create config FIFO")
            .success()
    );

    let config_output = Command::new("timeout")
        .args(["2s", env!("CARGO_BIN_EXE_awswit"), "list", "--config-file"])
        .arg(&config_fifo)
        .arg("--credentials-file")
        .arg(&fixture.credentials)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("run with config FIFO");
    assert_eq!(config_output.status.code(), Some(1));
    assert!(text(&config_output.stderr).contains("CONFIG_READ"));

    let config_fifo_link = fixture.root.path().join("config-fifo.link");
    symlink(&config_fifo, &config_fifo_link).expect("link config FIFO");
    let linked_config_output = Command::new("timeout")
        .args(["2s", env!("CARGO_BIN_EXE_awswit"), "list", "--config-file"])
        .arg(&config_fifo_link)
        .arg("--credentials-file")
        .arg(&fixture.credentials)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("run with symlink to config FIFO");
    assert_eq!(linked_config_output.status.code(), Some(1));
    assert!(text(&linked_config_output.stderr).contains("CONFIG_READ"));

    let config_device_output = Command::new("timeout")
        .args(["2s", env!("CARGO_BIN_EXE_awswit"), "list", "--config-file"])
        .arg("/dev/null")
        .arg("--credentials-file")
        .arg(&fixture.credentials)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("run with character device as config");
    assert_eq!(config_device_output.status.code(), Some(1));
    assert!(text(&config_device_output.stderr).contains("CONFIG_READ"));

    let state_root = fixture.root.path().join("state");
    let history_root = state_root.join("awswit");
    fs::create_dir_all(&history_root).expect("create history root");
    let history_fifo = history_root.join("history.json");
    assert!(
        Command::new("mkfifo")
            .arg(&history_fifo)
            .status()
            .expect("create history FIFO")
            .success()
    );

    let history_output = Command::new("timeout")
        .args([
            "2s",
            env!("CARGO_BIN_EXE_awswit"),
            "activate",
            "default",
            "--config-file",
        ])
        .arg(&fixture.config)
        .arg("--credentials-file")
        .arg(&fixture.credentials)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("AWSWIT_HOOK", "1")
        .env("XDG_STATE_HOME", &state_root)
        .env("HOME", fixture.root.path())
        .output()
        .expect("run with history FIFO");
    assert_eq!(history_output.status.code(), Some(0));
    assert!(text(&history_output.stdout).contains("AWSWIT-COMMIT"));
    assert!(!text(&history_output.stderr).contains("HISTORY_READ"));
    assert!(text(&history_output.stderr).contains("HISTORY_WRITE"));
}

#[cfg(unix)]
#[test]
fn exec_accepts_a_non_unicode_explicit_path() {
    use std::os::unix::ffi::OsStringExt;

    let fixture = Fixture::new();
    let relative_path = OsString::from_vec(b"config-\xff".to_vec());
    let path = fixture.root.path().join(&relative_path);
    fs::copy(&fixture.config, &path).expect("copy non-Unicode config");
    let mut arguments = vec![
        OsString::from("exec"),
        OsString::from("default"),
        OsString::from("--config-file"),
        relative_path,
        OsString::from("--credentials-file"),
        fixture.credentials.as_os_str().to_owned(),
        OsString::from("--"),
        OsString::from("true"),
    ];
    let mut command = fixture.command();
    let output = command
        .current_dir(fixture.root.path())
        .args(arguments.drain(..))
        .output()
        .expect("run with a relative non-Unicode source");
    assert!(output.status.success(), "{}", text(&output.stderr));
}

#[cfg(unix)]
#[test]
fn activation_preserves_an_absolute_non_unicode_environment_source() {
    use std::os::unix::ffi::OsStringExt;

    let fixture = Fixture::new();
    let config = fixture
        .root
        .path()
        .join(OsString::from_vec(b"config-environment-\xff".to_vec()));
    fs::copy(&fixture.config, &config).expect("copy non-Unicode environment config");

    let output = fixture
        .command()
        .env("AWSWIT_HOOK", "1")
        .env("AWS_CONFIG_FILE", &config)
        .env("AWS_SHARED_CREDENTIALS_FILE", &fixture.credentials)
        .args(["activate", "default"])
        .output()
        .expect("activate with an absolute non-Unicode environment source");

    assert!(output.status.success(), "{}", text(&output.stderr));
    let frame = text(&output.stdout);
    assert!(frame.contains("SET AWS_PROFILE=default\n"));
    assert!(!frame.contains("AWS_CONFIG_FILE"));
    assert!(!frame.contains("AWS_SHARED_CREDENTIALS_FILE"));
    assert!(frame.ends_with("AWSWIT-COMMIT\n"));
}

#[cfg(unix)]
fn write_hook(fixture: &Fixture, shell: &str) -> PathBuf {
    let hook = fixture.root.path().join(format!("awswit.{shell}"));
    let output = clean_command()
        .args(["init", shell])
        .output()
        .expect("generate hook");
    assert!(output.status.success());
    fs::write(&hook, output.stdout).expect("write hook");
    hook
}

#[cfg(unix)]
fn path_with_binary() -> OsString {
    let binary_directory = Path::new(env!("CARGO_BIN_EXE_awswit"))
        .parent()
        .expect("binary directory");
    std::env::join_paths(std::iter::once(binary_directory.to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("compose PATH")
}

#[cfg(target_os = "linux")]
fn wait_with_timeout(mut child: std::process::Child) -> Output {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().expect("collect child output"),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("collect timed-out output");
                panic!(
                    "child timed out; stdout={:?}, stderr={:?}",
                    text(&output.stdout),
                    text(&output.stderr)
                );
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("could not poll child: {error}");
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn wait_for_transcript(child: &mut std::process::Child, transcript: &Path, marker: &[u8]) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let bytes = fs::read(transcript).unwrap_or_default();
        if bytes.windows(marker.len()).any(|window| window == marker) {
            return;
        }
        if let Some(status) = child.try_wait().expect("poll PTY child") {
            panic!(
                "PTY child exited before readiness ({status}); transcript={:?}",
                text(&bytes)
            );
        }
        assert!(
            std::time::Instant::now() < deadline,
            "PTY readiness timed out; transcript={:?}",
            text(&bytes)
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn bash_hook_treats_profile_metacharacters_as_literal_data() {
    let fixture = Fixture::new();
    let marker = fixture.root.path().join("injection-marker");
    let profile = format!(
        "$(touch {}) `touch {}` = 東京\u{200b}",
        marker.display(),
        marker.display()
    );
    fs::write(
        &fixture.config,
        format!("[profile {profile}]\nregion = ap-northeast-1\n"),
    )
    .expect("write malicious-name fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");
    let hook = write_hook(&fixture, "bash");

    let script = r#"source "$HOOK"; awswit "$TARGET" --config-file "$CONFIG" --credentials-file "$CREDENTIALS"; rc=$?; printf '%s\n' "$AWS_PROFILE"; exit $rc"#;
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("TARGET", &profile)
        .env("CONFIG", &fixture.config)
        .env("CREDENTIALS", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .env("XDG_DATA_HOME", fixture.root.path().join("data"))
        .output()
        .expect("run bash hook");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout).trim(), profile);
    assert_eq!(text(&output.stderr), "awswit: AWS profile activated\n");
    assert!(!marker.exists(), "profile data was executed by the shell");
}

#[cfg(unix)]
#[test]
fn bash_hook_can_activate_a_profile_named_like_a_help_flag_after_separator() {
    let fixture = Fixture::new();
    fs::write(&fixture.config, "[profile -h]\nregion = us-east-1\n")
        .expect("write hyphenated-name fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");
    let hook = write_hook(&fixture, "bash");

    let script = r#"source "$HOOK"; awswit activate --config-file "$CONFIG" --credentials-file "$CREDENTIALS" -- -h; rc=$?; printf '%s\n' "$AWS_PROFILE"; exit $rc"#;
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("CONFIG", &fixture.config)
        .env("CREDENTIALS", &fixture.credentials)
        .env("AWS_CONFIG_FILE", &fixture.config)
        .env("AWS_SHARED_CREDENTIALS_FILE", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .env("XDG_DATA_HOME", fixture.root.path().join("data"))
        .output()
        .expect("run hyphenated profile hook");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout).trim(), "-h");
}

#[cfg(unix)]
#[test]
fn bash_completion_and_named_option_activate_a_leading_hyphen_profile() {
    let fixture = Fixture::new();
    fs::write(&fixture.config, "[profile -h]\nregion = us-east-1\n")
        .expect("write hyphenated-name fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");
    let hook = write_hook(&fixture, "bash");

    let script = r#"source "$HOOK"; COMP_WORDS=(awswit -); COMP_CWORD=1; _awswit_complete; printf '<%s>\n' "${COMPREPLY[@]}"; awswit --profile=-h --config-file "$CONFIG" --credentials-file "$CREDENTIALS"; printf 'ACTIVE|%s\n' "$AWS_PROFILE""#;
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("CONFIG", &fixture.config)
        .env("CREDENTIALS", &fixture.credentials)
        .env("AWS_CONFIG_FILE", &fixture.config)
        .env("AWS_SHARED_CREDENTIALS_FILE", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .env("XDG_DATA_HOME", fixture.root.path().join("data"))
        .output()
        .expect("run leading-hyphen completion");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("<--profile=-h>"));
    assert!(text(&output.stdout).contains("ACTIVE|-h"));
}

#[cfg(unix)]
#[test]
fn hook_markers_are_scoped_to_wrapper_calls_and_doctor_detects_the_hook() {
    let fixture = Fixture::new();
    let hook = write_hook(&fixture, "bash");
    let script = r#"source "$HOOK"; test -z "${AWSWIT_HOOK-}"; test -z "${AWSWIT_SHELL-}"; awswit doctor --format json --config-file "$CONFIG" --credentials-file "$CREDENTIALS"; test -z "${AWSWIT_HOOK-}"; test -z "${AWSWIT_SHELL-}""#;
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("CONFIG", &fixture.config)
        .env("CREDENTIALS", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .output()
        .expect("run scoped hook marker test");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("\"hook_detected\": true"));

    let direct = fixture.run(["activate", "default"]);
    assert_eq!(direct.status.code(), Some(1));
    assert!(text(&direct.stderr).contains("HOOK_REQUIRED"));
}

#[cfg(unix)]
#[test]
fn bash_completion_preserves_profile_names_as_literal_lines() {
    let fixture = Fixture::new();
    let marker = fixture.root.path().join("completion-injection-marker");
    let profile = format!("team space 'quote' $(touch {})", marker.display());
    let invisible_profile = format!("team\u{200b} space 'quote' $(touch {})", marker.display());
    fs::write(
        &fixture.config,
        format!(
            "[profile {profile}]\nregion = ap-northeast-1\n\
             [profile {invisible_profile}]\nregion = ap-northeast-1\n"
        ),
    )
    .expect("write completion fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");
    let hook = write_hook(&fixture, "bash");

    let script = r#"source "$HOOK"; COMP_WORDS=(awswit team); COMP_CWORD=1; _awswit_complete; printf '<%s>\n' "${COMPREPLY[@]}""#;
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("AWS_CONFIG_FILE", &fixture.config)
        .env("AWS_SHARED_CREDENTIALS_FILE", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .output()
        .expect("run Bash completion");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout).trim(), format!("<{profile}>"));
    assert!(!text(&output.stdout).contains(&invisible_profile));
    assert!(
        !marker.exists(),
        "completion data was executed by the shell"
    );
}

#[cfg(unix)]
#[test]
fn bash_hook_composes_static_options_with_dynamic_profiles_and_hides_root_collisions() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[profile list]\nregion=us-east-1\n[profile team]\nregion=us-east-1\n",
    )
    .expect("write completion collision fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");
    let hook = write_hook(&fixture, "bash");

    let script = r#"source "$HOOK"
COMP_WORDS=(awswit activate --r); COMP_CWORD=2; _awswit_complete; printf 'OPTION|%s\n' "${COMPREPLY[@]}"
COMP_WORDS=(awswit l); COMP_CWORD=1; _awswit_complete; printf 'ROOT|%s\n' "${COMPREPLY[@]}"
COMP_WORDS=(awswit activate l); COMP_CWORD=2; _awswit_complete; printf 'PROFILE|%s\n' "${COMPREPLY[@]}""#;
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("AWS_CONFIG_FILE", &fixture.config)
        .env("AWS_SHARED_CREDENTIALS_FILE", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .output()
        .expect("run composed Bash completion");

    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.lines().any(|line| line == "OPTION|--region"));
    assert_eq!(
        stdout.lines().filter(|line| *line == "ROOT|list").count(),
        1,
        "command/profile collision was duplicated at root: {stdout}"
    );
    assert!(stdout.lines().any(|line| line == "PROFILE|list"));
}

#[cfg(unix)]
#[test]
fn bash_rejects_a_truncated_frame_without_partial_changes() {
    let fixture = Fixture::new();
    let hook = write_hook(&fixture, "bash");
    let script = r#"source "$HOOK"; export AWS_PROFILE=before; frame=$'AWSWIT-PATCH 1 ACTIVATE\nSET AWS_PROFILE=after\nSET AWS_DEFAULT_PROFILE=after\nSET AWSWIT_PROFILE=after\nUNSET AWS_REGION\nUNSET AWS_DEFAULT_REGION'; if _awswit_apply_patch "$frame" >/dev/null 2>/dev/null; then exit 9; fi; test "$AWS_PROFILE" = before"#;
    let status = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("HOOK", hook)
        .status()
        .expect("run invalid frame test");
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn bash_rejects_special_variable_attributes_without_partial_changes() {
    let fixture = Fixture::new();
    let hook = write_hook(&fixture, "bash");
    let script = r#"source "$HOOK"; export AWS_DEFAULT_PROFILE=before; original_path=$PATH; declare -n AWS_PROFILE=PATH; frame=$'AWSWIT-PATCH 1 ACTIVATE\nSET AWS_PROFILE=after\nSET AWS_DEFAULT_PROFILE=after\nSET AWSWIT_PROFILE=after\nUNSET AWS_REGION\nUNSET AWS_DEFAULT_REGION\nAWSWIT-COMMIT'; if _awswit_apply_patch "$frame" >/dev/null 2>/dev/null; then exit 9; fi; test "$PATH" = "$original_path" && test "$AWS_DEFAULT_PROFILE" = before"#;
    let status = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .env("HOOK", hook)
        .status()
        .expect("run special variable test");
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn zsh_rejects_special_variable_attributes_without_partial_changes() {
    let fixture = Fixture::new();
    let hook = write_hook(&fixture, "zsh");
    let script = r#"source "$HOOK"; export AWS_PROFILE=before; typeset -a AWS_DEFAULT_PROFILE; frame=$'AWSWIT-PATCH 1 ACTIVATE\nSET AWS_PROFILE=after\nSET AWS_DEFAULT_PROFILE=after\nSET AWSWIT_PROFILE=after\nUNSET AWS_REGION\nUNSET AWS_DEFAULT_REGION\nAWSWIT-COMMIT'; if _awswit_apply_patch "$frame" >/dev/null 2>/dev/null; then exit 9; fi; [[ "$AWS_PROFILE" = before ]]"#;
    let status = Command::new("zsh")
        .args(["-f", "-c", script])
        .env("HOOK", hook)
        .status()
        .expect("run Zsh special variable test");
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn zsh_dynamic_completion_survives_static_completer_state_changes() {
    let fixture = Fixture::new();
    fs::write(&fixture.config, include_str!("completion_profiles.ini"))
        .expect("write Zsh completion fixture");
    fs::write(&fixture.credentials, "").expect("clear credentials fixture");
    let hook = write_hook(&fixture, "zsh");
    let script = r#"source "$HOOK"
function _awswit { (( CURRENT += 1 )); }
typeset -ga captured
function compadd { captured+=("${@:2}"); }
words=(awswit l)
CURRENT=2
_awswit_with_profiles
for candidate in "${captured[@]}"; do print -r -- "ROOT|$candidate"; done
captured=()
words=(awswit activate l)
CURRENT=3
_awswit_with_profiles
for candidate in "${captured[@]}"; do print -r -- "PROFILE|$candidate"; done"#;
    let output = Command::new("zsh")
        .args(["-f", "-c", script])
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("AWS_CONFIG_FILE", &fixture.config)
        .env("AWS_SHARED_CREDENTIALS_FILE", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .output()
        .expect("run Zsh completion state test");

    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert_eq!(
        stdout.lines().filter(|line| *line == "ROOT|list").count(),
        0
    );
    assert!(stdout.lines().any(|line| line == "ROOT|team space"));
    assert!(stdout.lines().any(|line| line == "ROOT|--profile=-h"));
    assert!(stdout.lines().any(|line| line == "PROFILE|list"));
    assert!(stdout.lines().any(|line| line == "PROFILE|team space"));
    assert!(stdout.lines().any(|line| line == "PROFILE|--profile=-h"));
}

#[cfg(unix)]
#[test]
fn shell_scripts_pass_syntax_checks() {
    let fixture = Fixture::new();
    let bash = write_hook(&fixture, "bash");
    let zsh = write_hook(&fixture, "zsh");
    assert!(
        Command::new("bash")
            .args(["-n"])
            .arg(bash)
            .status()
            .expect("bash -n")
            .success()
    );
    assert!(
        Command::new("zsh")
            .args(["-n"])
            .arg(zsh)
            .status()
            .expect("zsh -n")
            .success()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn list_broken_pipe_never_panics() {
    let fixture = Fixture::new();
    let mut config = String::new();
    // Stay below the catalog's record budget while exceeding a pipe buffer by
    // enough that the writer must observe EPIPE.
    for index in 0..10_000 {
        config.push_str(&format!("[profile profile-{index:05}]\nregion=us-east-1\n"));
    }
    fs::write(&fixture.config, config).expect("write large catalog");
    let command = format!(
        "\"{}\" list --format names --config-file \"{}\" --credentials-file \"{}\" | head -n0",
        env!("CARGO_BIN_EXE_awswit"),
        fixture.config.display(),
        fixture.credentials.display()
    );
    let output = Command::new("bash")
        .args(["-o", "pipefail", "-c", &command])
        .output()
        .expect("run broken-pipe pipeline");
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(!text(&output.stderr).contains("panicked"));
}

#[cfg(target_os = "linux")]
#[test]
fn hook_opens_the_picker_while_stdout_is_captured_and_escape_restores_termios() {
    use std::io::Write;

    let fixture = Fixture::new();
    let hook = write_hook(&fixture, "bash");
    let transcript = fixture.root.path().join("escape.typescript");
    let command = r#"stty cols 120 rows 40; before=$(stty -g); source "$HOOK"; awswit --config-file "$CONFIG" --credentials-file "$CREDENTIALS"; rc=$?; after=$(stty -g); printf '\nTERMIOS|%s|%s\n' "$before" "$after"; exit $rc"#;
    let mut child = Command::new("script")
        .args(["-qefc", command])
        .arg(&transcript)
        .env("PATH", path_with_binary())
        .env("HOOK", hook)
        .env("CONFIG", &fixture.config)
        .env("CREDENTIALS", &fixture.credentials)
        .env("HOME", fixture.root.path())
        .env("XDG_DATA_HOME", fixture.root.path().join("data"))
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn PTY");
    // Synchronize on rendered TUI output instead of sleeping: a slow runner
    // must not consume Escape in the shell before raw mode is active.
    wait_for_transcript(&mut child, &transcript, b"awswit - AWS profiles");
    child
        .stdin
        .take()
        .expect("PTY stdin")
        .write_all(b"\x1b")
        .expect("send Escape");
    let output = wait_with_timeout(child);
    let terminal = text(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(130),
        "{terminal:?} {}",
        text(&output.stderr)
    );
    assert!(terminal.contains("awswit - AWS profiles"), "{terminal:?}");
    let marker = terminal
        .lines()
        .find(|line| line.contains("TERMIOS|"))
        .expect("termios marker");
    let states: Vec<_> = marker.trim().split('|').collect();
    assert!(states.len() >= 3, "{marker:?}");
    assert_eq!(
        states[1], states[2],
        "terminal mode was not restored: {marker:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn sigterm_restores_termios_before_terminating() {
    let fixture = Fixture::new();
    let transcript = fixture.root.path().join("signal.typescript");
    let command = r#"stty cols 120 rows 40; before=$(stty -g); env AWSWIT_HOOK=1 XDG_DATA_HOME="$DATA" TERM=xterm-256color awswit activate --config-file "$CONFIG" --credentials-file "$CREDENTIALS"; rc=$?; after=$(stty -g); printf '\nSIGNAL_TERMIOS|%s|%s\n' "$before" "$after"; exit $rc"#;
    let mut child = Command::new("script")
        .args(["-qefc", command])
        .arg(&transcript)
        .env("PATH", path_with_binary())
        .env("CONFIG", &fixture.config)
        .env("CREDENTIALS", &fixture.credentials)
        .env("DATA", fixture.root.path().join("data"))
        .env("HOME", fixture.root.path())
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn signal PTY");

    let held_stdin = child.stdin.take().expect("signal PTY stdin");

    wait_for_transcript(&mut child, &transcript, b"awswit - AWS profiles");

    let tty_process = (0..100)
        .find_map(|_| {
            let shell = Command::new("pgrep")
                .args(["-P", &child.id().to_string()])
                .output()
                .ok()
                .and_then(|output| text(&output.stdout).lines().next()?.parse::<u32>().ok());
            let picker = shell.and_then(|shell| {
                Command::new("pgrep")
                    .args(["-P", &shell.to_string()])
                    .output()
                    .ok()
                    .and_then(|output| text(&output.stdout).lines().next()?.parse::<u32>().ok())
            });
            if picker.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            picker
        })
        .expect("picker process");
    let signal_status = Command::new("kill")
        .args(["-TERM", &tty_process.to_string()])
        .status()
        .expect("send SIGTERM");
    if !signal_status.success() {
        drop(held_stdin);
        let _ = child.kill();
        let output = child
            .wait_with_output()
            .expect("collect failed signal test");
        panic!(
            "picker exited before SIGTERM; stdout={:?}, stderr={:?}",
            text(&output.stdout),
            text(&output.stderr)
        );
    }

    drop(held_stdin);
    let output = wait_with_timeout(child);
    assert!(
        !output.status.success(),
        "SIGTERM unexpectedly became success"
    );
    let terminal = text(&output.stdout);
    let marker = terminal
        .lines()
        .find(|line| line.contains("SIGNAL_TERMIOS|"))
        .expect("signal termios marker");
    let states: Vec<_> = marker.trim().split('|').collect();
    assert!(states.len() >= 3, "{marker:?}");
    assert_eq!(
        states[1], states[2],
        "SIGTERM left the terminal in raw mode: {marker:?}"
    );
    assert!(
        terminal.contains("\u{1b}[?1049l"),
        "alternate screen was not left"
    );
}
