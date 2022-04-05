use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Instant,
};

use crate::{SharedState, Spawner};

pub struct TimerFuture {
    deadline: Instant,
    state: Rc<SharedState>,
}

impl TimerFuture {
    pub(crate) fn new(deadline: Instant, spawner: Spawner) -> Self {
        Self {
            deadline,
            state: spawner.state,
        }
    }
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.deadline > Instant::now() {
            self.state
                .due_times
                .borrow_mut()
                .entry(self.deadline)
                .or_insert_with(Vec::new)
                .push(cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}
