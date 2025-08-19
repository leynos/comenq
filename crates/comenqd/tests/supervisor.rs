//! Tests for the task supervision logic.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

async fn supervise_until_restarts<F1, F2>(
    mut make1: F1,
    mut make2: F2,
    mut shutdown: watch::Receiver<()>,
) where
    F1: FnMut() -> JoinHandle<anyhow::Result<()>> + Send + 'static,
    F2: FnMut() -> JoinHandle<anyhow::Result<()>> + Send + 'static,
{
    let mut t1 = make1();
    let mut t2 = make2();
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                t1.abort();
                t2.abort();
                break;
            }
            res = &mut t1 => {
                let _ = res;
                tokio::time::sleep(Duration::from_millis(10)).await;
                t1 = make1();
            }
            res = &mut t2 => {
                let _ = res;
                tokio::time::sleep(Duration::from_millis(10)).await;
                t2 = make2();
            }
        }
    }
}

#[tokio::test]
async fn restarts_failed_listener() {
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = Arc::clone(&attempts);
    let listener_maker = {
        let attempts = Arc::clone(&attempts_clone);
        let mut shutdown = shutdown_rx.clone();
        move || {
            let attempts = Arc::clone(&attempts);
            let mut shutdown = shutdown.clone();
            tokio::spawn(async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(anyhow::anyhow!("fail"))
                } else {
                    let _ = shutdown.changed().await;
                    Ok(())
                }
            })
        }
    };

    let worker_maker = {
        let mut shutdown = shutdown_rx.clone();
        move || {
            let mut shutdown = shutdown.clone();
            tokio::spawn(async move {
                let _ = shutdown.changed().await;
                Ok(())
            })
        }
    };

    let supervisor = tokio::spawn(supervise_until_restarts(
        listener_maker,
        worker_maker,
        shutdown_rx,
    ));
    while attempts.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    shutdown_tx.send(()).unwrap();
    supervisor.await.unwrap();
    assert!(attempts.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn restarts_failed_worker() {
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = Arc::clone(&attempts);
    let worker_maker = {
        let attempts = Arc::clone(&attempts_clone);
        let mut shutdown = shutdown_rx.clone();
        move || {
            let attempts = Arc::clone(&attempts);
            let mut shutdown = shutdown.clone();
            tokio::spawn(async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(anyhow::anyhow!("fail"))
                } else {
                    let _ = shutdown.changed().await;
                    Ok(())
                }
            })
        }
    };

    let listener_maker = {
        let mut shutdown = shutdown_rx.clone();
        move || {
            let mut shutdown = shutdown.clone();
            tokio::spawn(async move {
                let _ = shutdown.changed().await;
                Ok(())
            })
        }
    };

    let supervisor = tokio::spawn(supervise_until_restarts(
        listener_maker,
        worker_maker,
        shutdown_rx,
    ));
    while attempts.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    shutdown_tx.send(()).unwrap();
    supervisor.await.unwrap();
    assert!(attempts.load(Ordering::SeqCst) >= 2);
}
