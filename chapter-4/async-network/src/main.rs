use std::{future::Future, panic::catch_unwind, thread};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use std::sync::LazyLock;

use async_task::{Runnable, Task};
use futures_lite::future;
use std::time::Instant;
use std::net::TcpStream as StdTcpStream;
use flume::{Sender, Receiver};
use std::net::ToSocketAddrs;
use std::net::Shutdown;
use anyhow::{bail, Context as _, Error, Result};
use async_native_tls::TlsStream;
use http::Uri;
use hyper::{Body, Client, Request, Response};
use smol::{io, prelude::*, Async};

use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll as MioPoll, Token};
use std::io::{Read, Write};

const SERVER: Token = Token(0);
const CLIENT: Token = Token(1);

pub struct ServerFuture {
    pub server: TcpListener,
    pub poll: MioPoll,
}
impl Future for ServerFuture {

    type Output = String;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
            -> Poll<Self::Output> {
        let mut events = Events::with_capacity(1);

        let _ = self.poll.poll(
            &mut events,
            Some(Duration::from_millis(200))
        ).unwrap();


        for event in events.iter() {
            if event.token() == SERVER && event.is_readable() {
                let (mut stream, _) = self.server.accept().unwrap();
                let mut buffer = [0u8; 1024];
                let mut received_data = Vec::new();

                loop {
                    match stream.read(&mut buffer) {
                        Ok(n) if n > 0 => {
                            received_data.extend_from_slice(&buffer[..n]);
                        }
                        Ok(_) => {
                            break;
                        }
                        Err(e) => {
                            eprintln!("Error reading from stream: {}", e);
                            break;
                        }
                    }
                }

                if !received_data.is_empty() {
                    let received_str = String::from_utf8_lossy(&received_data);
                    return Poll::Ready(received_str.to_string())
                }
                cx.waker().wake_by_ref();
                return Poll::Pending
            }
        }
        cx.waker().wake_by_ref();
        return Poll::Pending
    }
}


#[derive(Debug, Clone, Copy)]
enum FutureType {
    High,
    Low
}

fn spawn_task<F, T>(future: F, order: FutureType) -> Task<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{

    static HIGH_CHANNEL: LazyLock<(Sender<Runnable>, Receiver<Runnable>)> =
        LazyLock::new(|| flume::unbounded::<Runnable>());
    static LOW_CHANNEL: LazyLock<(Sender<Runnable>, Receiver<Runnable>)> =
        LazyLock::new(|| flume::unbounded::<Runnable>());
    static HIGH_QUEUE: LazyLock<flume::Sender<Runnable>> = LazyLock::new(|| {
        let high_num = std::env::var("HIGH_NUM").unwrap().parse::<usize>()
            .unwrap();
        for _ in 0..high_num {
            let high_receiver = HIGH_CHANNEL.1.clone();
            let low_receiver = LOW_CHANNEL.1.clone();
            thread::spawn(move || {
                loop {
                    match high_receiver.try_recv() {
                        Ok(runnable) => {
                            println!("running high prioirity task in high queue");
                            let _ = catch_unwind(|| runnable.run());
                        },
                        Err(_) => {
                            match low_receiver.try_recv() {
                                Ok(runnable) => {
                                    println!("running low prioirity task in high queue");
                                    let _ = catch_unwind(|| runnable.run());
                                },
                                Err(_) => {
                                    thread::sleep(Duration::from_millis(100));
                                }
                            }
                        }
                    };
                }
            });
        }
        HIGH_CHANNEL.0.clone()
    });

    static LOW_QUEUE: LazyLock<flume::Sender<Runnable>> = LazyLock::new(|| {
        let low_num = std::env::var("LOW_NUM").unwrap().parse::<usize>()
            .unwrap();
        for _ in 0..low_num {
            let high_receiver = HIGH_CHANNEL.1.clone();
            let low_receiver = LOW_CHANNEL.1.clone();
            thread::spawn(move || {
                loop {
                    match low_receiver.try_recv() {
                        Ok(runnable) => {
                            println!("running low prioirity task in low queue");
                            let _ = catch_unwind(|| runnable.run());
                        },
                        Err(_) => {
                            match high_receiver.try_recv() {
                                Ok(runnable) => {
                                    println!("running high prioirity task in low queue");
                                    let _ = catch_unwind(|| runnable.run());
                                },
                                Err(_) => {
                                    thread::sleep(Duration::from_millis(100));
                                }
                            }
                        }
                    };
                }
            });
        }
        LOW_CHANNEL.0.clone()
    });

    let schedule_high = |runnable| HIGH_QUEUE.send(runnable).unwrap();
    let schedule_low = |runnable| LOW_QUEUE.send(runnable).unwrap();

    let schedule = match order {
        FutureType::High => schedule_high,
        FutureType::Low => schedule_low
    };
    let (runnable, task) = async_task::spawn(future, schedule);
    runnable.schedule();
    task
}

macro_rules! spawn_task {
    ($future:expr) => {
        spawn_task!($future, FutureType::Low)
    };
    ($future:expr, $order:expr) => {
        spawn_task($future, $order)
    };
}

macro_rules! join {
    ($($future:expr),*) => {
        {
            let mut results = Vec::new();
            $(
                results.push(future::block_on($future));
            )*
            results
        }
    };
}

macro_rules! try_join {
    ($($future:expr),*) => {
        {
            let mut results = Vec::new();
            $(
                let result = catch_unwind(|| future::block_on($future));
                results.push(result);
            )*
            results
        }
    };
}

struct Runtime {
    high_num: usize,
    low_num: usize,
}

impl Runtime {
    pub fn new() -> Self {
        let num_cores = std::thread::available_parallelism().unwrap()
            .get();
        Self {
            high_num: num_cores - 2,
            low_num: 1,
        }
    }
    pub fn with_high_num(mut self, num: usize) -> Self {
        self.high_num = num;
        self
    }
    pub fn with_low_num(mut self, num: usize) -> Self {
        self.low_num = num;
        self
    }

    pub fn run(&self) {
        unsafe {
            std::env::set_var("HIGH_NUM", self.high_num.to_string());
            std::env::set_var("LOW_NUM", self.low_num.to_string());
        }
        let high = spawn_task!(async {}, FutureType::High);
        let low = spawn_task!(async {}, FutureType::Low);
        join!(high, low);
    }
}

struct CustomExecutor;

impl<F: Future + Send + 'static> hyper::rt::Executor<F> for CustomExecutor {
    fn execute(&self, fut: F) {
        spawn_task!(async {
            println!("sending request");
            fut.await;
        }).detach();
    }
}

enum CustomStream {
    Plain(Async<StdTcpStream>),
    Tls(TlsStream<Async<StdTcpStream>>),
}

#[derive(Clone)]
struct CustomConnector;

impl hyper::service::Service<Uri> for CustomConnector {
    type Response = CustomStream;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<
        Self::Response, Self::Error>> + Send
    >>;
    fn poll_ready(&mut self, _: &mut Context<'_>)
                  -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move {
            let host = uri.host().context("cannot parse host")?;

            match uri.scheme_str() {
                Some("http") => {
                    let socket_addr = {
                        let host = host.to_string();
                        let port = uri.port_u16().unwrap_or(80);
                        smol::unblock(move || (host.as_str(), port).to_socket_addrs())
                            .await?
                            .next()
                            .context("cannot resolve address")?
                    };
                    let stream = Async::<StdTcpStream>::connect(socket_addr).await?;
                    Ok(CustomStream::Plain(stream))
                }
                Some("https") => {
                    let socket_addr = {
                        let host = host.to_string();
                        let port = uri.port_u16().unwrap_or(443);
                        smol::unblock(move || (host.as_str(), port).to_socket_addrs())
                            .await?
                            .next()
                            .context("cannot resolve address")?
                    };
                    let stream = Async::<StdTcpStream>::connect(socket_addr).await?;
                    let stream = async_native_tls::connect(host, stream).await?;
                    Ok(CustomStream::Tls(stream))
                }
                scheme => bail!("unsupported scheme: {:?}", scheme),
            }
        })

    }
}

impl tokio::io::AsyncRead for CustomStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            CustomStream::Plain(s) => {
                Pin::new(s)
                    .poll_read(cx, buf.initialize_unfilled())
                    .map_ok(|size| {
                        buf.advance(size);
                    })
            }
            CustomStream::Tls(s) => {
                Pin::new(s)
                    .poll_read(cx, buf.initialize_unfilled())
                    .map_ok(|size| {
                        buf.advance(size);
                    })
            }
        }
    }
}

impl tokio::io::AsyncWrite for CustomStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            CustomStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            CustomStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
                  -> Poll<io::Result<()>> {
        match &mut *self {
            CustomStream::Plain(s) => Pin::new(s).poll_flush(cx),
            CustomStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
                     -> Poll<io::Result<()>> {
        match &mut *self {
            CustomStream::Plain(s) => {
                s.get_ref().shutdown(Shutdown::Write)?;
                Poll::Ready(Ok(()))
            }
            CustomStream::Tls(s) => Pin::new(s).poll_close(cx),
        }
    }
}

impl hyper::client::connect::Connection for CustomStream {
    fn connected(&self) -> hyper::client::connect::Connected {
        hyper::client::connect::Connected::new()
    }
}

async fn fetch(req: Request<Body>) -> Result<Response<Body>> {
    Ok(Client::builder()
        .executor(CustomExecutor)
        .build::<_, Body>(CustomConnector)
        .request(req)
        .await?)
}

fn main() -> Result<(), anyhow::Error> {
    Runtime::new().with_low_num(2).with_high_num(4).run();

    let future  = async {
        let req = Request::get("https://www.rust-lang.org")
            .body(Body::empty())
            .unwrap();
        let response = fetch(req).await.unwrap();


        let body_bytes = hyper::body::to_bytes(response.into_body())
            .await.unwrap();
        let html = String::from_utf8(body_bytes.to_vec()).unwrap();
        println!("{}", html);
    };
    let test = spawn_task!(future);
    let _outcome = future::block_on(test);

    let addr = "127.0.0.1:13265".parse()?;
    let mut server = TcpListener::bind(addr)?;
    let mut stream = TcpStream::connect(server.local_addr()?)?;

    let poll: MioPoll = MioPoll::new()?;
    poll.registry()
        .register(&mut server, SERVER, Interest::READABLE)?;

    let server_worker = ServerFuture{
        server,
        poll,
    };
    let test = spawn_task!(server_worker);

    let mut client_poll: MioPoll = MioPoll::new()?;
    client_poll.registry()
        .register(&mut stream, CLIENT, Interest::WRITABLE)?;

    let mut events = Events::with_capacity(128);

    let _ = client_poll.poll(
        &mut events,
        None
    ).unwrap();

    for event in events.iter() {
        if event.token() == CLIENT && event.is_writable() {
            let message = "that's so dingo!\n";
            let _ = stream.write_all(message.as_bytes());
        }
    }

    let outcome = future::block_on(test);
    println!("outcome: {}", outcome);

    Ok(())
}

