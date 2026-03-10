//! Autocomplete helper - outputs profile names for shell completion

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use configparser::ini::Ini;

fn main() {
    // Load profiles quickly without full validation
    let profiles = load_profile_names();
    
    // Output sorted profile names
    let mut names: Vec<_> = profiles.into_iter().collect();
    names.sort();
    
    for name in names {
        println!("{}", name);
    }
}

fn load_profile_names() -> Vec<String> {
    let mut profiles = Vec::new();

    // Load from ~/.aws/config
    let config_path = dirs::home_dir()
        .map(|h| h.join(".aws").join("config"))
        .unwrap_or_default();
    
    if let Some(names) = load_profiles_from_file(&config_path, true) {
        profiles.extend(names);
    }

    // Load from ~/.aws/credentials
    let creds_path = dirs::home_dir()
        .map(|h| h.join(".aws").join("credentials"))
        .unwrap_or_default();
    
    if let Some(names) = load_profiles_from_file(&creds_path, false) {
        profiles.extend(names);
    }

    // Check environment variables
    if let Ok(config_file) = std::env::var("AWS_CONFIG_FILE") {
        if let Some(names) = load_profiles_from_file(Path::new(&config_file), true) {
            profiles.extend(names);
        }
    }

    if let Ok(creds_file) = std::env::var("AWS_SHARED_CREDENTIALS_FILE") {
        if let Some(names) = load_profiles_from_file(Path::new(&creds_file), false) {
            profiles.extend(names);
        }
    }

    // Deduplicate
    profiles.sort();
    profiles.dedup();

    // Filter out auto-refresh profiles
    profiles.retain(|p| !p.starts_with("autoawswit-") && !p.starts_with("auto-refresh-"));

    profiles
}

fn load_profiles_from_file(path: &Path, is_config: bool) -> Option<Vec<String>> {
    if !path.exists() {
        return None;
    }

    let content = fs::read_to_string(path).ok()?;
    let mut ini = Ini::new_cs();
    ini.read(content).ok()?;

    let mut names = Vec::new();
    
    for section in ini.sections() {
        let profile_name = if is_config && section.starts_with("profile ") {
            section.strip_prefix("profile ").unwrap().to_string()
        } else if section == "default" || !is_config {
            section.clone()
        } else {
            continue;
        };
        
        names.push(profile_name);
    }

    Some(names)
}
