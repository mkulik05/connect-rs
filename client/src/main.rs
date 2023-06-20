use anyhow::Context;
use chrono::prelude::*;
use nix::unistd::Uid;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::sync::Arc;
use stun_client::*;
// use surge_ping;
use tokio::fs::write;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use wireguard_keys::{self, Privkey};
extern crate redis;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
mod kernel_wg;
mod server;
mod server_trait;
mod wg_trait;
use crate::kernel_wg::KernelWg;
use crate::server::Server;
use crate::server_trait::ServerTrait;
use crate::wg_trait::Wg;

const LOCAL_ADDR: &str = "0.0.0.0";
const STUN_ADDR: &str = "stun.1und1.de:3478";
const WS_ADDR: &str = "wss://server.mishakulik2.workers.dev/";
const REDIS_URL: &str = "rediss://client:08794a557c35bd6449abc35ed8d3128930daa4600a87a768ca79848ee31760c3@balanced-mastiff-35201.upstash.io:35201";

const GHOST_WG_IP: &str = "10.9.0.0";
const GHOST_WG_PUB_KEY: &str = "UvtAsAD2mLgHhSqwTRkwykaNGuh3oiZg5bTwm/zf1Hs=";
const GHOST_WG_ADDRESS: &str = "1.1.1.1:32322";

const INTERFACE_PREFIX: &str = "cnrs-";
const MAX_INTERFACE_ROOM_PART_LEN: usize = 8;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinReq {
    room_name: String,
    peer_info: PeerInfo,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PeerInfo {
    wg_ip: String,
    pub_key: String,
    mapped_addr: String,
    username: String,
}

#[derive(Clone, Debug)]
pub enum CnrsMessage {
    PeerDiscovered(JoinReq),
    Shutdown,
}

#[tokio::main]
async fn main() -> ExitCode {
    if !Uid::effective().is_root() {
        println!("You should run this app with root permissions");
        return ExitCode::from(1);
    }

    let port = get_unused_port();
    println!("Working with port {}\n", port);

    let private_key = Privkey::generate();
    let public_key = private_key.pubkey();
    let key_name = Local::now()
        .format("/tmp/connect-wg-%Y_%m_%d_%H_%M_%S.key")
        .to_string();

    match write(&key_name, private_key.to_base64().as_bytes()).await {
        Ok(_) => {}
        Err(e) => eprintln!("Error saving key: {}", e),
    };

    println!(
        "Public key: {}\nPrivate key: {}\n",
        &public_key.to_base64(),
        &private_key.to_base64()
    );

    let room_name: String = read_str_from_cli(String::from("Input room id: "));
    let username = read_str_from_cli(String::from("\nInput your username: "));

    println!("\nJoining room... GLHF");

    let interface_name = INTERFACE_PREFIX.to_owned() + {
        if room_name.len() <= MAX_INTERFACE_ROOM_PART_LEN {
            &room_name
        } else {
            &room_name[..MAX_INTERFACE_ROOM_PART_LEN]
        }
    };
    let (tx, _) = broadcast::channel(16);
    let wg: Arc<dyn Wg> = Arc::new(KernelWg {});
    let server: Arc<dyn ServerTrait> = Arc::new(Server {});
    match join_room(
        wg.clone(),
        server.clone(),
        tx.clone(),
        &room_name,
        username,
        public_key.to_base64(),
        port,
        key_name,
        &interface_name,
    )
    .await
    {
        Ok(_) => {
            println!("Joined room: {}", room_name);
            let running = Arc::new(AtomicBool::new(true));
            let r = running.clone();

            ctrlc::set_handler(move || {
                r.store(false, Ordering::SeqCst);
            })
            .expect("Error setting Ctrl-C handler");

            println!("Waiting for Ctrl-C...");
            while running.load(Ordering::SeqCst) {}
            if let Err(e) = wg.clean_wg_up(interface_name.as_str()).await {
                eprintln!("Error on exit: {}", e);
            } else {
                println!("Got it! Exiting...");
                if let Err(e) = tx.send(CnrsMessage::Shutdown) {
                    eprintln!("Failed to send shutdown message: {}", e);
                };
                return ExitCode::SUCCESS;
            };
        }
        Err(err) => eprintln!("Joining room failed: {}", err),
    };

    ExitCode::SUCCESS
}

async fn join_room(
    wg: Arc<dyn Wg>,
    server: Arc<dyn ServerTrait>,
    sender: Sender<CnrsMessage>,
    room_name: &String,
    username: String,
    pub_key: String,
    port: u16,
    key_name: String,
    interface_name: &String,
) -> Result<(), anyhow::Error> {
    let mapped_addr = loop {
        println!("Geting mapped address");
        match get_mapped_addr(LOCAL_ADDR, port).await {
            Ok(addr) => break addr,
            Err(e) => eprintln!("{}", e),
        }
    };
    println!("Mapped address: {}\n", mapped_addr);

    let data = PeerInfo {
        wg_ip: "".to_string(),
        pub_key,
        mapped_addr,
        username,
    };
    let data = JoinReq {
        room_name: room_name.clone(),
        peer_info: data,
    };
    let data = serde_json::to_string(&data)?;
    let wg_ip = server.send_peer_info(&data).await?;

    println!("Connect info sent");
    wg.init_wg(&interface_name, port, key_name.as_str(), wg_ip.as_str())
        .await
        .with_context(|| "failed to init wg")?;

    let mut rx2 = sender.subscribe();
    let wg = wg.clone();
    let interface_name = interface_name.clone();
    tokio::spawn(async move {
        while let Ok(data) = rx2.recv().await {
            if let CnrsMessage::PeerDiscovered(join_req) = data {
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
                        println!("{} joined", &join_req.peer_info.username);
                    }
                    Err(err) => {
                        eprintln!("Error during initialising new connection: {:?}", err)
                    }
                };
            }
        }
    });

    server
        .connect_to_each(sender.clone(), room_name, &wg_ip)
        .await
        .with_context(|| "Error during connection to other peers")?;

    server
        .sub_to_changes(sender.clone(), &wg_ip, &room_name)
        .await?;

    Ok(())
}

fn get_unused_port() -> u16 {
    let port = rand::thread_rng().gen_range(5000..20000);
    if let Ok(_) = TcpListener::bind(("localhost", port)) {
        return port;
    }
    return get_unused_port();
}

fn read_str_from_cli(msg: String) -> String {
    println!("{msg}");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

async fn get_mapped_addr(addr: &str, port: u16) -> Result<String, anyhow::Error> {
    let mut client = Client::new(format!("{}:{}", addr, port), None).await?;
    let res = client.binding_request(STUN_ADDR, None).await?;
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
