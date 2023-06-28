mod app_backend;
mod kernel_wg;
mod server;
mod server_trait;
mod table_data;
mod toml_conf;
mod wg_trait;

use crate::toml_conf::Config;
use crossterm::style::Print;
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, ExecutableCommand, QueueableCommand};
use std::io::{stdout, Stdout, Write};
use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicBool, Arc};
use table_data::TableData;
use tokio::sync::broadcast;

const PROFILE_PATH: &str = "profile.toml";

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 1 {
        println!("To generate new connection config enter required info");
        let room_name = app_backend::read_str_from_cli(String::from("Input room id: "));
        let username = app_backend::read_str_from_cli(String::from("\nInput your username: "));
        let conf_path = app_backend::read_str_from_cli(String::from("\nInput new config path: "));
        if let Err(err) = Config::generate(
            PROFILE_PATH,
            conf_path.as_str(),
            username.as_str(),
            room_name.as_str(),
        ) {
            eprintln!("Error during profile generation {}", err);
            anyhow::bail!("Failed to generate profile");
        };
        println!("Your config was saved to {}", conf_path);
        return Ok(());
    }
    println!("Starting connection...");

    let conf_path = args[1].clone();
    let profile = Config::from_file(conf_path.as_str());
    if let Err(err) = profile {
        eprintln!("Error during profile loading {}", err);
        anyhow::bail!("Failed to load profile");
    };

    let mut my_wg_ip = String::new();
    let (tx, _) = broadcast::channel(16);
    let mut peers = TableData::new();
    peers.calc_ping();
    let peers = Arc::new(peers);
    if let Err(e) = app_backend::start_backend(tx.clone(), peers.clone(), &mut my_wg_ip).await {
        anyhow::bail!("Error during backend start: {:?}", e);
    };

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        eprintln!("Handled");
    })
    .expect("Error setting Ctrl-C handler");

    let mut stdout = stdout();
    stdout.execute(Clear(ClearType::All))?;
    stdout.execute(cursor::MoveToRow(1))?;
    println!(
        "Room: {}\n\n{:^15} {:^15} {:^8}",
        Config::global().room.room_name.as_str(), "USERNAME", "Local IP", "PING"
    );
    println!(
        "{:<15} {:^15} {:^8}",
        &Config::global().room.username,
        my_wg_ip,
        "-"
    );
    stdout.execute(cursor::SavePosition)?;
    while running.load(Ordering::SeqCst) {
        update_table(&mut stdout, &peers).await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    stdout.execute(Clear(ClearType::All)).unwrap();
    if let Err(e) = tx.send(app_backend::CnrsMessage::Shutdown) {
        eprintln!("Failed to send shutdown message why: {}", e);
    };
    if let Some(handle) = &peers.calc_ping_task {
        handle.abort();
    }

    while let Ok(msg) = tx.subscribe().recv().await {
        if let app_backend::CnrsMessage::FinishedDisconnecting = msg {
            break;
        }
    }
    Ok(())
}

async fn update_table(stdout: &mut Stdout, peers: &TableData) -> Result<(), anyhow::Error> {
    stdout.execute(cursor::RestorePosition)?;
    stdout.execute(Clear(ClearType::FromCursorDown))?;
    for peer in &(*peers.data.read().await) {
        let ping_millis = peer.ping.load(Ordering::SeqCst);
        let ping_str = match ping_millis {
            u16::MAX => "-".to_string(),
            1000u16.. => ">999".to_string(),
            _ => format!("{:?}", ping_millis),
        };
        let mut username = peer.username.clone();
        username.truncate(15);
        stdout.queue(Print(format!(
            "{:<15} {:^15} {:^8}",
            &username, peer.wg_ip, ping_str
        )))?;
        stdout.queue(cursor::MoveToNextLine(1))?;
    }
    stdout.flush()?;
    Ok(())
}
