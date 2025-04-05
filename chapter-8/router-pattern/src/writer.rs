use serde_json;
use std::collections::HashMap;
use tokio::fs::File;
use tokio::io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{
    mpsc::channel,
    mpsc::{Receiver, Sender},
    oneshot,
};
use tokio::time::{self, Duration, Instant};

use crate::{ActorType, KeyValueMessage, RoutingMessage, ROUTER_SENDER};

pub enum WriterLogMessage {
    Set(String, Vec<u8>),
    Delete(String),
    Get(oneshot::Sender<HashMap<String, Vec<u8>>>),
}

impl WriterLogMessage {
    pub fn from_key_value_message(message: &KeyValueMessage) -> Option<WriterLogMessage> {
        match message {
            KeyValueMessage::Get(_) => None,
            KeyValueMessage::Delete(message) => Some(WriterLogMessage::Delete(message.key.clone())),
            KeyValueMessage::Set(message) => Some(WriterLogMessage::Set(
                message.key.clone(),
                message.value.clone(),
            )),
        }
    }
}

pub async fn read_data_from_file(file_path: &str) -> io::Result<HashMap<String, Vec<u8>>> {
    let mut file = File::open(file_path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    let data: HashMap<String, Vec<u8>> = serde_json::from_str(&contents)?;
    Ok(data)
}

async fn load_map(file_path: &str) -> HashMap<String, Vec<u8>> {
    match read_data_from_file(file_path).await {
        Ok(data) => {
            println!("Data loaded from file: {:?}", data);
            return data;
        }
        Err(e) => {
            println!("Failed to read from file: {:?}", e);
            println!("Starting with an empty hashmap.");
            return HashMap::new();
        }
    }
}

pub async fn writer_actor(mut receiver: Receiver<WriterLogMessage>) -> io::Result<()> {
    let mut map = load_map("./data.json").await;
    let mut file = File::create("./data.json").await.unwrap();

    let timeout_duration = Duration::from_millis(200);
    let router_sender = ROUTER_SENDER.get().unwrap().clone();
    loop {
        match time::timeout(timeout_duration, receiver.recv()).await {
            Ok(Some(message)) => {
                match message {
                    WriterLogMessage::Set(key, value) => {
                        map.insert(key, value);
                    }
                    WriterLogMessage::Delete(key) => {
                        map.remove(&key);
                    }
                    WriterLogMessage::Get(response) => {
                        let _ = response.send(map.clone());
                    }
                }
                let contents = serde_json::to_string(&map).unwrap();
                file.set_len(0).await?;
                file.seek(std::io::SeekFrom::Start(0)).await?;
                file.write_all(contents.as_bytes()).await?;
                file.flush().await?;
            },
            Ok(None) => break,
            Err(_) => router_sender.send(RoutingMessage::Heartbeat(ActorType::Writer)).await.unwrap(),
        }
    }
    Ok(())
}
