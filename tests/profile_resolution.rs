//! Profile resolution tests

#[test]
fn test_profile_role_arn_parsing() {
    let full_arn = "arn:aws:iam::123456789012:role/MyRole";
    let shorthand = "123456789012:MyRole";

    let parsed = if shorthand.starts_with("arn:") {
        shorthand.to_string()
    } else if shorthand.contains(':') {
        let parts: Vec<&str> = shorthand.splitn(2, ':').collect();
        if parts.len() == 2 {
            format!("arn:aws:iam::{}:role/{}", parts[0], parts[1])
        } else {
            shorthand.to_string()
        }
    } else {
        shorthand.to_string()
    };

    assert_eq!(parsed, full_arn);
}

#[test]
fn test_shorthand_with_path() {
    let shorthand = "123456789012:admin/MyRole";
    let parsed = if shorthand.starts_with("arn:") {
        shorthand.to_string()
    } else if shorthand.contains(':') {
        let parts: Vec<&str> = shorthand.splitn(2, ':').collect();
        if parts.len() == 2 {
            format!("arn:aws:iam::{}:role/{}", parts[0], parts[1])
        } else {
            shorthand.to_string()
        }
    } else {
        shorthand.to_string()
    };

    assert_eq!(parsed, "arn:aws:iam::123456789012:role/admin/MyRole");
}

#[test]
fn test_full_arn_passthrough() {
    let arn = "arn:aws:iam::123456789012:role/MyRole";
    let parsed = if arn.starts_with("arn:") {
        arn.to_string()
    } else {
        panic!("Should be detected as full ARN")
    };

    assert_eq!(parsed, arn);
}
