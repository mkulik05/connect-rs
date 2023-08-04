use anyhow::Context;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use std::sync::Arc;
use stunclient::StunClient;
use tokio::fs::write;
use tokio::sync::broadcast::Sender;
use wireguard_keys::{self, Privkey};
extern crate redis;
use crate::kernel_wg::KernelWg;
use crate::logger::{log, LogLevel};
use crate::server::Server;
use crate::server_trait::ServerTrait;
use crate::toml_conf::Config;
use crate::wg_trait::Wg;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinReq {
    pub room_name: String,
    pub peer_info: PeerInfo,
    pub is_reconnecting: bool,
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
    // Aditionally is used to connect to each peer after joining room
    // Can be received from server
    PeerDiscovered(JoinReq),

    // Peer was connected to pc (from wg side)
    PeerConnected(JoinReq),

    // Can be received from server
    PeerDisconnected(DisconnectReq),

    // Stop handling messages
    Shutdown,

    // Used to confirm that cleaned up before shutdown (sended disconnect signal)
    FinishedDisconnecting,
}

pub async fn start_backend(
    sender: Sender<CnrsMessage>,
    my_wg_ip: &mut String,
) -> Result<(), anyhow::Error> {
    let priv_key = Privkey::from_base64(Config::global().room.priv_key.as_str())?;
    let interface_name = Config::global().interface.interface_name.clone();

    let port = get_unused_port();
    log!(LogLevel::Debug, "Working with port {}", port);

    let public_key = priv_key.pubkey().to_base64();
    let conf_path = format!("{}.conf", interface_name.as_str());

    // For linux just putting key into config
    // Additional configuration is done using linux abilities
    #[cfg(target_os = "linux")]
    if let Err(e) = write(&conf_path, priv_key.to_base64().as_bytes()).await {
        log!(LogLevel::Error, "Error saving key: {}", e);
    };
    log!(LogLevel::Debug, "Public key: {}", &public_key);

    let wg: Arc<dyn Wg> = Arc::new(KernelWg {});
    let server: Arc<dyn ServerTrait> = Arc::new(Server {});

    Server::run(sender.clone(), public_key.clone());

    // Getting our wireguard ip
    let wg_ip = send_join_req(
        server.clone(),
        &public_key,
        port,
    )
    .await?;
    *my_wg_ip = wg_ip.clone();

    // For windows creating whole wireguard interface config
    #[cfg(target_os = "windows")]
    if let Err(e) = write(
        &conf_path,
        format!(
            "[Interface]\nPrivateKey = {}\nListenPort = {}\nAddress = {}/24",
            priv_key.to_base64(),
            port,
            wg_ip.clone()
        )
        .as_bytes(),
    )
    .await
    {
        log!(LogLevel::Error, "Error saving wg config: {}", e);
    };

    if let Err(e) = join_room(
        ConnectConfig {
            wg: wg.clone(),
            server: server.clone(),
            cnrs_sender: sender.clone(),
        },
        public_key,
        port,
        conf_path,
        wg_ip,
    )
    .await
    {
        anyhow::bail!("Joining room failed: {}", e);
    };
    Ok(())
}

async fn send_join_req(
    server: Arc<dyn ServerTrait>,
    pub_key: &String,
    port: u16,
) -> Result<String, anyhow::Error> {
    let mapped_addr = loop {
        match get_mapped_addr("0.0.0.0", port).await {
            Ok(addr) => break addr,
            Err(e) => log!(LogLevel::Error, "{}", e),
        }
    };
    log!(LogLevel::Debug, "Mapped address: {}", mapped_addr);

    let data = PeerInfo {
        wg_ip: "".to_string(),
        last_connected: "".to_string(),
        pub_key: pub_key.to_owned(),
        is_online: true,
        username: Config::global().room.username.to_owned(),
        mapped_addr,
    };
    let data = JoinReq {
        room_name: Config::global().room.room_name.clone().to_owned(),
        peer_info: data,
        is_reconnecting: false,
    };
    let wg_ip = server.send_peer_info(data).await?;
    log!(LogLevel::Debug, "Connect info sent");
    Ok(wg_ip)
}

struct ConnectConfig {
    wg: Arc<dyn Wg>,
    server: Arc<dyn ServerTrait>,
    cnrs_sender: Sender<CnrsMessage>,
}

async fn join_room(
    connect_conf: ConnectConfig,
    pub_key: String,
    port: u16,
    key_name: String,
    wg_ip: String,
) -> Result<(), anyhow::Error> {
    {
        let server = connect_conf.server.clone();
        let room_name = Config::global().room.room_name.clone();
        let msg = UpdateTimerReq { room_name, pub_key };
        tokio::spawn(async move {
            loop {
                if let Err(e) = server.update_connection_time(msg.clone()).await {
                    log!(
                        LogLevel::Error,
                        "Error updating last connection time: {}",
                        e
                    );
                };
                log!(
                    LogLevel::Debug,
                    "Sended request to update last connection time"
                );
                tokio::time::sleep(std::time::Duration::from_secs(3600 * 6)).await;
            }
        });
    }

    connect_conf
        .wg
        .init_wg(port, key_name.as_str(), wg_ip.as_str(), connect_conf.cnrs_sender.clone())
        .await
        .with_context(|| "failed to init wg")?;

    log!(LogLevel::Debug, "Inited wg");

    // let mut rx2 = connect_conf.cnrs_sender.subscribe();
    // let wg = connect_conf.wg.clone();
    // let interface_name = connect_conf.interface_name.clone();
    // let pub_key = connect_conf.pub_key.clone();
    // {
    //     let sender = connect_conf.cnrs_sender.clone();
    //     let server = connect_conf.server.clone();
    //     let room_name = room_name.clone();

        
    // }

    let room_name = Config::global().room.room_name.clone();
    connect_conf
        .server
        .connect_to_each(connect_conf.cnrs_sender.clone(), &room_name, &wg_ip)
        .await
        .with_context(|| "Error during connection to other peers")?;

    connect_conf
        .server
        .sub_to_changes(connect_conf.cnrs_sender.clone(), &wg_ip, &room_name)
        .await?;
    Ok(())
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
    let local_addr: SocketAddr = format!("{}:{}", addr, port).parse()?;
    let stun_addr = Config::global()
        .addrs
        .stun_addr
        .to_socket_addrs()?
        .filter(|x| x.is_ipv4())
        .next()
        .unwrap();
    let udp = tokio::net::UdpSocket::bind(&local_addr).await?;

    let c = StunClient::new(stun_addr);
    let f = c.query_external_address_async(&udp);
    let my_external_addr = f.await?;
    Ok(my_external_addr.to_string())
}
