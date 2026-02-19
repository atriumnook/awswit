use std::env;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: autoawswit <profile-name>");
        std::process::exit(1);
    }

    let profile_name = &args[1];

    tracing_subscriber::fmt()
        .with_env_filter("awswit=info")
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("Starting auto-refresh daemon for profile: {}", profile_name);

    // The daemon loop is part of the main awswit crate
    // For standalone binary, we reimplement a simplified version
    loop {
        tracing::info!("Auto-refreshing credentials for: {}", profile_name);

        // Use the awswit binary to refresh
        let output = tokio::process::Command::new("awswit")
            .args(["--refresh", "--show-commands", profile_name])
            .output()
            .await;

        match output {
            Ok(out) => {
                if out.status.success() {
                    tracing::info!("Credentials refreshed successfully");
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    tracing::error!("Failed to refresh: {}", stderr);
                }
            }
            Err(e) => {
                tracing::error!("Failed to execute awswit: {}", e);
            }
        }

        // Default refresh interval: 45 minutes
        tokio::time::sleep(tokio::time::Duration::from_secs(2700)).await;
    }
}
