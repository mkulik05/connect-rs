use async_trait::async_trait;

#[async_trait]
pub trait Wg: Send + Sync {
    async fn init_wg(
        &self,
        interface_name: &str,
        port: u16,
        key_path: &str,
        my_wg_ip: &str,
    ) -> Result<(), anyhow::Error>;
    async fn add_wg_peer(
        &self,
        interface_name: &str,
        remote_pub_key: &str,
        peer_address: &str,
        remote_wg_ip: &str,
    ) -> Result<(), anyhow::Error>;
    async fn clean_wg_up(&self, interface_name: &str) -> Result<(), anyhow::Error>;
}
