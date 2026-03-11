//! Profile resolution tests using real awswit::cli::Args

use awswit::cli::Args;

#[test]
fn test_profile_role_arn_shorthand() {
    let args = Args {
        role_arn: Some("123456789012:MyRole".to_string()),
        ..Default::default()
    };
    assert_eq!(
        args.resolve_role_arn(),
        Some("arn:aws:iam::123456789012:role/MyRole".to_string())
    );
}

#[test]
fn test_shorthand_with_path() {
    let args = Args {
        role_arn: Some("123456789012:admin/MyRole".to_string()),
        ..Default::default()
    };
    assert_eq!(
        args.resolve_role_arn(),
        Some("arn:aws:iam::123456789012:role/admin/MyRole".to_string())
    );
}

#[test]
fn test_full_arn_passthrough() {
    let args = Args {
        role_arn: Some("arn:aws:iam::123456789012:role/MyRole".to_string()),
        ..Default::default()
    };
    assert_eq!(
        args.resolve_role_arn(),
        Some("arn:aws:iam::123456789012:role/MyRole".to_string())
    );
}

#[test]
fn test_no_role_arn() {
    let args = Args::default();
    assert_eq!(args.resolve_role_arn(), None);
}
