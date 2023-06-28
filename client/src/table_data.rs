use crate::app_backend::{CnrsMessage, DisconnectReq, PeerInfo};
use crate::update_table;
use std::io::stdout;
use std::sync::{
    atomic::{AtomicU16, Ordering},
    Arc,
};
use tokio::sync::{broadcast::Sender, RwLock};
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
}

impl TableData {
    pub fn new() -> Self {
        TableData {
            data: Arc::new(RwLock::new(Vec::new())),
            calc_ping_task: None,
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
        if let Err(e) = update_table(&mut stdout(), self).await {
            eprintln!("Error during table update: {:?}", e);
        };
    }
    pub async fn remove_peer(&self, info: &DisconnectReq) {
        {
            self.data
                .write()
                .await
                .retain(|peer| *peer.pub_key != info.pub_key);
        }
        if let Err(e) = update_table(&mut stdout(), self).await {
            eprintln!("Error during table update: {:?}", e);
        };
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
