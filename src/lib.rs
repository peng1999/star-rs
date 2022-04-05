pub mod timer;

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
}

#[derive(Clone)]
pub struct Spawner {
    state: Rc<SharedState>,
}

impl Spawner {
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        self.state.spawn(future);
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

    pub fn run(&self) {
        loop {
            match self.state.pending.borrow_mut().pop_front() {
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
                    // sleep until next timmer due
                    let due_times = self.state.due_times.borrow_mut();
                    let due_time = due_times.keys().next().unwrap();
                    std::thread::sleep(due_time.duration_since(Instant::now()));
                }
            }
            // check timer that is due
            let mut due_times = self.state.due_times.borrow_mut();
            let mut splited = due_times.split_off(&Instant::now());
            std::mem::swap(&mut splited, &mut due_times);
            for v in splited.into_values().flatten() {
                v.wake();
            }
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
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use crate::{Runtime, Spawner};

    async fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    async fn fun(res: Rc<RefCell<i32>>) {
        let a = add(1, 1).await;
        let b = add(a, 1).await;
        *res.borrow_mut() = b;
    }

    #[test]
    fn simple_compose() {
        let ans = Rc::new(RefCell::new(0));
        let runtime = Runtime::new();
        let spawner = runtime.spawner();
        spawner.spawn(fun(ans.clone()));
        runtime.run();
        assert_eq!(*ans.borrow(), 3);
    }

    async fn fun2(res: Rc<RefCell<i32>>, s: Spawner) {
        s.sleep(Duration::from_millis(100)).await;
        *res.borrow_mut() += 21;
    }

    #[test]
    fn spawn_sleep() {
        let ans = Rc::new(RefCell::new(0));
        let runtime = Runtime::new();
        let spawner = runtime.spawner();
        spawner.spawn(fun2(ans.clone(), spawner.clone()));
        spawner.spawn(fun2(ans.clone(), spawner.clone()));
        runtime.run();
        assert_eq!(*ans.borrow(), 42);
    }
}
