use anyhow;
use chrono::prelude::*;
use nix::unistd::Uid;
use rand::Rng;
use redis::Commands;
use serde::{Deserialize, Serialize};
use serde_json;
use std::net::TcpListener;
use std::process::Command;
// use std::sync::Arc;
use stun_client::*;
// use surge_ping;
use tokio::fs;
use tokio::sync::oneshot;
// use tokio::time::{sleep, Duration};
use websockets::{self, Frame};
use wireguard_keys::{self, Privkey};

extern crate redis;
use std::process::ExitCode;

const LOCAL_ADDR: &str = "0.0.0.0";
const STUN_ADDR: &str = "stun.1und1.de:3478";
const WS_ADDR: &str = "wss://server.mishakulik2.workers.dev/";
const REDIS_URL: &str = "rediss://client:08794a557c35bd6449abc35ed8d3128930daa4600a87a768ca79848ee31760c3@balanced-mastiff-35201.upstash.io:35201";

const GHOST_WG_IP: &str = "10.9.0.0";
const GHOST_WG_PUB_KEY: &str = "UvtAsAD2mLgHhSqwTRkwykaNGuh3oiZg5bTwm/zf1Hs=";
const GHOST_WG_ADDRESS: &str = "1.1.1.1:32322";

const INTERFACE_PREFIX: &str = "cnrs-";
const MAX_INTERFACE_ROOM_PART_LEN: usize = 8;

#[derive(Serialize, Deserialize, Debug)]

enum WsMessage {
    WgIpMsg(String),
    JoinReqMsg(JoinReq),
}

#[derive(Serialize, Deserialize, Debug)]
struct JoinReq {
    room_name: String,
    peer_info: PeerInfo,
}

#[derive(Serialize, Deserialize, Debug)]
struct PeerInfo {
    wg_ip: String,
    pub_key: String,
    mapped_addr: String,
    username: String,
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

    match fs::write(&key_name, private_key.to_base64().as_bytes()).await {
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

    match join_room(
        &room_name,
        username,
        public_key.to_base64(),
        port,
        key_name,
        &(INTERFACE_PREFIX.to_owned() + {
            if room_name.len() <= MAX_INTERFACE_ROOM_PART_LEN {
                &room_name
            } else {
                &room_name[..MAX_INTERFACE_ROOM_PART_LEN]
            }
        }),
    )
    .await
    {
        Ok(_) => println!("Joined room: {}", room_name),
        Err(err) => eprintln!("Joining room failed: {}", err),
    };
    loop {}
}

async fn join_room(
    room_name: &String,
    username: String,
    pub_key: String,
    port: u16,
    key_name: String,
    interface_name: &String,
) -> Result<String, anyhow::Error> {
    let mapped_addr = loop {
        println!("Geting mapped address");
        match get_mapped_addr(LOCAL_ADDR, port).await {
            Ok(addr) => break addr,
            Err(e) => eprintln!("{}", e),
        }
    };
    println!("Mapped address: {}\n", mapped_addr);
    let ws = websockets::WebSocket::connect(WS_ADDR).await?;
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
    let data = serde_json::to_string(&data).unwrap();
    let (mut ws_read, mut ws_write) = ws.split();

    ws_write.send_text(data).await.unwrap();
    println!("Connect info sent");

    let (sender, receiver) = oneshot::channel();
    let interface_name = interface_name.clone();
    let room_name = room_name.clone();
    tokio::spawn(async move {
        let mut sender = Some(sender);
        let mut wg_ip = None;
        while let Ok(msg) = ws_read.receive().await {
            match msg {
                Frame::Text { payload, .. } => {
                    let data: WsMessage = serde_json::from_str(payload.as_str()).unwrap();
                    if let WsMessage::WgIpMsg(ip) = data {
                        if sender.is_some() {
                            if let Err(_) = sender.unwrap().send(ip.clone()) {
                                println!("the receiver dropped");
                            } else {
                                println!("Got socket msg with wg ip");
                                wg_ip = Some(ip);
                                match wg_connect_to_each(
                                    &wg_ip.clone().unwrap(),
                                    &room_name,
                                    port,
                                    &key_name,
                                    &interface_name,
                                )
                                .await
                                {
                                    Ok(_) => {
                                        sub_to_room(
                                            wg_ip.clone().unwrap(),
                                            room_name.clone(),
                                            port,
                                            key_name.clone(),
                                            &interface_name,
                                        )
                                        .await;
                                    }
                                    Err(e) => {
                                        eprintln!("Error during connection to other peers: {}", e)
                                    }
                                };
                            }
                            sender = None;
                        }
                    }
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

async fn sub_to_room(
    wg_ip: String,
    room_name: String,
    port: u16,
    key_name: String,
    interface_name: &String,
) {
    {
        let room_name = room_name.clone();
        let interface_name = interface_name.clone();
        tokio::spawn(async move {
            let client = redis::Client::open(REDIS_URL).unwrap();
            let mut con = client.get_connection().unwrap();
            let mut pubsub = con.as_pubsub();
            pubsub.subscribe(room_name.as_str()).unwrap();

            loop {
                let msg = pubsub.get_message().unwrap();
                let payload: String = msg.get_payload().unwrap();
                if msg.get_channel_name() == room_name {
                    let data: WsMessage = serde_json::from_str(payload.as_str()).unwrap();
                    if let WsMessage::JoinReqMsg(join_req) = data {
                        println!("{} is trying to join", &join_req.peer_info.username);
                        if wg_ip != join_req.peer_info.wg_ip {
                            match init_wg(
                                &join_req.peer_info.username,
                                &wg_ip,
                                port,
                                &join_req.peer_info.pub_key,
                                &join_req.peer_info.mapped_addr,
                                &join_req.peer_info.wg_ip,
                                &key_name,
                                &interface_name,
                            )
                            .await
                            {
                                Ok(_) => {
                                    println!("{} joined", &join_req.peer_info.username);
                                }
                                Err(err) => {
                                    eprintln!("Error during initialising new connection: {}", err)
                                }
                            };
                        }
                    }
                }
            }
        });
    }
}

async fn wg_connect_to_each(
    wg_ip: &String,
    room_name: &String,
    port: u16,
    key_name: &String,
    interface_name: &String,
) -> Result<(), anyhow::Error> {
    let client = redis::Client::open(REDIS_URL).unwrap();
    let mut con = client.get_connection().unwrap();
    let peers_n: u16 = con.llen(&room_name)?;
    for i in 0..peers_n {
        let data: String = con.lindex(&room_name, i as isize)?;
        let peer_info: PeerInfo = serde_json::from_str(&data[..]).unwrap();
        if peer_info.wg_ip != *wg_ip {
            init_wg(
                peer_info.username.as_str(),
                &wg_ip,
                port,
                peer_info.pub_key.as_str(),
                peer_info.mapped_addr.as_str(),
                peer_info.wg_ip.as_str(),
                &key_name.as_str(),
                interface_name,
            )
            .await?;
        }
    }
    Ok(())
}

async fn init_wg(
    remote_username: &str,
    my_wg_ip: &str,
    port: u16,
    remote_pub_key: &str,
    peer_address: &str,
    remote_wg_ip: &str,
    key_name: &str,
    interface_name: &String,
) -> Result<(), anyhow::Error> {
    let res =
        run_terminal_command(format!("ip link show {} >/dev/null", &interface_name), true).await?;
    if res.len() != 0 {
        run_terminal_command(
            format!("ip link add dev {} type wireguard", &interface_name),
            false,
        )
        .await?;
        run_terminal_command(
            format!("ip addr add {}/24 dev {}", &my_wg_ip, &interface_name),
            false,
        )
        .await?;
        run_terminal_command(
            format!("ip link set mtu 1420 up dev {}", &interface_name),
            false,
        )
        .await?;

        run_terminal_command(
            format!(
                "wg set {} listen-port {} private-key {}",
                &interface_name, port, key_name
            ),
            false,
        )
        .await?;

        run_terminal_command(
            format!(
                "wg set {} peer {} persistent-keepalive 1 endpoint {} allowed-ips {}",
                &interface_name, &GHOST_WG_PUB_KEY, &GHOST_WG_ADDRESS, &GHOST_WG_IP
            ),
            false,
        )
        .await?;
    }

    run_terminal_command(
        format!(
            "wg set {} peer {} persistent-keepalive 5 endpoint {} allowed-ips {}",
            &interface_name, &remote_pub_key, &peer_address, &remote_wg_ip
        ),
        false,
    )
    .await?;

    run_terminal_command(format!("ip link set up {}", &interface_name), false).await?;

    Ok(())
}

async fn run_terminal_command(
    command: String,
    allow_error: bool,
) -> Result<Vec<u8>, anyhow::Error> {
    let output = Command::new("bash").arg("-c").arg(&command).output()?;
    if !output.status.success() {
        if allow_error {
            return Ok(output.stderr);
        }
        anyhow::bail!(
            "Error during command execution: \n{}\n{:?}",
            command,
            output
        )
    }
    Ok(output.stdout)
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
