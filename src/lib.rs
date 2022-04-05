pub mod timer;
mod yield_now;

pub use yield_now::yield_now;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

use crossbeam_channel::{unbounded, Receiver, Sender};

pub struct Task {
    id: usize,
    future: Pin<Box<dyn Future<Output = ()>>>,
}

pub struct TaskWaker {
    task_id: usize,
    sender: Sender<usize>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.sender.send(self.task_id).unwrap();
    }
}

struct SharedState {
    next_id: Cell<usize>,
    pending: RefCell<VecDeque<Task>>,
    due_times: RefCell<BTreeMap<Instant, Vec<Waker>>>,
}

impl SharedState {
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.pending.borrow_mut().push_back(Task {
            id,
            future: Box::pin(future),
        });
    }

    /// check timer that is due
    fn check_timer(&self) {
        let mut due_times = self.due_times.borrow_mut();
        let mut splited = due_times.split_off(&Instant::now());
        std::mem::swap(&mut splited, &mut due_times);
        for v in splited.into_values().flatten() {
            v.wake();
        }
    }

    /// sleep until next timmer due
    fn sleep_until_due(&self) {
        let due_times = self.due_times.borrow_mut();
        let due_time = due_times.keys().next().unwrap();
        std::thread::sleep(due_time.duration_since(Instant::now()));
    }
}

struct JoinHandleInner<T> {
    result: Cell<Option<T>>,
    waker: Cell<Option<Waker>>,
}

pub struct JoinHandle<T> {
    inner: Rc<JoinHandleInner<T>>,
}

impl<T> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(res) = this.inner.result.take() {
            Poll::Ready(res)
        } else {
            this.inner.waker.set(Some(cx.waker().clone()));
            Poll::Pending
        }
    }
}

#[derive(Clone)]
pub struct Spawner {
    state: Rc<SharedState>,
}

impl Spawner {
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        let handle = Rc::new(JoinHandleInner {
            result: Cell::new(None),
            waker: Cell::new(None),
        });
        let handle_ = handle.clone();
        let wrapped = async move {
            let result = future.await;
            handle_.result.set(Some(result));
            if let Some(waker) = handle_.waker.take() {
                waker.wake();
            }
        };
        self.state.spawn(wrapped);
        JoinHandle { inner: handle }
    }

    pub fn sleep(&self, duration: Duration) -> timer::TimerFuture {
        let deadline = Instant::now() + duration;
        timer::TimerFuture::new(deadline, self.clone())
    }
}

pub struct Runtime {
    state: Rc<SharedState>,
    sleeping: RefCell<HashMap<usize, Task>>,
    // receive id of task to wake
    receiver: Receiver<usize>,
    sender: Sender<usize>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        Runtime {
            state: Rc::new(SharedState {
                next_id: Cell::new(0),
                pending: RefCell::new(VecDeque::new()),
                due_times: RefCell::new(BTreeMap::new()),
            }),
            sleeping: Default::default(),
            receiver,
            sender,
        }
    }

    pub fn spawner(&self) -> Spawner {
        Spawner {
            state: self.state.clone(),
        }
    }

    fn new_waker(&self, task_id: usize) -> Waker {
        let waker = TaskWaker {
            task_id,
            sender: self.sender.clone(),
        };
        Waker::from(Arc::new(waker))
    }

    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        let spawner = self.spawner();
        let handle = spawner.spawn(future);
        self.enter();
        handle.inner.result.take().unwrap()
    }

    pub fn enter(&self) {
        loop {
            let task = self.state.pending.borrow_mut().pop_front();
            match task {
                Some(mut task) => {
                    let waker = self.new_waker(task.id);
                    let ctx = &mut Context::from_waker(&waker);

                    match task.future.as_mut().poll(ctx) {
                        Poll::Pending => {
                            // park task
                            self.sleeping.borrow_mut().insert(task.id, task);
                        }
                        Poll::Ready(()) => {}
                    }
                }
                None if self.sleeping.borrow().is_empty() => break,
                None => {
                    self.state.sleep_until_due();
                }
            }

            self.state.check_timer();

            // check if there is a task to wake
            while let Ok(task_id) = self.receiver.try_recv() {
                let task = self.sleeping.borrow_mut().remove(&task_id).unwrap();
                self.state.pending.borrow_mut().push_back(task);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        time::Duration,
    };

    use crate::{yield_now, Runtime, Spawner};

    async fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    async fn fun() -> i32 {
        let a = add(1, 1).await;
        let b = add(a, 1).await;
        b
    }

    #[test]
    fn simple_compose() {
        let runtime = Runtime::new();
        let ans = runtime.block_on(fun());
        assert_eq!(ans, 3);
    }

    async fn fun2(s: Spawner) -> i32 {
        let a = Rc::new(Cell::new(0));
        let mut handles = vec![];
        for _ in 0..6 {
            let a = a.clone();
            let s_ = s.clone();
            let h = s.spawn(async move {
                s_.sleep(Duration::from_millis(100)).await;
                a.set(a.get() + 7);
            });
            handles.push(h);
        }
        for h in handles {
            h.await;
        }
        a.get()
    }

    #[test]
    fn spawn_sleep() {
        let runtime = Runtime::new();
        let spawner = runtime.spawner();
        let ans = runtime.block_on(fun2(spawner));
        assert_eq!(ans, 42);
    }

    #[test]
    fn test_yield() {
        let runtime = Runtime::new();
        let spawner = runtime.spawner();
        let ans = runtime.block_on(async move {
            let objs = Rc::new(RefCell::new(Vec::new()));
            let mut handles = vec![];
            for i in 0..3 {
                let objs = objs.clone();
                let h = spawner.spawn(async move {
                    for _ in 0..2 {
                        objs.borrow_mut().push(i);
                        yield_now().await;
                    }
                });
                handles.push(h);
            }

            for h in handles {
                h.await;
            }

            objs.take()
        });

        assert_eq!(ans, vec![0, 1, 2, 0, 1, 2]);
    }
}
