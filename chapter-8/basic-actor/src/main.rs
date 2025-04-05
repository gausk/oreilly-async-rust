use tokio::sync::{
    mpsc::channel,
    mpsc::{Receiver, Sender},
    oneshot
};

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc::error::TryRecvError;


struct Message {
    value: i64
}

async fn basic_actor(mut rx: Receiver<Message>) {
    let mut state = 0;

    while let Some(msg) = rx.recv().await {
        state += msg.value;
        //println!("Received: {}", msg.value);
        //println!("State: {}", state);
    }
}

struct RespMessage {
    value: i32,
    responder: oneshot::Sender<i32>
}

async fn resp_actor(mut rx: Receiver<RespMessage>) {
    let mut state = 0;

    while let Some(msg) = rx.recv().await {
        state += msg.value;
        if msg.responder.send(state).is_err() {
            //eprintln!("Failed to send response");
        }
    }
}

async fn actor_replacement(state: Arc<Mutex<i64>>, value: i64) -> i64 {
    let mut state = state.lock().await;
    *state += value;
    return *state
}

#[tokio::main]
async fn main() {
    let iteration = 1000000;
    let now = tokio::time::Instant::now();
    let (tx, rx) = channel::<Message>(100);

    let _actor_handle = tokio::spawn(
        basic_actor(rx)
    );
    for i in 0..iteration {
        let msg = Message { value: 1 };
        tx.send(msg).await.unwrap();
    }
    println!("Elapsed: {:?}", now.elapsed());


    let (tx, rx) = channel::<RespMessage>(100);

    let _resp_actor_handle = tokio::spawn(async {
        resp_actor(rx).await;
    });

    let now = tokio::time::Instant::now();
    for i in 0..iteration {
        let (resp_tx, resp_rx) = oneshot::channel::<i32>();
        let msg = RespMessage {
            value: 1,
            responder: resp_tx
        };
        tx.send(msg).await.unwrap();
        //println!("Response: {}", resp_rx.await.unwrap());
    }

    println!("Elapsed: {:?}", now.elapsed());

    let state = Arc::new(Mutex::new(0));
    let mut handles = Vec::new();

    let now = tokio::time::Instant::now();

    for i in 0..iteration {
        let state_ref = state.clone();
        let future = async move {
            let handle = tokio::spawn(async move {
                actor_replacement(state_ref, i).await
            });
            let _ = handle.await.unwrap();
        };
        handles.push(tokio::spawn(future));
    }
    for handle in handles {
        let _ = handle.await.unwrap();
    }
    println!("Elapsed: {:?}", now.elapsed());

}