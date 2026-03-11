//! Unit tests that exercise real awswit library code instead of local reimplementations.

mod mfa_validation {
    use awswit::profile::validate_mfa_token;

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

mod credentials_expiry {
    use awswit::aws::Credentials;
    use chrono::{Duration, Utc};

    #[test]
    fn test_expired_credentials() {
        let creds = Credentials {
            expiration: Some(Utc::now() - Duration::hours(1)),
            ..Default::default()
        };
        assert!(creds.is_expired());
    }

    #[test]
    fn test_not_expired_credentials() {
        let creds = Credentials {
            expiration: Some(Utc::now() + Duration::hours(1)),
            ..Default::default()
        };
        assert!(!creds.is_expired());
    }

    #[test]
    fn test_no_expiration_not_expired() {
        let creds = Credentials::default();
        assert!(!creds.is_expired());
    }
}

mod shell_export {
    use awswit::shell::ShellExporter;
    use awswit::shell::ShellType;

    #[test]
    fn test_no_session_token_emits_unset_bash() {
        let creds = awswit::aws::Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_export_commands(&creds, "test");
        assert!(output.contains("unset AWS_SESSION_TOKEN"));
        assert!(output.contains("unset AWS_SECURITY_TOKEN"));
    }

    #[test]
    fn test_shell_output_rejects_newline_in_profile() {
        let creds = awswit::aws::Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output(&creds, "evil\nprofile");
        assert!(result.is_err());
    }
}

mod credential_file_validation {
    use awswit::autorefresh::credentials_file::{validate_credential_value, validate_profile_name};

    #[test]
    fn shell_quote_injection_in_profile_name() {
        assert!(validate_profile_name("evil]\n[injected").is_err());
    }

    #[test]
    fn control_chars_in_credential_value() {
        assert!(validate_credential_value("key", "value\x00null").is_err());
    }

    #[test]
    fn section_header_injection_in_value() {
        assert!(validate_credential_value("key", "[injected]").is_err());
    }

    #[test]
    fn valid_profile_name() {
        assert!(validate_profile_name("autoawswit-my.profile_1").is_ok());
    }

    #[test]
    fn valid_credential_value() {
        assert!(validate_credential_value("key", "AKIAIOSFODNN7EXAMPLE").is_ok());
    }
}

