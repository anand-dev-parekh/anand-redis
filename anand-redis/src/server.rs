use tokio::net::{TcpListener, TcpStream};

pub async fn run() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:6379").await?;
    println!("Listening on 127.0.0.1:6379");

    loop {
        let (stream, addr) = listener.accept().await?;
        println!("Client connected: {addr}");

        tokio::spawn(async move {
            handle_client(stream, addr).await;
        });
    }
}

async fn handle_client(stream: TcpStream, addr: std::net::SocketAddr) {
    drop(stream);
    println!("Client disconnected: {addr}");
}
