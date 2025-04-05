use std::boxed::Box;
use std::future::Future;
use std::pin::Pin;

pub trait AsyncProcess<X, Z> {
    fn get_result(&self, key: X) -> Pin<Box<dyn Future<Output = Result<Z, String>> + Send>>;
}

async fn do_something<T>(async_handle: T, input: i32) -> Result<i32, String>
where
    T: AsyncProcess<i32, i32> + Send + Sync + 'static,
{
    println!("something is happening");
    let result: i32 = async_handle.get_result(input).await?;
    if result > 10 {
        return Err("result is too big".to_string());
    }
    if result == 8 {
        return Ok(result * 2);
    }
    Ok(result * 3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    use mockall::predicate::*;
    use std::boxed::Box;

    mock! {
        DatabaseHandler {}
        impl AsyncProcess<i32, i32> for DatabaseHandler {
            fn get_result(&self, key: i32) -> Pin<Box<dyn Future<Output = Result<i32, String>> + Send>>;
        }
    }

    #[test]
    fn do_something_fail() {
        let mut handle = MockDatabaseHandler::new();
        handle
            .expect_get_result()
            .with(eq(4))
            .returning(|_| Box::pin(async move { Ok(11) }));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = runtime.block_on(do_something(handle, 4));
        assert_eq!(outcome, Err("result is too big".to_string()));
    }
}
