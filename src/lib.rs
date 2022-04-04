use std::{
    cell::{Cell, RefCell},
    collections::{VecDeque, HashMap},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
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

pub struct Runtime {
    next_id: Cell<usize>,
    pending: RefCell<VecDeque<Task>>,
    sleeping: RefCell<HashMap<usize, Task>>,
    // receive id of task to wake
    receiver: Receiver<usize>,
    sender: Sender<usize>,
}

impl Runtime {
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        Runtime {
            next_id: Cell::new(0),
            pending: Default::default(),
            sleeping: Default::default(),
            receiver,
            sender,
        }
    }

    pub fn spawn<F>(&self, future: F)
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

    fn new_waker(&self, task_id: usize) -> Waker {
        let waker = TaskWaker {
            task_id,
            sender: self.sender.clone(),
        };
        Waker::from(Arc::new(waker))
    }

    pub fn run(&self) {
        loop {
            match self.pending.borrow_mut().pop_front() {
                Some(mut task) => {
                    let waker = self.new_waker(task.id);
                    let ctx = &mut Context::from_waker(&waker);

                    match task.future.as_mut().poll(ctx) {
                        Poll::Pending => {
                            // park task
                        }
                        Poll::Ready(()) => {}
                    }
                }
                None if self.sleeping.borrow().is_empty() => break,
                None => {
                    // sleep until next timmer due
                }
            }
            // check if there is a task to wake
            while let Ok(task_id) = self.receiver.try_recv() {
                let task = self.sleeping.borrow_mut().remove(&task_id).unwrap();
                self.pending.borrow_mut().push_back(task);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{rc::Rc, cell::RefCell};

    use crate::Runtime;

    async fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    async fn fun(res: Rc<RefCell<i32>>) {
        let a = add(1, 1).await;
        let b = add(a, 1).await;
        *res.borrow_mut() = b;
    }

    #[test]
    fn it_works() {
        let ans = Rc::new(RefCell::new(0));
        let runtime = Runtime::new();
        runtime.spawn(fun(ans.clone()));
        runtime.run();
        assert_eq!(*ans.borrow(), 3);
    }
}
