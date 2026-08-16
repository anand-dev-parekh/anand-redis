use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::commands::{self, Db};
use crate::resp;

pub async fn run() -> std::io::Result<()> {
    // Loading before the bind means no client can ever observe a half-filled store.
    // A damaged dump costs us the data, not the server.
    let store = match commands::persistence::load().await {
        Ok(store) => store,
        Err(err) => {
            eprintln!("Failed to load dump.rdb: {err} — starting with an empty database");
            HashMap::new()
        }
    };

    let db: Db = Arc::new(RwLock::new(store));
    let listener = TcpListener::bind("127.0.0.1:6379").await?;
    println!("Listening on 127.0.0.1:6379");

    let expiry_db = Arc::clone(&db);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let now = Instant::now();
            let mut store = expiry_db.write().unwrap();
            store.retain(|_, entry| entry.expires_at.map_or(true, |e| e > now));
        }
    });

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
