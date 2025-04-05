use std::sync::OnceLock;
use tokio::sync::{
    mpsc::channel,
    mpsc::{Receiver, Sender},
    oneshot,
};
use tokio::time::{self, Duration, Instant};
use crate::writer::WriterLogMessage;
use std::collections::HashMap;

mod writer;

pub static ROUTER_SENDER: OnceLock<Sender<RoutingMessage>> = OnceLock::new();

struct SetKeyValueMessage {
    key: String,
    value: Vec<u8>,
    response: oneshot::Sender<()>,
}

struct GetKeyValueMessage {
    key: String,
    response: oneshot::Sender<Option<Vec<u8>>>,
}

struct DeleteKeyValueMessage {
    key: String,
    response: oneshot::Sender<()>,
}

enum KeyValueMessage {
    Get(GetKeyValueMessage),
    Delete(DeleteKeyValueMessage),
    Set(SetKeyValueMessage),
}

enum RoutingMessage {
    KeyValue(KeyValueMessage),
    Heartbeat(ActorType),
    Reset(ActorType),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum ActorType {
    KeyValue,
    Writer
}

async fn key_value_actor(mut receiver: Receiver<KeyValueMessage>) {
    let (writer_key_value_sender, writer_key_value_receiver) = channel(32);
    let _writer_handle = tokio::spawn(writer::writer_actor(writer_key_value_receiver));

    let timeout_duration = Duration::from_millis(200);
    let router_sender = ROUTER_SENDER.get().unwrap().clone();

    let mut map = std::collections::HashMap::new();
    loop {
        match time::timeout(timeout_duration, receiver.recv()).await {
            Ok(Some(message)) => {
                if let Some(write_message) = WriterLogMessage::from_key_value_message(&message) {
                    let _ = writer_key_value_sender.send(
                        write_message
                    ).await;
                }
                match message {
                    KeyValueMessage::Get(GetKeyValueMessage { key, response }) => {
                        // we could also get from writer
                        /*
                        let (get_sender, get_receiver) = oneshot::channel();
                        let _ = writer_key_value_sender.send(WriterLogMessage::Get(
                            get_sender
                        )).await;
                        let mut map = get_receiver.await.unwrap();
                        */
                        let _ = response.send(map.get(&key).cloned());
                    }
                    KeyValueMessage::Delete(DeleteKeyValueMessage { key, response }) => {
                        map.remove(&key);
                        let _ = response.send(());
                    }
                    KeyValueMessage::Set(SetKeyValueMessage { key, value, response, }) => {
                        map.insert(key, value);
                        let _ = response.send(());
                    }
                }
            },
            Ok(None) => break,
            Err(_) => router_sender.send(RoutingMessage::Heartbeat(ActorType::KeyValue)).await.unwrap(),
        }
    }
}

async fn router(mut receiver: Receiver<RoutingMessage>) {
    let (mut key_value_sender, mut key_value_receiver) = channel(32);
    let mut key_value_handle = tokio::spawn(
        key_value_actor(key_value_receiver)
    );

    let (heartbeat_sender, heartbeat_receiver) = channel(32);
    tokio::spawn(heartbeat_actor(heartbeat_receiver));

    while let Some(message) = receiver.recv().await {
        match message {
            RoutingMessage::KeyValue(message) => {
                let _ = key_value_sender.send(message).await;
            },
            RoutingMessage::Heartbeat(message) => {
                let _ = heartbeat_sender.send(message).await;
            },
            RoutingMessage::Reset(message) => {
                match message {
                    ActorType::KeyValue | ActorType::Writer => {
                        let (new_key_value_sender, new_key_value_receiver) = channel(
                            32
                        );
                        key_value_handle.abort();
                        key_value_sender = new_key_value_sender;
                        key_value_receiver = new_key_value_receiver;
                        key_value_handle = tokio::spawn(
                            key_value_actor(key_value_receiver)
                        );
                        time::sleep(Duration::from_millis(100)).await;
                    },
                }
            }
        }
    }
}

async fn heartbeat_actor(mut receiver: Receiver<ActorType>) {
    let mut map = HashMap::new();
    let timeout_duration = Duration::from_millis(200);
    loop {
        match time::timeout(timeout_duration, receiver.recv()).await {
            Ok(Some(actor_name)) => map.insert(
                actor_name, Instant::now()
            ),
            Ok(None) => break,
            Err(_) => {
                continue;
            }
        };
        let half_second_ago = Instant::now() -
            Duration::from_millis(500);
        for (key, &value) in map.iter() {
            if value < half_second_ago {
                match key {
                    ActorType::KeyValue | ActorType::Writer => {
                        ROUTER_SENDER.get().unwrap().send(RoutingMessage::Reset(ActorType::KeyValue)).await.unwrap();
                        map.remove(&ActorType::KeyValue);
                        map.remove(&ActorType::Writer);
                        break;
                    }
                }
            }
        }
    }
}


pub async fn set(key: String, value: Vec<u8>) -> Result<(), std::io::Error> {
    let (tx, rx) = oneshot::channel();
    ROUTER_SENDER
        .get()
        .unwrap()
        .send(RoutingMessage::KeyValue(KeyValueMessage::Set(
            SetKeyValueMessage {
                key,
                value,
                response: tx,
            },
        )))
        .await
        .unwrap();
    rx.await.unwrap();
    Ok(())
}

pub async fn get(key: String) -> Result<Option<Vec<u8>>, std::io::Error> {
    let (tx, rx) = oneshot::channel();
    ROUTER_SENDER
        .get()
        .unwrap()
        .send(RoutingMessage::KeyValue(KeyValueMessage::Get(
            GetKeyValueMessage { key, response: tx },
        )))
        .await
        .unwrap();
    Ok(rx.await.unwrap())
}

pub async fn delete(key: String) -> Result<(), std::io::Error> {
    let (tx, rx) = oneshot::channel();
    ROUTER_SENDER
        .get()
        .unwrap()
        .send(RoutingMessage::KeyValue(KeyValueMessage::Delete(
            DeleteKeyValueMessage { key, response: tx },
        )))
        .await
        .unwrap();
    Ok(rx.await.unwrap())
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let (sender, receiver) = channel(32);
    ROUTER_SENDER.set(sender).unwrap();
    tokio::spawn(router(receiver));
    let _ = set("hello".to_string(), b"world".to_vec()).await?;
    let value = get("hello".to_string()).await?;
    println!("value: {:?}", value);
    let value = get("hello".to_string()).await?;
    println!("value: {:?}", value);
    ROUTER_SENDER.get().unwrap().send(
        RoutingMessage::Reset(ActorType::KeyValue)
    ).await.unwrap();
    let value = get("hello".to_string()).await?;
    println!("value: {:?}", value);
    let _ = set("test".to_string(), b"world".to_vec()).await?;
    std::thread::sleep(std::time::Duration::from_secs(1));
    Ok(())
}

