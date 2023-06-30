mod app_backend;
mod kernel_wg;
mod server;
mod server_trait;
mod table_data;
mod toml_conf;
mod wg_trait;

use crate::toml_conf::Config;
use crossterm::style::Print;
use clap::{Args, Parser, Subcommand};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, ExecutableCommand, QueueableCommand};
use std::io::stdout;
use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicBool, Arc};
use table_data::TableData;
use tokio::sync::{broadcast, mpsc};

const PROFILE_PATH: &str = "profile.toml";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate new connection profile
    Generate(GenerateArgs),

    /// Join room with information from profile
    Join(JoinArgs),
}

#[derive(Args)]
struct GenerateArgs {
    room_name: String,
    username: String,
}

#[derive(Args)]
struct JoinArgs {
    profile: String,
}

#[derive(Debug)]
pub enum TableMessage {
    UpdateTable,
    InfoLog(String),
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Generate(args) => {
            println!(
                "Generating config with this information:\n\nroom name: {}\nusername: {}",
                args.room_name, args.username
            );

            let mut conf_path = format!("{}-profile.toml", args.room_name);
            if std::path::Path::new(&conf_path).exists() {
                println!("\nFile {} already exists", conf_path);
                let new_conf_path = app_backend::read_str_from_cli(String::from(
                    "Input new config path (empty to rewrite existing config): ",
                ));
                if !new_conf_path.is_empty() {
                    conf_path = new_conf_path;
                }
            }
            if let Err(err) = Config::generate(
                PROFILE_PATH,
                conf_path.as_str(),
                args.username.as_str(),
                args.room_name.as_str(),
            ) {
                eprintln!("Error during profile generation {}", err);
                anyhow::bail!("Failed to generate profile");
            };
            println!("Your config was saved to {}", conf_path);
            return Ok(());
        }
        Commands::Join(args) => {
            println!("Starting connection...");
            let conf_path = args.profile.clone();
            let profile = Config::from_file(conf_path.as_str());
            if let Err(err) = profile {
                eprintln!("Error during profile loading {}", err);
                anyhow::bail!("Failed to load profile");
            };
        }
    }

    let mut my_wg_ip = String::new();
    let (sender, _) = broadcast::channel(16);
    let (table_sender, mut table_receiver) = mpsc::channel(100);
    let mut peers = TableData::new(table_sender.clone());
    peers.calc_ping();

    let peers = Arc::new(peers);
    if let Err(e) = app_backend::start_backend(
        sender.clone(),
        peers.clone(),
        &mut my_wg_ip,
        table_sender.clone(),
    )
    .await
    {
        anyhow::bail!("Error during backend start: {:?}", e);
    };

    {
        *peers.my_wg_ip.write().await = my_wg_ip;
    }
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let mut username = Config::global().room.username.clone();
    username.truncate(15);

    stdout().execute(cursor::MoveDown(1)).unwrap();
    let logger_task = {
        let mut stdout = std::io::stdout();
        let peers = peers.clone();
        tokio::spawn(async move {
            let mut lines_n = 0;
            loop {
                if let Some(msg) = table_receiver.recv().await {
                    if lines_n != 0 {
                        stdout.queue(cursor::MoveUp(lines_n as u16)).unwrap();
                    }
                    stdout.queue(Clear(ClearType::FromCursorDown)).unwrap();
                    if let TableMessage::InfoLog(log) = msg {
                        stdout.queue(Print(format!("{}\n", log))).unwrap();
                    }

                    lines_n = match peers.update_table().await {
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("Error updating table: {:?}", e);
                            lines_n
                        }
                    };
                    eprintln!("lines - {}", lines_n);
                }
            }
        })
    };
    while running.load(Ordering::SeqCst) {
        table_sender.send(TableMessage::UpdateTable).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    if let Err(e) = sender.send(app_backend::CnrsMessage::Shutdown) {
        eprintln!("Failed to send shutdown message why: {}", e);
    };
    if let Some(handle) = &peers.calc_ping_task {
        handle.abort();
    }

    while let Ok(msg) = sender.subscribe().recv().await {
        if let app_backend::CnrsMessage::FinishedDisconnecting = msg {
            logger_task.abort();
            break;
        }
    }
    Ok(())
}
