use crate::server_trait::ServerTrait;
use crate::CnrsMessage;
use crate::REDIS_URL;
use crate::WS_ADDR;
use crate::{JoinReq, PeerInfo};
use anyhow::Context;
use async_trait::async_trait;
use futures::prelude::*;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;
use tokio::sync::oneshot;
use websockets::{self, Frame};
#[derive(Serialize, Deserialize, Debug)]
enum WsMessage {
    WgIpMsg(String),
    JoinReqMsg(JoinReq),
}

pub struct Server {}

impl Server {
    async fn process_socket_msg(
        payload: &String,
        sender: &mut Option<oneshot::Sender<String>>,
    ) -> Result<(), anyhow::Error> {
        let data: WsMessage = serde_json::from_str(payload.as_str())
            .with_context(|| format!("Error parsing socket message: '{}'", payload))?;

        if let WsMessage::WgIpMsg(ip) = data {
            if let Some(sender) = sender.take() {
                if let Err(e) = sender.send(ip.clone()) {
                    anyhow::bail!("Error on sending to channel {}", e);
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ServerTrait for Server {
    async fn send_peer_info(&self, data: &String) -> Result<String, anyhow::Error> {
        let ws = websockets::WebSocket::connect(WS_ADDR).await?;
        let (mut ws_read, mut ws_write) = ws.split();
        ws_write.send_text(data.to_owned()).await?;

        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let mut sender = Some(sender);
            while let Ok(msg) = ws_read.receive().await {
                match msg {
                    Frame::Text { payload, .. } => {
                        match Server::process_socket_msg(&payload, &mut sender).await {
                            Ok(_) => break,
                            Err(e) => eprintln!("Error: {:?}", e),
                        };
                    }
                    _ => {}
                }
            }
        });
        if let Ok(wg_ip) = receiver.await {
            return Ok(wg_ip);
        }
        anyhow::bail!("Didn't receive message with wg ip")
    }

    async fn sub_to_changes(
        &self,
        sender: Sender<CnrsMessage>,
        wg_ip: &String,
        room_name: &String,
    ) -> Result<(), anyhow::Error> {
        let client = redis::Client::open(REDIS_URL)?;
        let con = client.get_tokio_connection().await?;

        let room_name = room_name.to_string();
        let wg_ip = wg_ip.to_string();
        let mut rx = sender.subscribe();
        tokio::spawn(async move {
            let mut pubsub = con.into_pubsub();

            if let Err(e) = pubsub.subscribe(room_name.as_str()).await {
                eprintln!("Failed to subscribe to channel: {}", e);
                return;
            }
            let mut pubsub = pubsub.into_on_message();
            loop {
                if let Ok(CnrsMessage::Shutdown) = rx.try_recv() {
                    return;
                }
                let msg = match pubsub.next().await {
                    Some(msg) => msg,
                    None => {
                        continue;
                    }
                };
                let payload: String = match msg.get_payload() {
                    Ok(payload) => payload,
                    Err(err) => {
                        eprintln!("Error on getting msg payload {}", err);
                        continue;
                    }
                };
                if msg.get_channel_name() == room_name {
                    let ws_msg: Result<WsMessage, _> = serde_json::from_str(payload.as_str());
                    if let Ok(data) = ws_msg {
                        if let WsMessage::JoinReqMsg(join_req) = data {
                            println!("{} is trying to join", &join_req.peer_info.username);
                            if *wg_ip != join_req.peer_info.wg_ip {
                                match sender.send(CnrsMessage::PeerDiscovered(join_req)) {
                                    Ok(_) => {}
                                    Err(e) => {
                                        eprintln!("Error is sending broadcast message: {}", e)
                                    }
                                };
                            }
                        }
                    }
                }
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
        let client = redis::Client::open(REDIS_URL)?;
        let mut con = client.get_tokio_connection().await?;
        let len: isize = con.llen(room_name).await?;
        let peers: Vec<String> = con.lrange(room_name, 0, len).await?;
        for peer in peers {
            let peer_info: PeerInfo = serde_json::from_str(peer.as_str())?;
            if peer_info.wg_ip != *wg_ip {
                sender.send(CnrsMessage::PeerDiscovered(JoinReq {
                    room_name: room_name.to_string(),
                    peer_info,
                }))?;
            }
        }
        Ok(())
    }
}
