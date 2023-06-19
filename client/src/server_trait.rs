use async_trait::async_trait;
use tokio::sync::broadcast::Sender;
use crate::CnrsMessage;
#[async_trait]
pub trait ServerTrait: Send + Sync {
    async fn send_peer_info(&self, data: &String) -> Result<String, anyhow::Error>;
    async fn sub_to_changes(
        &self,
        sender: Sender<CnrsMessage>,
        wg_ip: &String,
        room_name: &String,
    ) -> Result<(), anyhow::Error>;
}
