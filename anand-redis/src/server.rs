use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::commands::{self, Db};
use crate::resp;

pub async fn run() -> std::io::Result<()> {
    let db: Db = Arc::new(RwLock::new(HashMap::new()));
    let listener = TcpListener::bind("127.0.0.1:6379").await?;
    println!("Listening on 127.0.0.1:6379");

    loop {
        let (stream, addr) = listener.accept().await?;
        println!("Client connected: {addr}");

        let db = Arc::clone(&db);
        tokio::spawn(async move {
            handle_client(stream, addr, db).await;
        });
    }
}

async fn handle_client(stream: TcpStream, addr: std::net::SocketAddr, db: Db) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    loop {
        let value = match resp::parse(&mut reader).await {
            Ok(v) => v,
            Err(_) => break,
        };

        let response = commands::dispatch(value, &db);
        let bytes = resp::serialize(&response);

        if write_half.write_all(&bytes).await.is_err() {
            break;
        }
    }

    println!("Client disconnected: {addr}");
}
