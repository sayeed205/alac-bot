use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use engine::queue::{EnqueueOptions, QueueError, SequentialRipQueue};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn value(value: u64) -> Pin<Box<dyn std::future::Future<Output = u64> + Send>> {
    Box::pin(async move { value })
}

async fn wait_for(notify: &Notify) {
    tokio::time::timeout(Duration::from_secs(1), notify.notified())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tasks_execute_sequentially_in_enqueue_order() {
    let queue = SequentialRipQueue::new();
    let order = Arc::new(Mutex::new(Vec::new()));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let queue = queue.clone();
        let order = order.clone();
        let started = started.clone();
        let release = release.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        started.notify_one();
                        Box::pin(async move {
                            order.lock().unwrap().push(0);
                            release.notified().await;
                            0
                        })
                    },
                    None,
                )
                .await
        })
    };
    wait_for(&started).await;
    let second = {
        let queue = queue.clone();
        let order = order.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        let order = order.clone();
                        Box::pin(async move {
                            order.lock().unwrap().push(1);
                            1
                        })
                    },
                    None,
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    let third = {
        let queue = queue.clone();
        let order = order.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        let order = order.clone();
                        Box::pin(async move {
                            order.lock().unwrap().push(2);
                            2
                        })
                    },
                    None,
                )
                .await
        })
    };
    release.notify_one();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    third.await.unwrap().unwrap();
    assert_eq!(&*order.lock().unwrap(), &[0, 1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn positions_skip_first_and_renumber_after_dequeue() {
    let queue = SequentialRipQueue::new();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let positions = Arc::new(Mutex::new(Vec::new()));
    let first = {
        let started = started.clone();
        let release = release.clone();
        let queue = queue.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        started.notify_one();
                        Box::pin(async move {
                            release.notified().await;
                            1
                        })
                    },
                    None,
                )
                .await
        })
    };
    wait_for(&started).await;
    for number in 2..=3 {
        let positions = positions.clone();
        let queue = queue.clone();
        let handle = tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| value(number),
                    Some(EnqueueOptions {
                        on_position_change: Some(Arc::new(move |position| {
                            positions.lock().unwrap().push((number, position))
                        })),
                        on_start: None,
                        signal: None,
                    }),
                )
                .await
                .unwrap();
        });
        if number == 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        } else {
            drop(handle);
        }
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(&*positions.lock().unwrap(), &[(2, 1), (3, 2)]);
    release.notify_one();
    first.await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(positions.lock().unwrap().contains(&(3, 1)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_start_fires_in_execution_order() {
    let queue = SequentialRipQueue::new();
    let starts = Arc::new(Mutex::new(Vec::new()));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let queue = queue.clone();
        let starts = starts.clone();
        let started = started.clone();
        let release = release.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        started.notify_one();
                        Box::pin(async move {
                            release.notified().await;
                            0
                        })
                    },
                    Some(EnqueueOptions {
                        on_position_change: None,
                        on_start: Some(Arc::new(move || starts.lock().unwrap().push(0))),
                        signal: None,
                    }),
                )
                .await
                .unwrap();
        })
    };
    wait_for(&started).await;
    let mut handles = Vec::new();
    for number in 1..3 {
        let starts = starts.clone();
        let queue = queue.clone();
        handles.push(tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| value(number),
                    Some(EnqueueOptions {
                        on_position_change: None,
                        on_start: Some(Arc::new(move || starts.lock().unwrap().push(number))),
                        signal: None,
                    }),
                )
                .await
                .unwrap();
        }));
    }
    release.notify_one();
    first.await.unwrap();
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(&*starts.lock().unwrap(), &[0, 1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_abort_removes_item_renumbers_and_returns_aborted() {
    let queue = SequentialRipQueue::new();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let queue = queue.clone();
        let started = started.clone();
        let release = release.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        started.notify_one();
                        Box::pin(async move {
                            release.notified().await;
                            1
                        })
                    },
                    None,
                )
                .await
        })
    };
    wait_for(&started).await;
    let signal = CancellationToken::new();
    let positions = Arc::new(Mutex::new(Vec::new()));
    let second = {
        let queue = queue.clone();
        let signal = signal.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| value(2),
                    Some(EnqueueOptions {
                        on_position_change: None,
                        on_start: None,
                        signal: Some(signal),
                    }),
                )
                .await
        })
    };
    let third_positions = positions.clone();
    let third = {
        let queue = queue.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| value(3),
                    Some(EnqueueOptions {
                        on_position_change: Some(Arc::new(move |position| {
                            third_positions.lock().unwrap().push(position)
                        })),
                        on_start: None,
                        signal: None,
                    }),
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    signal.cancel();
    assert_eq!(second.await.unwrap().unwrap_err(), QueueError::Aborted);
    assert_eq!(&*positions.lock().unwrap(), &[2, 1]);
    release.notify_one();
    first.await.unwrap().unwrap();
    third.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_abort_cancels_child_but_task_result_propagates() {
    let queue = SequentialRipQueue::new();
    let outer = CancellationToken::new();
    let child_seen = Arc::new(Notify::new());
    let child_seen_task = child_seen.clone();
    let handle = tokio::spawn({
        let queue = queue.clone();
        let outer = outer.clone();
        async move {
            queue
                .enqueue(
                    move |child| {
                        Box::pin(async move {
                            child.cancelled().await;
                            child_seen_task.notify_one();
                            77
                        })
                    },
                    Some(EnqueueOptions {
                        on_position_change: None,
                        on_start: None,
                        signal: Some(outer),
                    }),
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    outer.cancel();
    assert_eq!(handle.await.unwrap().unwrap(), 77);
    wait_for(&child_seen).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_fails_pending_without_touching_active() {
    let queue = SequentialRipQueue::new();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let queue = queue.clone();
        let started = started.clone();
        let release = release.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        started.notify_one();
                        Box::pin(async move {
                            release.notified().await;
                            1
                        })
                    },
                    None,
                )
                .await
        })
    };
    wait_for(&started).await;
    let second = {
        let queue = queue.clone();
        tokio::spawn(async move { queue.enqueue(move |_| value(2), None).await })
    };
    let third = {
        let queue = queue.clone();
        tokio::spawn(async move { queue.enqueue(move |_| value(3), None).await })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    queue.clear();
    assert_eq!(second.await.unwrap().unwrap_err(), QueueError::Cleared);
    assert_eq!(third.await.unwrap().unwrap_err(), QueueError::Cleared);
    release.notify_one();
    assert_eq!(first.await.unwrap().unwrap(), 1);
}

#[tokio::test]
async fn pre_aborted_enqueue_does_not_queue() {
    let queue = SequentialRipQueue::new();
    let signal = CancellationToken::new();
    signal.cancel();
    let result = queue
        .enqueue(
            |_| value(1),
            Some(EnqueueOptions {
                on_position_change: None,
                on_start: None,
                signal: Some(signal),
            }),
        )
        .await;
    assert_eq!(result.unwrap_err(), QueueError::Aborted);
    assert_eq!(queue.get_pending_count(), 0);
    assert!(!queue.is_processing());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn processing_and_pending_counts_transition() {
    let queue = SequentialRipQueue::new();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let queue = queue.clone();
        let started = started.clone();
        let release = release.clone();
        tokio::spawn(async move {
            queue
                .enqueue(
                    move |_| {
                        started.notify_one();
                        Box::pin(async move {
                            release.notified().await;
                            1
                        })
                    },
                    None,
                )
                .await
        })
    };
    wait_for(&started).await;
    assert!(queue.is_processing());
    assert_eq!(queue.get_pending_count(), 0);
    let second = {
        let queue = queue.clone();
        tokio::spawn(async move { queue.enqueue(move |_| value(2), None).await })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(queue.get_pending_count(), 1);
    release.notify_one();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert!(!queue.is_processing());
    assert_eq!(queue.get_pending_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_panic_does_not_stall_following_items() {
    let queue = SequentialRipQueue::new();
    let panic = queue
        .enqueue(|_| Box::pin(async { panic!("boom") }), None)
        .await;
    assert!(matches!(panic, Err(QueueError::Task(message)) if message == "boom"));
    assert_eq!(queue.enqueue(|_| value(2), None).await.unwrap(), 2);
}
