use crate::app_backend::{CnrsMessage, DisconnectReq, JoinReq, PeerInfo, UpdateTimerReq};
use crate::toml_conf::Config;
use crate::server_trait::ServerTrait;
use async_trait::async_trait;
use futures::prelude::*;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;
use crate::logger::{log, LogLevel};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum InfoMsg {
    JoinMsg(JoinReq),
    DisconnectMsg(DisconnectReq),
    UpdateLastConnected(UpdateTimerReq),
}

pub struct Server {}

#[async_trait]
impl ServerTrait for Server {
    async fn send_peer_info(&self, data: JoinReq) -> Result<String, anyhow::Error> {
        let data: String = serde_json::to_string(&InfoMsg::JoinMsg(data))?;
        let client = reqwest::Client::new();
        let res = client
            .post(&Config::global().addrs.server_addr)
            .body(data)
            .send()
            .await?;
        if res.status() == 200 {
            let data = res.text().await?;
            return Ok(data);
        }
        anyhow::bail!(
            "Status after post request: {}, msg: {}",
            res.status(),
            res.text().await.unwrap_or("-".to_string())
        );
    }

    async fn sub_to_changes(
        &self,
        sender: Sender<CnrsMessage>,
        wg_ip: &String,
        room_name: &String,
    ) -> Result<(), anyhow::Error> {
        let client = redis::Client::open(Config::global().addrs.redis_addr.clone())?;
        let con = client.get_tokio_connection().await?;

        let room_name = room_name.to_string();
        let wg_ip = wg_ip.to_string();
        let mut rx = sender.subscribe();
        tokio::spawn(async move {
            let mut pubsub = con.into_pubsub();

            if let Err(e) = pubsub.subscribe(room_name.as_str()).await {
                log!(LogLevel::Error,"Failed to subscribe to channel: {}", e);
                return;
            }
            let mut pubsub = pubsub.into_on_message();
            loop {
                tokio::select! {
                    broadcast_msg = rx.recv() => {
                        if let Ok(CnrsMessage::Shutdown) = broadcast_msg {
                            return;
                        }
                    }
                    msg = pubsub.next()  => {
                        let msg = match msg {
                            Some(msg) => msg,
                            None => {
                                continue;
                            }
                        };
                        let payload: String = match msg.get_payload() {
                            Ok(payload) => payload,
                            Err(err) => {
                                log!(LogLevel::Error,"Error on getting msg payload {}", err);
                                continue;
                            }
                        };
                        if msg.get_channel_name() == room_name {
                            let msg: Result<InfoMsg, _> = serde_json::from_str(payload.as_str());
                            if let Ok(data) = msg {
                                match data {
                                    InfoMsg::JoinMsg(data) => {
                                        // println!("{} is trying to join", &data.peer_info.username);
                                        if *wg_ip != data.peer_info.wg_ip {
                                            match sender.send(CnrsMessage::PeerDiscovered(data)) {
                                                Ok(_) => {}
                                                Err(e) => {
                                                    log!(LogLevel::Error,"Error is sending broadcast message: {}", e)
                                                }
                                            };
                                        }
                                    },
                                    InfoMsg::DisconnectMsg(data) => {

                                        match sender.send(CnrsMessage::PeerDisconnected(data)) {
                                            Ok(_) => {}
                                            Err(e) => {
                                                log!(LogLevel::Error,"Error is sending broadcast message: {}", e)
                                            }
                                        };

                                    },
                                    _ => {}
                                }


                            }
                        }
                    }
                };
            }
        });
        Ok(())
    }
    async fn connect_to_each(
        &self,
        sender: Sender<CnrsMessage>,
        room_name: &String,
        wg_ip: &String,
    ) -> Result<(), anyhow::Error> {
        let client = redis::Client::open(Config::global().addrs.redis_addr.clone())?;
        let mut con = client.get_tokio_connection().await?;
        let len: isize = con.llen(room_name).await?;
        let peers: Vec<String> = con.lrange(room_name, 0, len).await?;
        for peer in peers {
            let peer_info: PeerInfo = serde_json::from_str(peer.as_str())?;
            if peer_info.is_online && peer_info.wg_ip != *wg_ip {
                sender.send(CnrsMessage::PeerDiscovered(JoinReq {
                    room_name: room_name.to_string(),
                    is_reconnecting: false,
                    peer_info,
                }))?;
            }
        }
        Ok(())
    }
    async fn send_disconnect_signal(&self, data: DisconnectReq) -> Result<(), anyhow::Error> {
        let data = serde_json::to_string(&InfoMsg::DisconnectMsg(data))?;
        let client = reqwest::Client::new();
        let res = client
            .post(&Config::global().addrs.server_addr)
            .body(data)
            .send()
            .await?;
        let status = res.status();
        if status != 200 {
            let data = res.text().await?;
            anyhow::bail!("Status after post request: {}, msg: {}", status, data);
        }
        Ok(())
    }
    async fn update_connection_time(&self, data: UpdateTimerReq) -> Result<(), anyhow::Error> {
        let data: String = serde_json::to_string(&InfoMsg::UpdateLastConnected(data))?;
        let client = reqwest::Client::new();
        let res = client
            .post(&Config::global().addrs.server_addr)
            .body(data)
            .send()
            .await?;
        if res.status() == 200 {
            return Ok(());
        }
        anyhow::bail!(
            "Status after post request: {}, msg: {}",
            res.status(),
            res.text().await.unwrap_or("-".to_string())
        );
    }
}
