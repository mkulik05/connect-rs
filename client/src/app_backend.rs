use anyhow::Context;
use chrono::prelude::*;
use nix::unistd::Uid;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::sync::Arc;
use stun_client::*;
use tokio::fs::write;
use tokio::sync::broadcast::Sender;
use wireguard_keys::{self, Privkey};
extern crate redis;
use crate::kernel_wg::KernelWg;
use crate::server::Server;
use crate::server_trait::ServerTrait;
use crate::table_data::TableData;
use crate::toml_conf::Config;
use crate::wg_trait::Wg;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinReq {
    pub room_name: String,
    pub peer_info: PeerInfo,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DisconnectReq {
    pub room_name: String,
    pub pub_key: String,
    pub username: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateTimerReq {
    pub room_name: String,
    pub pub_key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PeerInfo {
    pub wg_ip: String,
    pub pub_key: String,
    pub mapped_addr: String,
    pub username: String,
    pub last_connected: String,
    pub is_online: bool,
}

#[derive(Clone, Debug)]
pub enum CnrsMessage {
    PeerDiscovered(JoinReq),
    PeerDisconnected(DisconnectReq),
    Shutdown,
    FinishedDisconnecting,
}

pub async fn start_backend(
    sender: Sender<CnrsMessage>,
    display: Arc<TableData>,
    my_wg_ip: &mut String,
) -> Result<(), anyhow::Error> {
    let priv_key = Privkey::from_base64(Config::global().room.priv_key.as_str())?;
    let interface_name = Config::global().interface.interface_name.clone();
    let room_name = Config::global().room.room_name.clone();
    let username = Config::global().room.username.clone();

    if !Uid::effective().is_root() {
        println!("You should run this app with root permissions");
        return Ok(());
    }

    let port = get_unused_port();
    // println!("Working with port {}\n", port);

    let public_key = priv_key.pubkey();
    let key_name = Local::now()
        .format("/tmp/connect-wg-%Y_%m_%d_%H_%M_%S.key")
        .to_string();

    match write(&key_name, priv_key.to_base64().as_bytes()).await {
        Ok(_) => {}
        Err(e) => eprintln!("Error saving key: {}", e),
    };

    // println!(
    //     "Public key: {}\nPrivate key: {}\n",
    //     &public_key.to_base64(),
    //     &priv_key.to_base64()
    // );

    // println!("\nJoining room... GLHF");

    let wg: Arc<dyn Wg> = Arc::new(KernelWg {});
    let server: Arc<dyn ServerTrait> = Arc::new(Server {});

    let wg_ip_res = join_room(
        display,
        wg.clone(),
        server.clone(),
        sender.clone(),
        &room_name,
        username.clone(),
        public_key.to_base64(),
        port,
        key_name,
        &interface_name,
    )
    .await;
    match wg_ip_res {
        Ok(ip) => *my_wg_ip = ip,
        Err(e) => anyhow::bail!("Joining room failed: {}", e),
    };
    Ok(())
}

async fn join_room(
    display: Arc<TableData>,
    wg: Arc<dyn Wg>,
    server: Arc<dyn ServerTrait>,
    sender: Sender<CnrsMessage>,
    room_name: &String,
    username: String,
    pub_key: String,
    port: u16,
    key_name: String,
    interface_name: &String,
) -> Result<String, anyhow::Error> {
    let mapped_addr = loop {
        // println!("Geting mapped address");
        match get_mapped_addr("0.0.0.0", port).await {
            Ok(addr) => break addr,
            Err(e) => eprintln!("{}", e),
        }
    };
    // println!("Mapped address: {}\n", mapped_addr);

    let data = PeerInfo {
        wg_ip: "".to_string(),
        last_connected: "".to_string(),
        pub_key: pub_key.clone(),
        is_online: true,
        username: username.clone(),
        mapped_addr,
    };
    let data = JoinReq {
        room_name: room_name.clone(),
        peer_info: data,
    };
    let wg_ip = server.send_peer_info(data).await?;

    {
        let pub_key = pub_key.clone();
        let server = server.clone();
        let room_name = room_name.clone();
        let msg = UpdateTimerReq { room_name, pub_key };
        tokio::spawn(async move {
            loop {
                if let Err(e) = server.update_connection_time(msg.clone()).await {
                    eprintln!("Error updating last connection time: {}", e);
                };
                tokio::time::sleep(std::time::Duration::from_secs(3600 * 6)).await;
            }
        });
    }

    // println!("Connect info sent");
    wg.init_wg(interface_name, port, key_name.as_str(), wg_ip.as_str())
        .await
        .with_context(|| "failed to init wg")?;

    let mut rx2 = sender.subscribe();
    let wg = wg.clone();
    let interface_name = interface_name.clone();
    let pub_key = pub_key.clone();
    {
        let sender = sender.clone();
        let server = server.clone();
        let room_name = room_name.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(data) = rx2.recv().await {
                    match data {
                        CnrsMessage::Shutdown => {
                            if let Err(e) = wg.clean_wg_up(interface_name.as_str()).await {
                                eprintln!("Error on exit: {}", e);
                            }
                            if let Err(e) = server
                                .send_disconnect_signal(DisconnectReq {
                                    room_name: room_name.to_string(),
                                    pub_key,
                                    username,
                                })
                                .await
                            {
                                eprintln!("Failed to send disconnect message: {}", e);
                            }
                            if let Err(e) = sender.send(CnrsMessage::FinishedDisconnecting) {
                                eprintln!("Failed to semd stopped signal: {}", e);
                            };
                            break;
                        }
                        CnrsMessage::PeerDiscovered(join_req) => {
                            match wg
                                .add_wg_peer(
                                    &interface_name,
                                    &join_req.peer_info.pub_key,
                                    &join_req.peer_info.mapped_addr,
                                    &join_req.peer_info.wg_ip,
                                )
                                .await
                            {
                                Ok(_) => {
                                    display.add_peer(&join_req.peer_info).await;
                                    // println!("{} joined", &join_req.peer_info.username);
                                }
                                Err(err) => {
                                    eprintln!("Error during initialising new connection: {:?}", err)
                                }
                            };
                        }
                        CnrsMessage::PeerDisconnected(disconnect_req) => {
                            match wg
                                .remove_wg_peer(&interface_name, &disconnect_req.pub_key)
                                .await
                            {
                                Ok(_) => {
                                    display.remove_peer(&disconnect_req).await;
                                    // println!("{} disconnected", &disconnect_req.username);
                                }
                                Err(err) => {
                                    eprintln!("Error during removing wg peer: {:?}", err)
                                }
                            };
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    server
        .connect_to_each(sender.clone(), room_name, &wg_ip)
        .await
        .with_context(|| "Error during connection to other peers")?;

    server
        .sub_to_changes(sender.clone(), &wg_ip, room_name)
        .await?;

    Ok(wg_ip)
}

fn get_unused_port() -> u16 {
    let port = rand::thread_rng().gen_range(5000..20000);
    if TcpListener::bind(("localhost", port)).is_ok() {
        return port;
    }
    get_unused_port()
}

pub fn read_str_from_cli(msg: String) -> String {
    println!("{msg}");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

async fn get_mapped_addr(addr: &str, port: u16) -> Result<String, anyhow::Error> {
    let mut client = Client::new(format!("{}:{}", addr, port), None).await?;
    let res = client
        .binding_request(&Config::global().addrs.stun_addr, None)
        .await?;
    let class = res.get_class();
    if class != Class::SuccessResponse {
        anyhow::bail!("Invalid response class: {:?}", class)
    }

    if let Some(addr) = Attribute::get_xor_mapped_address(&res) {
        Ok(addr.to_string())
    } else {
        anyhow::bail!("Didn't got mapped address from stun client")
    }
}
