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
