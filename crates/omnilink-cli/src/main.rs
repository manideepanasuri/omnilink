//! Omnilink CLI client.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug, PartialEq)]
#[command(
    name = "omnilink",
    author,
    version,
    about = "Omnilink CLI client for Linux-to-Android integration",
    long_about = "A command-line interface for communicating with omnilink-daemon, controlling paired devices, sending files, and streaming Android status to Waybar."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Commands {
    /// Query connection status and device telemetry
    Status {
        /// Stream status continuously in Waybar JSON format
        #[arg(short = 'w', long = "watch-waybar")]
        watch_waybar: bool,
    },

    /// Send a file to the connected Android device
    Send {
        /// Absolute or relative path of the file to send
        #[arg(value_name = "FILE")]
        path: PathBuf,
    },

    /// Launch low-latency screen or single-app mirroring
    Mirror {
        /// Android package name to launch in virtual display (e.g., com.spotify.music)
        #[arg(short, long)]
        package: Option<String>,

        /// Request a floating window in Hyprland
        #[arg(short, long, default_value_t = true)]
        floating: bool,
    },

    /// Android notification actions
    Notification {
        #[command(subcommand)]
        action: NotificationCommands,
    },

    /// Remote device controls
    Device {
        #[command(subcommand)]
        action: DeviceCommands,
    },

    /// Generate pairing QR code and connection credentials for Android companion onboarding
    Pair {
        /// Port for the Omnilink QUIC endpoint
        #[arg(short, long, default_value_t = 42424)]
        port: u16,

        /// Output pairing payload as raw JSON only without QR code
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum NotificationCommands {
    /// Send an inline reply to an Android notification
    Reply {
        /// Android notification key / ID
        #[arg(short, long)]
        id: String,

        /// Reply text message
        #[arg(short, long)]
        text: String,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum DeviceCommands {
    /// Toggle mobile hotspot on the phone
    ToggleHotspot {
        /// Enable or disable mobile hotspot
        #[arg(long)]
        enable: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    execute_command(cli.command).await
}

async fn execute_command(command: Commands) -> Result<()> {
    match command {
        Commands::Status { watch_waybar } => {
            if watch_waybar {
                println!(
                    "{}",
                    serde_json::json!({
                        "text": "Connected",
                        "tooltip": "Omnilink Daemon Connected",
                        "class": "connected"
                    })
                );
            } else {
                println!("Device: Connected (telemetry placeholder)");
            }
        }
        Commands::Send { path } => {
            println!("Sending file: {}", path.display());
        }
        Commands::Mirror { package, floating } => {
            println!(
                "Launching mirror session (package: {:?}, floating: {})",
                package, floating
            );
        }
        Commands::Notification { action } => match action {
            NotificationCommands::Reply { id, text } => {
                println!("Replying to notification {}: {}", id, text);
            }
        },
        Commands::Device { action } => match action {
            DeviceCommands::ToggleHotspot { enable } => {
                println!("Toggling mobile hotspot: enable={}", enable);
            }
        },
        Commands::Pair { port, json } => {
            let identity = omnilink_daemon::identity::load_or_generate_identity(None)?;
            let payload = omnilink_daemon::pairing::generate_pairing_payload(&identity, port)?;

            if json {
                println!("{}", payload.to_json_pretty()?);
            } else {
                let json_data = payload.to_json()?;
                let qr_code = omnilink_daemon::pairing::render_qr_code(&json_data)?;

                println!("\n=== OMNILINK PAIRING ===");
                println!("Scan this QR code with the Omnilink Android app to pair:\n");
                println!("{}", qr_code);
                println!("PAIRING PAYLOAD:");
                println!("{}", payload.to_json_pretty()?);
                println!("\nPAIRING SUMMARY:");
                println!("  Hostname:   {}", payload.name);
                println!("  Endpoints:  {}", payload.addresses.join(", "));
                println!("  Public Key: {}", payload.public_key_base64);
                println!("  Token:      {}", payload.token);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status_basic() {
        let cli = Cli::try_parse_from(["omnilink", "status"]).unwrap();
        assert_eq!(
            cli.command,
            Commands::Status {
                watch_waybar: false
            }
        );
    }

    #[test]
    fn test_parse_status_waybar() {
        let cli = Cli::try_parse_from(["omnilink", "status", "--watch-waybar"]).unwrap();
        assert_eq!(cli.command, Commands::Status { watch_waybar: true });

        let cli_short = Cli::try_parse_from(["omnilink", "status", "-w"]).unwrap();
        assert_eq!(cli_short.command, Commands::Status { watch_waybar: true });
    }

    #[test]
    fn test_parse_send() {
        let cli = Cli::try_parse_from(["omnilink", "send", "/path/to/file.pdf"]).unwrap();
        assert_eq!(
            cli.command,
            Commands::Send {
                path: PathBuf::from("/path/to/file.pdf")
            }
        );
    }

    #[test]
    fn test_parse_mirror_default() {
        let cli = Cli::try_parse_from(["omnilink", "mirror"]).unwrap();
        assert_eq!(
            cli.command,
            Commands::Mirror {
                package: None,
                floating: true,
            }
        );
    }

    #[test]
    fn test_parse_mirror_with_package() {
        let cli = Cli::try_parse_from([
            "omnilink",
            "mirror",
            "--package",
            "com.spotify.music",
            "--floating",
        ])
        .unwrap();
        assert_eq!(
            cli.command,
            Commands::Mirror {
                package: Some("com.spotify.music".to_string()),
                floating: true,
            }
        );
    }

    #[test]
    fn test_parse_notification_reply() {
        let cli = Cli::try_parse_from([
            "omnilink",
            "notification",
            "reply",
            "--id",
            "notif-123",
            "--text",
            "Hello world",
        ])
        .unwrap();
        assert_eq!(
            cli.command,
            Commands::Notification {
                action: NotificationCommands::Reply {
                    id: "notif-123".to_string(),
                    text: "Hello world".to_string(),
                }
            }
        );
    }

    #[test]
    fn test_parse_device_hotspot() {
        let cli =
            Cli::try_parse_from(["omnilink", "device", "toggle-hotspot", "--enable"]).unwrap();
        assert_eq!(
            cli.command,
            Commands::Device {
                action: DeviceCommands::ToggleHotspot { enable: true }
            }
        );
    }

    #[tokio::test]
    async fn test_execute_status_command() {
        let result = execute_command(Commands::Status {
            watch_waybar: false,
        })
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_pair_default() {
        let cli = Cli::try_parse_from(["omnilink", "pair"]).unwrap();
        assert_eq!(
            cli.command,
            Commands::Pair {
                port: 42424,
                json: false,
            }
        );
    }

    #[test]
    fn test_parse_pair_custom_port_and_json() {
        let cli = Cli::try_parse_from(["omnilink", "pair", "--port", "50000", "--json"]).unwrap();
        assert_eq!(
            cli.command,
            Commands::Pair {
                port: 50000,
                json: true,
            }
        );
    }

    #[tokio::test]
    async fn test_execute_pair_command() {
        let result = execute_command(Commands::Pair {
            port: 42424,
            json: true,
        })
        .await;
        assert!(result.is_ok());
    }
}
