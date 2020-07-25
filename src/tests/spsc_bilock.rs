use crate::spsc_bilock::channel;
use std::num::NonZeroUsize;
use tokio_test::{assert_err, assert_ok};

#[tokio::test]
async fn basic() {
    let (mut tx, mut rx) = channel::<u32>(NonZeroUsize::new(1).unwrap());
    assert_ok!(tx.send(1).await);
    assert_eq!(Some(1), rx.recv().await);
}

#[tokio::test]
async fn send_err_after_rx_shutdown() {
    let (mut tx, rx) = channel::<u32>(NonZeroUsize::new(1).unwrap());
    drop(rx);
    assert_err!(tx.send(1).await);
}

#[tokio::test]
async fn send_recv_with_buffer() {
    let (mut tx, mut rx) = channel::<u32>(NonZeroUsize::new(5).unwrap());

    tokio::spawn(async move {
        assert_ok!(tx.send(1).await);
        assert_ok!(tx.send(2).await);
    });

    assert_eq!(Some(1), rx.recv().await);
    assert_eq!(Some(2), rx.recv().await);
    assert_eq!(None, rx.recv().await);
}

#[tokio::test]
async fn send_recv_at_capacity() {
    let (mut tx, mut rx) = channel::<u32>(NonZeroUsize::new(2).unwrap());

    tokio::spawn(async move {
        assert_ok!(tx.send(1).await);
        assert_ok!(tx.send(2).await);
        assert_ok!(tx.send(3).await);
        assert_ok!(tx.send(4).await);
    });

    assert_eq!(Some(1), rx.recv().await);
    assert_eq!(Some(2), rx.recv().await);
    assert_eq!(Some(3), rx.recv().await);
    assert_eq!(Some(4), rx.recv().await);
    assert_eq!(None, rx.recv().await);
}
