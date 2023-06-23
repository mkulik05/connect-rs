use async_trait::async_trait;
use tokio::sync::broadcast::Sender;
use crate::{JoinReq, DisconnectReq, UpdateTimerReq};
use crate::CnrsMessage;
#[async_trait]
pub trait ServerTrait: Send + Sync {
    async fn send_peer_info(&self, data: JoinReq) -> Result<String, anyhow::Error>;
    async fn sub_to_changes(
        &self,
        sender: Sender<CnrsMessage>,
        wg_ip: &String,
        room_name: &String,
    ) -> Result<(), anyhow::Error>;
    async fn connect_to_each(&self, sender: Sender<CnrsMessage>, room_name: &String, wg_ip: &String) -> Result<(), anyhow::Error>; 
    async fn send_disconnect_signal(&self, data: DisconnectReq) -> Result<(), anyhow::Error>;
    async fn update_connection_time(&self, data: UpdateTimerReq) -> Result<(), anyhow::Error>;
}
