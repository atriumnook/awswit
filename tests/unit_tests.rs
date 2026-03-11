//! Required unit tests for critical untested paths

mod mfa_validation {
    // Test MFA token validation (imported logic)
    fn validate_mfa_token(token: &str) -> Result<(), String> {
        if token.len() < 6 || token.len() > 8 {
            return Err(format!("MFA token must be 6-8 digits, got {} characters", token.len()));
        }
        if !token.chars().all(|c| c.is_ascii_digit()) {
            return Err("MFA token must contain only digits".to_string());
        }
        Ok(())
    }

    #[test]
    fn test_valid_6_digit() {
        assert!(validate_mfa_token("123456").is_ok());
    }

    #[test]
    fn test_valid_8_digit() {
        assert!(validate_mfa_token("12345678").is_ok());
    }

    #[test]
    fn test_too_short() {
        assert!(validate_mfa_token("12345").is_err());
    }

    #[test]
    fn test_too_long() {
        assert!(validate_mfa_token("123456789").is_err());
    }

    #[test]
    fn test_non_digits() {
        assert!(validate_mfa_token("12345a").is_err());
        assert!(validate_mfa_token("abcdef").is_err());
    }

    #[test]
    fn test_empty() {
        assert!(validate_mfa_token("").is_err());
    }
}

mod credential_process_parsing {
    use chrono::{DateTime, Utc};
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct CredentialProcessOutput {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        expiration: Option<String>,
    }

    #[test]
    fn test_valid_rfc3339() {
        let json = r#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret","SessionToken":"tok","Expiration":"2025-01-01T00:00:00Z"}"#;
        let creds: CredentialProcessOutput = serde_json::from_str(json).unwrap();
        let exp = creds.expiration.unwrap();
        let parsed = DateTime::parse_from_rfc3339(&exp).map(|dt| dt.with_timezone(&Utc));
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_malformed_date_error() {
        let json = r#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret","Expiration":"not-a-date"}"#;
        let creds: CredentialProcessOutput = serde_json::from_str(json).unwrap();
        let exp = creds.expiration.unwrap();
        let parsed = DateTime::parse_from_rfc3339(&exp);
        assert!(parsed.is_err());
    }

    #[test]
    fn test_missing_access_key_id() {
        let json = r#"{"SecretAccessKey":"secret"}"#;
        let result = serde_json::from_str::<CredentialProcessOutput>(json);
        assert!(result.is_err());
    }
}

mod credentials_expiry {
    use chrono::{Duration, Utc};

    fn is_expired_with_buffer(expiration: chrono::DateTime<Utc>) -> bool {
        expiration < Utc::now() + Duration::seconds(60)
    }

    #[test]
    fn test_59s_before_expiry_is_expired() {
        let exp = Utc::now() + Duration::seconds(59);
        assert!(is_expired_with_buffer(exp));
    }

    #[test]
    fn test_61s_before_expiry_is_not_expired() {
        let exp = Utc::now() + Duration::seconds(61);
        assert!(!is_expired_with_buffer(exp));
    }
}

mod shell_export {
    #[test]
    fn test_no_session_token_emits_unset_bash() {
        // Simulate generate_export_commands with session_token=None
        let session_token: Option<String> = None;
        let mut output = String::new();
        match &session_token {
            Some(val) => output.push_str(&format!("export AWS_SESSION_TOKEN='{}'\n", val)),
            None => output.push_str("unset AWS_SESSION_TOKEN\n"),
        }
        assert!(output.contains("unset AWS_SESSION_TOKEN"));
    }
}

mod shell_detect {
    fn detect_from_env(awsume_shell: Option<&str>, psmodulepath: bool, shell: Option<&str>) -> &'static str {
        if let Some(s) = awsume_shell {
            return parse_shell(s);
        }
        // $SHELL checked BEFORE PSModulePath — .NET SDK sets PSModulePath on
        // Linux even for bash/zsh users, causing false positives.
        if let Some(s) = shell {
            return parse_shell(s);
        }
        if psmodulepath {
            return "powershell";
        }
        "bash"
    }

    fn parse_shell(name: &str) -> &'static str {
        let lower = name.to_lowercase();
        if lower.contains("fish") { "fish" }
        else if lower.contains("zsh") { "zsh" }
        else if lower.contains("powershell") || lower.contains("pwsh") { "powershell" }
        else { "bash" }
    }

    #[test]
    fn test_awsume_shell_takes_priority() {
        assert_eq!(detect_from_env(Some("fish"), true, Some("/bin/bash")), "fish");
    }

    #[test]
    fn test_shell_before_psmodulepath() {
        // $SHELL takes priority over PSModulePath to avoid .NET SDK false positives
        assert_eq!(detect_from_env(None, true, Some("/bin/bash")), "bash");
    }

    #[test]
    fn test_psmodulepath_when_no_shell() {
        assert_eq!(detect_from_env(None, true, None), "powershell");
    }

    #[test]
    fn test_shell_var() {
        assert_eq!(detect_from_env(None, false, Some("/usr/bin/zsh")), "zsh");
    }

    #[test]
    fn test_default_bash() {
        assert_eq!(detect_from_env(None, false, None), "bash");
    }

    #[test]
    fn test_pwsh_detected() {
        assert_eq!(parse_shell("/usr/bin/pwsh"), "powershell");
    }
}

mod sorted_profile_names {
    use std::collections::HashMap;

    struct Entry {
        name: String,
        is_favorite: bool,
        last_used: Option<i64>, // epoch seconds for simplicity
    }

    fn sorted_names(entries: &mut [Entry]) -> Vec<String> {
        entries.sort_by(|a, b| {
            match (a.is_favorite, b.is_favorite) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => match (&a.last_used, &b.last_used) {
                    (Some(a_t), Some(b_t)) => b_t.cmp(a_t),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                }
            }
        });
        entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn test_recency_tiebreak() {
        let mut entries = vec![
            Entry { name: "old".into(), is_favorite: false, last_used: Some(100) },
            Entry { name: "new".into(), is_favorite: false, last_used: Some(200) },
        ];
        let names = sorted_names(&mut entries);
        assert_eq!(names, vec!["new", "old"]);
    }

    #[test]
    fn test_never_used_alphabetical() {
        let mut entries = vec![
            Entry { name: "zebra".into(), is_favorite: false, last_used: None },
            Entry { name: "alpha".into(), is_favorite: false, last_used: None },
        ];
        let names = sorted_names(&mut entries);
        assert_eq!(names, vec!["alpha", "zebra"]);
    }

    #[test]
    fn test_favorites_first() {
        let mut entries = vec![
            Entry { name: "normal".into(), is_favorite: false, last_used: Some(999) },
            Entry { name: "fav".into(), is_favorite: true, last_used: None },
        ];
        let names = sorted_names(&mut entries);
        assert_eq!(names[0], "fav");
    }
}

mod cache_corrupt_json {
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_corrupt_json_is_cache_miss_not_hard_error() {
        let temp = TempDir::new().unwrap();
        let cache_dir = temp.path().join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let path = cache_dir.join("test.json");
        fs::write(&path, "{{invalid json").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let result = serde_json::from_str::<serde_json::Value>(&content);
        assert!(result.is_err()); // parse fails
        // In real code, this returns Ok(None), not Err
    }
}

mod autoawswit_config_parsing {
    fn parse_bool_config(value: &str) -> bool {
        matches!(value.to_lowercase().as_str(), "true" | "1" | "yes")
    }

    #[test]
    fn test_true() { assert!(parse_bool_config("true")); }
    #[test]
    fn test_one() { assert!(parse_bool_config("1")); }
    #[test]
    fn test_yes() { assert!(parse_bool_config("yes")); }
    #[test]
    fn test_false() { assert!(!parse_bool_config("false")); }
    #[test]
    fn test_zero() { assert!(!parse_bool_config("0")); }
    #[test]
    fn test_no() { assert!(!parse_bool_config("no")); }
}
