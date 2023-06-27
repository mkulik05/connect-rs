use crate::wg_trait::Wg;
use async_trait::async_trait;
use std::process::Command;

const GHOST_WG_IP: &str = "10.8.0.255";
const GHOST_WG_PUB_KEY: &str = "UvtAsAD2mLgHhSqwTRkwykaNGuh3oiZg5bTwm/zf1Hs=";
const GHOST_WG_ADDRESS: &str = "1.1.1.1:32322";

#[derive(Clone)]
pub struct KernelWg {}

impl KernelWg {
    pub async fn interface_exists(interface_name: &str) -> Result<bool, anyhow::Error> {
        let res =
            run_terminal_command(format!("ip link show {} >/dev/null", &interface_name), true)
                .await?;
        Ok(res.is_empty())
    }
    pub async fn create_wg_interface(interface_name: &str) -> Result<(), anyhow::Error> {
        run_terminal_command(
            format!("ip link add dev {} type wireguard", &interface_name),
            false,
        )
        .await?;
        Ok(())
    }

    pub async fn configure_wg(
        interface_name: &str,
        port: u16,
        key_name: &str,
    ) -> Result<(), anyhow::Error> {
        run_terminal_command(
            format!(
                "wg set {} listen-port {} private-key {}",
                &interface_name, port, key_name
            ),
            false,
        )
        .await?;
        Ok(())
    }

    pub async fn add_interface_ip(
        interface_name: &str,
        my_wg_ip: &str,
    ) -> Result<(), anyhow::Error> {
        run_terminal_command(
            format!("ip addr add {}/24 dev {}", &my_wg_ip, &interface_name),
            false,
        )
        .await?;
        Ok(())
    }
    pub async fn start_interface(interface_name: &str) -> Result<(), anyhow::Error> {
        run_terminal_command(format!("ip link set up {}", &interface_name), false).await?;
        Ok(())
    }
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

#[async_trait]
impl Wg for KernelWg {
    async fn init_wg(
        &self,
        interface_name: &str,
        port: u16,
        key_path: &str,
        my_wg_ip: &str,
    ) -> Result<(), anyhow::Error> {
        if !KernelWg::interface_exists(&interface_name).await? {
            KernelWg::create_wg_interface(interface_name).await?;
        }
        KernelWg::configure_wg(interface_name, port, key_path).await?;
        KernelWg::add_interface_ip(interface_name, my_wg_ip).await?;
        self.add_wg_peer(
            interface_name,
            GHOST_WG_PUB_KEY,
            GHOST_WG_ADDRESS,
            GHOST_WG_IP,
        )
        .await?;
        KernelWg::start_interface(&interface_name).await?;
        Ok(())
    }
    async fn add_wg_peer(
        &self,
        interface_name: &str,
        remote_pub_key: &str,
        peer_address: &str,
        remote_wg_ip: &str,
    ) -> Result<(), anyhow::Error> {
        run_terminal_command(
            format!(
                "wg set {} peer {} persistent-keepalive 1 endpoint {} allowed-ips {}",
                &interface_name, &remote_pub_key, &peer_address, &remote_wg_ip
            ),
            false,
        )
        .await?;
        Ok(())
    }
    async fn remove_wg_peer(
        &self,
        interface_name: &str,
        remote_pub_key: &str,
    ) -> Result<(), anyhow::Error> {
        run_terminal_command(
            format!(
                "wg set {} peer {} remove",
                &interface_name, &remote_pub_key
            ),
            false,
        )
        .await?;
        Ok(())
    }
    async fn clean_wg_up(&self, interface_name: &str) -> Result<(), anyhow::Error> {
        run_terminal_command(format!("ip link del {}", &interface_name), false).await?;
        Ok(())
    }
}
