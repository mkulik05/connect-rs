use crate::thread::sleep;
use anyhow;
use std::env::remove_var;
use std::net::UdpSocket;
use std::process::{Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc};
use std::time::Duration;
use std::{thread};
use stun_client::*;

const PORT: i32 = 38381;
const ADDR: &str = "0.0.0.0:38381";
const STUN_ADDR: &str = "stun.1und1.de:3478";

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mapped_address = loop {
        println!("Geting mapped address");
        match get_mapped_addr(STUN_ADDR).await {
            Ok(addr) => break addr,
            Err(e) => eprintln!("{}", e),
        }
    };
    println!("Mapped address: {}", mapped_address);

    let peer_address = get_remote_address().unwrap();
    println!("Remote address: {}", &peer_address);


    let private_key_gen = Command::new("wg")
        .arg("genkey")
        .output()
        .unwrap();
    // let private_key = private_key_gen.stdout;
    let private_key = String::from_utf8(private_key_gen.stdout)
        .unwrap()
        .trim()
        .to_string();
    let public_key_gen = Command::new("bash")
        .arg("-c")
        .arg(format!("echo {} | wg pubkey", &private_key))
        .output()
        .unwrap();
    let public_key = String::from_utf8(public_key_gen.stdout)
        .unwrap()
        .trim()
        .to_string();

    println!("Public key: {}\nPrivate key: {}", &public_key, &private_key);

    let my_wg_ip = read_str_from_cli(String::from("Input your wg ip"));
    let remote_wg_ip = read_str_from_cli(String::from("Input remote wg ip"));
    let remote_username = read_str_from_cli(String::from("Input remote username"));
    let remote_pub_key = read_str_from_cli(String::from("Input remote public key"));

    println!("Starting wg server... GLHF");

    Command::new("bash")
        .arg("-c")
        .arg(format!(
            "ip link add dev {} type wireguard",
            &remote_username
        ))
        .output()
        .unwrap();

    Command::new("bash")
        .arg("-c")
        .arg(format!("ip addr add {}/24 dev {}", &my_wg_ip, &remote_username))
        .output()
        .unwrap();

    Command::new("bash")
        .arg("-c")
        .arg(format!("ip link set mtu 1420 up dev {}", &remote_username))
        .output()
        .unwrap();

    Command::new("bash")
        .arg("-c")
        .arg(format!("echo {} > {}", &private_key, "/tmp/connect-wg-key.key"))
        .output()
        .unwrap();

    Command::new("bash")
        .arg("-c")
        .arg(format!(
            "wg set {} listen-port {} private-key {}",
            &remote_username, PORT, "/tmp/connect-wg-key.key"
        ))
        .output()
        .unwrap();

    Command::new("bash")
        .arg("-c")
        .arg(format!("wg set {} peer {} persistent-keepalive 0.5 endpoint {} allowed-ips {}", &remote_username, &remote_pub_key, &peer_address, &remote_wg_ip))
        .output()
        .unwrap();

    Command::new("bash")
        .arg("-c")
        .arg(format!("ip link set up {}", &remote_username))
        .output()
        .unwrap();

    // println!("Finished???");
    // test_connection(socket, &peer_address);

    // println!("{}", mapped_address);

    Ok(())
}

fn test_connection(socket: UdpSocket, addr: &String) {
    let socket2 = socket.try_clone().expect("Can't clone udp socket");
    let buf = [0; 1];
    let str_addr = (*addr).clone();
    thread::spawn(move || {
        let mut buf = [0; 256];
        loop {
            let (amnt, src) = socket2.recv_from(&mut buf).unwrap();
            if src.to_string() == str_addr {
                let data_bytes = &buf[..amnt];
                println!("{}", std::str::from_utf8(data_bytes).expect("Bad msg"));
            }
        }
    });
    loop {
        println!("> ");
        let mut buf = String::new();
        std::io::stdin().read_line(&mut buf).unwrap();
        socket.send_to(&buf.into_bytes(), addr).unwrap();
        println!("Sent");
    }
}

async fn punch(socket: UdpSocket, addr: &String) -> std::io::Result<()> {
    let socket2 = socket.try_clone().expect("Can't clone udp socket");
    let was_punched = Arc::new(AtomicBool::new(false));
    let mut buf = [0; 1];
    let str_addr = (*addr).clone();
    {
        let was_punched = was_punched.clone();
        tokio::spawn(async move {
            loop {
                let (_, src) = socket2.recv_from(&mut buf).unwrap();
                if src.to_string() == str_addr {
                    was_punched.store(true, Ordering::SeqCst);
                    break;
                }
            }
        })
    };
    loop {
        socket.send_to(&buf, addr).unwrap();
        sleep(Duration::from_millis(200));
        if was_punched.load(Ordering::SeqCst) {
            break;
        }
    }
    Ok(())
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

async fn get_mapped_addr(stun_addr: &str) -> Result<String, anyhow::Error> {
    let mut client = Client::new(ADDR, None).await?;
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
