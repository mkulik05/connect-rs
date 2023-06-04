use anyhow;
use std::process::{Command};
use stun_client::*;
use port_scanner::local_port_available;

const LOCAL_ADDR: &str = "0.0.0.0";
const STUN_ADDR: &str = "stun.1und1.de:3478";

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let port = match get_unused_port() {
        Some(port) => port,
        None => panic!("No free ports available")
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


    let private_key_gen = Command::new("wg")
        .arg("genkey")
        .output()
        .unwrap();
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
            &remote_username, port, "/tmp/connect-wg-key.key"
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

fn get_unused_port() -> Option<u16> {
    for port in 20000..27000 {
        if local_port_available(port) {
            return Some(port)
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
