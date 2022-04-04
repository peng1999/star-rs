use std::{future::Future, task::{Poll, Waker, Context}};

pub struct Runtime;

const RAW_WAKER_VTABLE: std::task::RawWakerVTable = std::task::RawWakerVTable::new(
    |_| RAW_WAKER,
    |_| (),
    |_| (),
    |_| (),
);

const RAW_WAKER: std::task::RawWaker = std::task::RawWaker::new(
    std::ptr::null(),
    &RAW_WAKER_VTABLE,
);

impl Runtime {
    pub fn new() -> Self {
        Runtime
    }

    pub fn block_on<F: Future>(&self, fut: F) -> F::Output {
        let mut fut = Box::pin(fut);
        loop {
            let waker = unsafe { Waker::from_raw(RAW_WAKER) };
            let ctx = &mut Context::from_waker(&waker);
            match fut.as_mut().poll(ctx) {
                Poll::Pending => {}
                Poll::Ready(val) => break val,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Runtime;

    async fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    async fn fun() -> i32 {
        let a = add(-1, 1).await;
        let b = add(a, 1).await;
        b - 1
    }

    #[test]
    fn it_works() {
        let runtime = Runtime::new();
        let res = runtime.block_on(fun());
        assert_eq!(res, 0);
    }
}
