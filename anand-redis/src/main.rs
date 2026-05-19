mod commands;
mod resp;
mod server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    server::run().await
}
