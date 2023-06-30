use crate::app_backend::{DisconnectReq, PeerInfo};
use crate::toml_conf::Config;
use crate::TableMessage;
use crossterm::style::Print;
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
    pub sender: tokio::sync::mpsc::Sender<TableMessage>,
}

impl TableData {
    pub fn new(sender: tokio::sync::mpsc::Sender<TableMessage>) -> Self {
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
        if let Err(e) = self.sender.send(TableMessage::UpdateTable).await {
            eprintln!("Error sending update request: {:?}", e);
        };
    }
    pub async fn remove_peer(&self, info: &DisconnectReq) {
        {
            self.data
                .write()
                .await
                .retain(|peer| *peer.pub_key != info.pub_key);
        }
        if let Err(e) = self.sender.send(TableMessage::UpdateTable).await {
            eprintln!("Error sending update request: {:?}", e);
        };
    }

    pub async fn update_table(&self) -> Result<u8, anyhow::Error> {
        let mut lines_n = 5;
        let mut stdout = stdout();
        let peers = &(*self.data.read().await);
        // eprintln!("3");
        //tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        println!(
            "\nRoom: {}\n\n{:<15} {:^15} {:^8}",
            Config::global().room.room_name.as_str(),
            "USERNAME",
            "Local IP",
            "PING"
        );
        println!(
            "{:<15} {:^15} {:^8}",
            Config::global().room.username.as_str(),
            self.my_wg_ip.read().await,
            "0 ms"
        );
        for peer in peers {
            lines_n += 1;
            let ping_millis = peer.ping.load(Ordering::SeqCst);
            let ping_str = match ping_millis {
                u16::MAX => "-".to_string(),
                1000u16.. => ">999 ms".to_string(),
                _ => format!("{:?} ms", ping_millis),
            };
            let mut username = peer.username.clone();
            username.truncate(15);
            stdout.queue(Print(format!(
                "{:<15} {:^15} {:^8}\n",
                &username, peer.wg_ip, ping_str
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
    async fn get_ping(ip: &String) -> Result<u16, anyhow::Error> {
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
