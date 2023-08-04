use crate::app_backend::PeerInfo;
use crate::logger::{log, LogLevel};
use crate::toml_conf::Config;
use crate::UIMessage;
use crossterm::style::{Color, Print, Stylize};
use crossterm::QueueableCommand;
use std::io::{stdout, Write};
use std::sync::{
    atomic::{AtomicU16, Ordering},
    Arc,
};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct TableItem {
    pub username: String,
    pub wg_ip: String,
    pub ping: AtomicU16,
    pub pub_key: String,
}

#[derive(Debug)]
pub struct TableData {
    pub data: Arc<RwLock<Vec<TableItem>>>,
    pub calc_ping_task: Option<JoinHandle<()>>,
    pub my_wg_ip: RwLock<String>,
    pub sender: tokio::sync::mpsc::Sender<UIMessage>,
}

impl TableData {
    pub fn new(sender: tokio::sync::mpsc::Sender<UIMessage>) -> Self {
        TableData {
            data: Arc::new(RwLock::new(Vec::new())),
            calc_ping_task: None,
            my_wg_ip: RwLock::new("".to_string()),
            sender,
        }
    }
    pub async fn add_peer(&self, info: &PeerInfo) {
        let displ_data = TableItem {
            username: info.username.clone(),
            wg_ip: info.wg_ip.clone(), 
            ping: AtomicU16::new(u16::MAX),
            pub_key: info.pub_key.clone(),
        };
        {
            self.data.write().await.push(displ_data);
        }
        if let Err(e) = self.sender.send(UIMessage::UpdateTable).await {
            log!(LogLevel::Error, "Error sending update request: {:?}", e);
        };
    }
    pub async fn remove_peer(&self, pub_key: &str) {
        {
            let pub_key = pub_key.to_owned();
            self.data
                .write()
                .await
                .retain(|peer| *peer.pub_key != pub_key);
        }
        if let Err(e) = self.sender.send(UIMessage::UpdateTable).await {
            log!(LogLevel::Error, "Error sending update request: {:?}", e);
        };
    }

    pub async fn update_table(&self) -> Result<u8, anyhow::Error> {
        let mut lines_n = 5;
        let mut stdout = stdout();
        let peers = &(*self.data.read().await);
        let styled_dot = "●".with(Color::Green);
        stdout
            .queue(Print(format!(
                "\nRoom: {}\n\n{:<15} {:^15} {:>7}\n",
                Config::global().room.room_name.as_str(),
                "  USERNAME",
                "Local IP",
                "PING"
            )))?
            .queue(Print(format!(
                "{} {:<13} {:^15} {:>7}\n",
                styled_dot,
                Config::global().room.username.as_str(),
                self.my_wg_ip.read().await,
                "0 ms"
            )))?;

        for peer in peers {
            lines_n += 1;
            let ping_millis = peer.ping.load(Ordering::SeqCst);
            let mut dot_color = Color::Red;
            let ping_str = match ping_millis {
                u16::MAX => "-".to_string(),
                1000.. => ">999 ms".to_string(),
                350.. => {
                    dot_color = Color::Yellow;
                    format!("{:?} ms", ping_millis)
                }
                _ => {
                    dot_color = Color::Green;
                    format!("{:?} ms", ping_millis)
                }
            };
            let styled_dot = "●".with(dot_color);
            let mut username = peer.username.clone();
            username.truncate(15);
            stdout.queue(Print(format!(
                "{} {:<13} {:^15} {:>7}\n",
                styled_dot, &username, peer.wg_ip, ping_str
            )))?;
        }

        stdout.flush()?;
        Ok(lines_n)
    }

    pub fn calc_ping(&mut self) {
        let data = self.data.clone();
        self.calc_ping_task.get_or_insert(tokio::spawn(async move {
            loop {
                {
                    let peers = data.read().await;
                    for (i, peer) in peers.iter().enumerate() {
                        let ping = TableData::get_ping(&peer.wg_ip).await;
                        let res_ping = if let Ok(ping) = ping { ping } else { u16::MAX };
                        peers[i].ping.store(res_ping, Ordering::SeqCst);
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }));
    }
    async fn get_ping(ip: &str) -> Result<u16, anyhow::Error> {
        let payload = [0; 8];
        match surge_ping::ping(ip.parse()?, &payload).await {
            Ok((_packet, duration)) => {
                let res = duration.as_millis();
                if res > u16::MAX as u128 {
                    return Ok(u16::MAX);
                }
                Ok(duration.as_millis() as u16)
            }
            Err(_) => anyhow::bail!("Error while pinging"),
        }
    }
}
