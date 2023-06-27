mod app_backend;
mod kernel_wg;
mod server;
mod server_trait;
mod toml_conf;
mod wg_trait;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    app_backend::start_backend().await?;
    Ok(())
}
