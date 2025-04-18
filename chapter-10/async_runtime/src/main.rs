use std::{
    future::Future,
    task::{Context, Poll},
    pin::Pin
};
pub struct CountingFuture {
    pub count: i32,
}

impl Future for CountingFuture {
    type Output = i32;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
            -> Poll<Self::Output> {
        self.count += 1;
        if self.count == 4 {
            println!("CountingFuture is done!");
            Poll::Ready(self.count)
        } else {
            cx.waker().wake_by_ref();
            println!(
                "CountingFuture is not done yet! {}",
                self.count
            );
            Poll::Pending
        }
    }
}

fn main() {
    let counter = CountingFuture { count: 0 };
    let counter_two = CountingFuture { count: 0 };
    let mut executor = async_runtime::executor::Executor::new();
    let handle = executor.spawn(counter);
    let _handle_two = executor.spawn(counter_two);
    std::thread::spawn(move || {
        loop {
            executor.poll();
        }
    });
    let result = handle.recv().unwrap();
    println!("Result: {}", result);
}
