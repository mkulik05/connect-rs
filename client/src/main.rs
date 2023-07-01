mod app_backend;
mod kernel_wg;
mod logger;
mod server;
mod server_trait;
mod table_data;
mod toml_conf;
mod wg_trait;

use crate::logger::Logger;
use crate::logger::{log, LogLevel};
use crate::toml_conf::Config;
use clap::{Args, Parser, Subcommand};
use crossterm::style::Print;
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
pub enum UIMessage {
    Shutdown,
    UpdateTable,
    InfoLog(String),
    ErrorLog(String),
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let (table_sender, mut table_receiver) = mpsc::channel(100);
    let cli = Cli::parse();
    match &cli.command {
        Commands::Generate(args) => {
            Logger::init(
                chrono::Local::now()
                    .format("/tmp/%d_%m_%Y_%H_%M_%S.txt")
                    .to_string(),
                None,
            )?;
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
                log!(LogLevel::ERROR, "Error during profile generation: {}", err);
                anyhow::bail!("Failed to generate profile");
            };
            println!("Your config was saved to {}", conf_path);
            return Ok(());
        }
        Commands::Join(args) => {
            Logger::init(
                chrono::Local::now()
                    .format("/tmp/%d_%m_%Y_%H_%M_%S.txt")
                    .to_string(),
                Some(table_sender.clone()),
            )?;
            println!("Starting connection...");
            let conf_path = args.profile.clone();
            let profile = Config::from_file(conf_path.as_str());
            if let Err(err) = profile {
                log!(LogLevel::ERROR, "Error during blank profile loading {}", err);
                anyhow::bail!("Failed to load profile");
            };
        }
    }

    let mut my_wg_ip = String::new();
    let (sender, _) = broadcast::channel(16);

    let mut peers = TableData::new(table_sender.clone());
    peers.calc_ping();
    let peers = Arc::new(peers);

    if let Err(e) = stdout().execute(cursor::MoveDown(1)) {
        log!(LogLevel::ERROR, "Error is table loop: {}", e);
    };

    let logger_task = {
        let peers = peers.clone();
        tokio::spawn(async move {
            let mut stdout = std::io::stdout();
            let mut lines_n = 0;
            let mut should_stop = false;

            while !should_stop {
                if let Some(msg) = table_receiver.recv().await {
                    match msg {
                        UIMessage::Shutdown => break,
                        UIMessage::InfoLog(log) | UIMessage::ErrorLog(log) => {
                            if lines_n != 0 {
                                if let Err(e) = stdout.queue(cursor::MoveUp(lines_n as u16)) {
                                    log!(LogLevel::ERROR, "Error is table loop: {}", e);
                                };
                            }
                            if let Err(e) = stdout.queue(Clear(ClearType::FromCursorDown)) {
                                log!(LogLevel::ERROR, "Error is table loop: {}", e);
                            };
                            if let Err(e) = stdout.queue(Print(format!("{}\n", log))) {
                                log!(LogLevel::ERROR, "Error is table loop: {}", e);
                            };

                            while let Ok(msg) = table_receiver.try_recv() {

                                match msg {
                                    UIMessage::Shutdown => should_stop = true,
                                    UIMessage::InfoLog(log) | UIMessage::ErrorLog(log) => {
                                        if let Err(e) = stdout.queue(Print(format!("{}\n", log))) {
                                            log!(LogLevel::ERROR, "Error is table loop: {}", e);
                                        };
                                    }
                                    _ => {}
                                }
                            }

                            lines_n = match peers.update_table().await {
                                Ok(n) => n,
                                Err(e) => {
                                    log!(LogLevel::ERROR, "Error updating table: {:?}", e);
                                    lines_n
                                }
                            };
                        }
                        _ => {}
                    }
                }
            }
        })
    };

    if let Err(e) = app_backend::start_backend(sender.clone(), peers.clone(), &mut my_wg_ip).await {
        log!(LogLevel::FATAL, "Error during backend start: {:?}", e);
        if let Err(e) = table_sender.send(UIMessage::Shutdown).await {
            log!(
                LogLevel::ERROR,
                "Failed to send shutdown message to table: {}",
                e
            );
        };
        if let Err(e) = logger_task.await {
            log!(LogLevel::ERROR, "Failed waiting for table task: {}", e);
        };
        return Ok(());
    }

    {
        *peers.my_wg_ip.write().await = my_wg_ip;
    }
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    while running.load(Ordering::SeqCst) {
        if let Err(e) = table_sender.send(UIMessage::UpdateTable).await {
            log!(
                LogLevel::ERROR,
                "Error while sending table update request: {}",
                e
            );
        };
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    if let Err(e) = sender.send(app_backend::CnrsMessage::Shutdown) {
        log!(LogLevel::ERROR, "Failed to send shutdown message: {}", e);
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
