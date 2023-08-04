use crate::app_backend::CnrsMessage;
use crate::logger::{log, LogLevel};
use crate::toml_conf::Config;
use crate::wg_trait::Wg;
use async_trait::async_trait;
use std::process::Command;

const GHOST_WG_IP: &str = "10.8.0.255";
const GHOST_WG_PUB_KEY: &str = "UvtAsAD2mLgHhSqwTRkwykaNGuh3oiZg5bTwm/zf1Hs=";
const GHOST_WG_ADDRESS: &str = "1.1.1.1:32322";

#[derive(Clone)]
pub struct KernelWg {}

async fn run_terminal_command(command: String) -> Result<(Vec<u8>, Vec<u8>), anyhow::Error> {
    #[cfg(target_os = "linux")]
    let output = Command::new("bash").arg("-c").arg(&command).output();

    #[cfg(target_os = "windows")]
    let output = Command::new("cmd").args(["/C", &command]).output();
    if output.is_err() {
        anyhow::bail!(format!("{:?}", output))
    } else {
        let output = output.unwrap();
        Ok((output.clone().stdout, output.stdout))
    }
}

macro_rules! run_terminal_command_panic {
    ($command:expr) => {{
        let res = run_terminal_command($command.clone()).await?;
        if !res.1.is_empty() {
            anyhow::bail!(
                "Error during command execution: \n{}\n{:?}",
                $command,
                res.1
            );
        }
        res
    }};
}

pub(crate) use run_terminal_command_panic;

#[async_trait]
impl Wg for KernelWg {
    async fn init_wg(
        &self,
        port: u16,
        conf_path: &str,
        my_wg_ip: &str,
        sender: tokio::sync::broadcast::Sender<CnrsMessage>,
    ) -> Result<(), anyhow::Error> {
        KernelWg::configure_wg(port, conf_path, my_wg_ip).await?;
        while let Ok(res) = run_terminal_command("wg".to_string()).await {
            if (res.0.len() != 0) || (res.1.len() != 0) {
                break;
            }
        }

        // Adding windows firewall rules to allow ping requests
        #[cfg(target_os = "windows")]
        {
            run_terminal_command_panic!("powershell -command New-NetFirewallRule -DisplayName 'Allow' -Profile Any -Direction Inbound -Action Allow -Protocol ICMPv4".to_string());
            run_terminal_command_panic!("powershell -command New-NetFirewallRule -DisplayName 'Allow' -Profile Any -Direction Inbound -Action Allow -Protocol ICMPv6".to_string());
        }

        self.clone().start_service(sender);

        self.add_wg_peer(GHOST_WG_PUB_KEY, GHOST_WG_ADDRESS, GHOST_WG_IP)
            .await?;
        Ok(())
    }
    async fn add_wg_peer(
        &self,
        remote_pub_key: &str,
        peer_address: &str,
        remote_wg_ip: &str,
    ) -> Result<(), anyhow::Error> {
        let interface_name = Config::global().interface.interface_name.as_str();
        run_terminal_command_panic!(format!(
            "wg set {} peer {} persistent-keepalive 1 endpoint {} allowed-ips {}",
            interface_name, &remote_pub_key, &peer_address, &remote_wg_ip
        ));
        Ok(())
    }
    async fn remove_wg_peer(&self, remote_pub_key: &str) -> Result<(), anyhow::Error> {
        let interface_name = Config::global().interface.interface_name.as_str();
        run_terminal_command_panic!(format!(
            "wg set {} peer {} remove",
            interface_name, &remote_pub_key
        ));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    async fn clean_wg_up(&self, interface_name: &str) -> Result<(), anyhow::Error> {
        run_terminal_command_panic!(format!("ip link del {}", &interface_name));
        Ok(())
    }
    #[cfg(target_os = "windows")]
    async fn clean_wg_up(&self, interface_name: &str) -> Result<(), anyhow::Error> {
        run_terminal_command_panic!(format!(
            "wireguard /uninstalltunnelservice {}",
            &interface_name
        ));
        Ok(())
    }
}
impl KernelWg {
    #[cfg(target_os = "linux")]
    pub async fn configure_wg(
        port: u16,
        conf_path: &str,
        my_wg_ip: &str,
    ) -> Result<(), anyhow::Error> {
        let interface_name = Config::global().interface.interface_name.as_str();
        run_terminal_command_panic!(format!("ip link add dev {} type wireguard", interface_name));
        run_terminal_command_panic!(format!(
            "wg set {} listen-port {} private-key {}",
            interface_name, port, conf_path
        ));
        run_terminal_command_panic!(format!(
            "ip addr add {}/24 dev {}",
            &my_wg_ip, interface_name
        ));
        run_terminal_command_panic!(format!("ip link set up {}", interface_name));
        Ok(())
    }

    pub fn start_service(self, rx: tokio::sync::broadcast::Sender<CnrsMessage>) {
        let wg = self.clone();
        let interface_name = Config::global().interface.interface_name.clone();
        tokio::spawn(async move {
            while let Ok(data) = rx.subscribe().recv().await {
                log!(LogLevel::Debug, "[wg] got broadcast msg {:?}", data);
                match data {
                    CnrsMessage::Shutdown => {
                        if let Err(e) = wg.clean_wg_up(interface_name.as_str()).await {
                            log!(LogLevel::Error, "[wg] error on exit: {}", e);
                        } else {
                            log!(LogLevel::Debug, "[wg] cleaned up");
                        }
                        break;
                    }
                    CnrsMessage::PeerDiscovered(join_req) => {
                        match wg
                            .add_wg_peer(
                                &join_req.peer_info.pub_key,
                                &join_req.peer_info.mapped_addr,
                                &join_req.peer_info.wg_ip,
                            )
                            .await
                        {
                            Ok(_) => {
                                if let Err(e) =
                                    rx.send(CnrsMessage::PeerConnected(join_req.clone()))
                                {
                                    log!(
                                        LogLevel::Error,
                                        "[wg] error on broadcast msg sending {:?}",
                                        e
                                    );
                                };
                                log!(
                                    LogLevel::Debug,
                                    "[wg] added peer {}",
                                    &join_req.peer_info.username
                                );
                            }
                            Err(err) => {
                                log!(
                                    LogLevel::Error,
                                    "[wg] error during adding new peer: {:?}",
                                    err
                                )
                            }
                        };
                    }
                    CnrsMessage::PeerDisconnected(disconnect_req) => {
                        if let Err(err) = wg.remove_wg_peer(&disconnect_req.pub_key).await {
                            log!(LogLevel::Error, "Error during removing wg peer: {:?}", err)
                        } else {
                            log!(
                                LogLevel::Debug,
                                "[wg] {} disconnected",
                                &disconnect_req.username
                            );
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    #[cfg(target_os = "windows")]
    pub async fn configure_wg(
        _interface_name: &str,
        _port: u16,
        conf_path: &str,
        _my_wg_ip: &str,
    ) -> Result<(), anyhow::Error> {
        use anyhow::Context;

        let mut conf_dir = std::env::current_dir().with_context(|| "Failed to get current dir")?;
        conf_dir.push(conf_path);
        run_terminal_command_panic!(format!(
            "wireguard /installtunnelservice {}",
            &conf_dir.display()
        ));
        // run_terminal_command_panic!(format!("wg setconf {}", &conf_path));
        // run_terminal_command_panic!(format!("ip link set up {}", &interface_name));
        Ok(())
    }
}
