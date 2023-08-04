use async_trait::async_trait;
use crate::app_backend::CnrsMessage;
#[async_trait]
pub trait Wg: Send + Sync {
    // Create wireguard for work (create interface, configure)
    async fn init_wg(
        &self,
        port: u16,
        conf_path: &str,
        my_wg_ip: &str,
        sender: tokio::sync::broadcast::Sender<CnrsMessage>
    ) -> Result<(), anyhow::Error>;

    // if peers with this public key existed - update it's info (not add new)
    async fn add_wg_peer(
        &self,
        remote_pub_key: &str,
        peer_address: &str,
        remote_wg_ip: &str,
    ) -> Result<(), anyhow::Error>;
    async fn remove_wg_peer(
        &self,
        remote_pub_key: &str,
    ) -> Result<(), anyhow::Error>;

    // Remove wireguard interface
    // May be called in the started if previous time there were no clean up
    // So interface remained 
    async fn clean_wg_up(&self, interface_name: &str) -> Result<(), anyhow::Error>;
}
