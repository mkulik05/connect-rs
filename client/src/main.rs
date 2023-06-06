use anyhow;
use chrono::prelude::*;
use nix::unistd::Uid;
use port_scanner::local_port_available;
use serde::{Deserialize, Serialize};
use serde_json;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use stun_client::*;
use surge_ping;
use tokio::fs;
use tokio::sync::oneshot;
use tokio::time::{sleep, Duration};
use websockets::{self, Frame};
use wireguard_keys::{self, Privkey};

const LOCAL_ADDR: &str = "0.0.0.0";
const STUN_ADDR: &str = "stun.1und1.de:3478";
const WS_ADDR: &str = "ws.1und1.de:3478";

#[derive(Serialize, Deserialize, Debug)]

enum WsMessage {
    WgIpMsg(String),
    JoinReqMsg(JoinReq),
}

#[derive(Serialize, Deserialize, Debug)]
struct JoinReq {
    room_id: String,
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
async fn main() -> std::io::Result<()> {
    if !Uid::effective().is_root() {
        panic!("You must run this executable with root permissions");
    }

    let port = match get_unused_port().await {
        Some(port) => port,
        None => panic!("No free ports available"),
    };
    println!("Working with port {}\n", port);

    let mapped_address = loop {
        println!("Geting mapped address");
        match get_mapped_addr(LOCAL_ADDR, port).await {
            Ok(addr) => break addr,
            Err(e) => eprintln!("{}", e),
        }
    };
    println!("Mapped address: {}\n", mapped_address);

    let peer_address = get_remote_address().unwrap();
    println!("Remote address: {}", &peer_address);

    let private_key = Privkey::generate();
    let public_key = private_key.pubkey();
    let key_name = format!(
        "/tmp/connect-wg-{}.key",
        Local::now().format("%Y_%m_%d-%H:%M:%S").to_string()
    );
    fs::write(&key_name, private_key.to_base64().as_bytes()).await?;

    println!(
        "Public key: {}\nPrivate key: {}",
        &public_key.to_base64(),
        &private_key.to_base64()
    );

    let my_wg_ip: String = read_str_from_cli(String::from("Input your wg ip"));
    let remote_wg_ip = read_str_from_cli(String::from("Input remote wg ip"));
    let remote_username = read_str_from_cli(String::from("Input remote username"));
    let remote_pub_key = read_str_from_cli(String::from("Input remote public key"));

    println!("Starting wg server... GLHF");

    match init_wg(
        &remote_username,
        &my_wg_ip,
        port,
        &remote_pub_key,
        &peer_address,
        &remote_wg_ip,
        &key_name,
    )
    .await
    {
        Ok(_) => {
            println!("Setted up wireguard");
            match start_ping(&remote_wg_ip[..], &remote_username).await {
                Ok(_) => {
                    println!("Stop pinging");
                }
                Err(e) => eprintln!("Failed pinging: {}", e),
            };
        }
        Err(e) => eprintln!("Failed with setting up wireguard: {}", e),
    }

    Ok(())
}

async fn join_room(
    id: String,
    username: String,
    mapped_addr: String,
    pub_key: String,
) -> Result<String, anyhow::Error> {
    let mut builder = websockets::WebSocket::builder();
    let ws = builder.connect(WS_ADDR).await.unwrap();
    let data = PeerInfo {
        wg_ip: "".to_string(),
        pub_key,
        mapped_addr,
        username,
    };
    let data = JoinReq {
        room_id: id,
        peer_info: data,
    };
    let data = serde_json::to_string(&data).unwrap();
    let (mut ws_read, mut ws_write) = ws.split();
    ws_write.send_text(data).await.unwrap();

    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let mut sender = Some(sender);
        while let Ok(msg) = ws_read.receive().await {
            match msg {
                Frame::Text { payload, .. } => {
                    let data: WsMessage = serde_json::from_str(&payload[..]).unwrap();
                    match data {
                        WsMessage::WgIpMsg(ip) => {
                            if sender.is_some() {
                                if let Err(_) = sender.unwrap().send(ip) {
                                    println!("the receiver dropped");
                                }
                                sender = None;
                            }
                        }
                        WsMessage::JoinReqMsg(join_req) => if sender.is_none() {},
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

async fn start_ping(remote_wg_ip: &str, remote_username: &str) -> Result<(), anyhow::Error> {
    let payload = [0; 8];
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    println!("Press Ctrl-C to stop");
    while running.load(Ordering::SeqCst) {
        match surge_ping::ping(remote_wg_ip.parse()?, &payload).await {
            Ok((_packet, duration)) => println!("Ping {}: {:.3?}", remote_wg_ip, duration),
            Err(e) => println!("Error while pinging {}: \n{:?}", remote_wg_ip, e),
        };
        sleep(Duration::from_millis(1000)).await;
    }
    run_terminal_command(format!("ip link del {}", &remote_username))
        .await
        .unwrap();
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
) -> Result<(), anyhow::Error> {
    run_terminal_command(format!(
        "ip link add dev {} type wireguard",
        &remote_username
    ))
    .await?;
    run_terminal_command(format!(
        "ip addr add {}/24 dev {}",
        &my_wg_ip, &remote_username
    ))
    .await?;
    run_terminal_command(format!("ip link set mtu 1420 up dev {}", &remote_username)).await?;

    run_terminal_command(format!(
        "wg set {} listen-port {} private-key {}",
        &remote_username, port, key_name
    ))
    .await?;

    run_terminal_command(format!(
        "wg set {} peer {} persistent-keepalive 1 endpoint {} allowed-ips {}",
        &remote_username, &remote_pub_key, &peer_address, &remote_wg_ip
    ))
    .await?;

    run_terminal_command(format!("ip link set up {}", &remote_username)).await?;

    Ok(())
}

async fn run_terminal_command(command: String) -> Result<Vec<u8>, anyhow::Error> {
    let output = Command::new("bash").arg("-c").arg(&command).output()?;
    if !output.status.success() {
        anyhow::bail!("Error during commnd execution: \n{}\n{:?}", command, output)
    }
    Ok(output.stdout)
}

async fn get_unused_port() -> Option<u16> {
    for port in 20000..27000 {
        if local_port_available(port) {
            return Some(port);
        }
    }
    None
}

fn get_remote_address() -> std::io::Result<String> {
    println!("Input remote addr:");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
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
