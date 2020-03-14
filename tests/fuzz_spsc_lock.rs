extern crate loom;

#[allow(dead_code)]
#[path = "../src/spsc_lock.rs"]
mod spsc_lock;

use futures::{
    future,
    sink::Sink,
    stream::{Stream, StreamExt},
};
use loom::{future::block_on, thread};
use spsc_lock::channel;
use std::{num::NonZeroUsize, pin::Pin, task::Poll};
use tokio_test::{assert_err, assert_ok};

#[test]
fn spsc_lock_fuzz_send_recv() {
    loom::model(|| {
        let (mut tx, mut rx) = channel(NonZeroUsize::new(2).unwrap());

        let th_tx = thread::spawn(move || {
            assert_ok!(block_on(tx.send(1)));
            assert_ok!(block_on(tx.send(2)));
            assert_ok!(block_on(tx.send(3)));
            // assert_ok!(block_on(tx.send(4)));
        });

        let th_rx = thread::spawn(move || {
            assert_eq!(Some(1), block_on(rx.next()));
            assert_eq!(Some(2), block_on(rx.recv()));
            assert_eq!(Some(3), block_on(rx.recv()));
            // assert_eq!(Some(4), block_on(rx.recv()));
            assert_eq!(None, block_on(rx.next()));
        });

        th_tx.join().unwrap();
        th_rx.join().unwrap();
    });
}

#[test]
fn spsc_lock_fuzz_send_close() {
    loom::model(|| {
        let (tx, mut rx) = channel::<()>(NonZeroUsize::new(1).unwrap());

        let th_rx = thread::spawn(move || {
            assert_eq!(None, block_on(rx.recv()));
        });

        let th_tx = thread::spawn(move || {
            drop(tx);
        });

        th_tx.join().unwrap();
        th_rx.join().unwrap();
    });
}

#[test]
fn spsc_lock_fuzz_recv_close() {
    loom::model(|| {
        let (mut tx, rx) = channel(NonZeroUsize::new(1).unwrap());

        let th_tx = thread::spawn(move || {
            if block_on(tx.send(1)).is_ok() {
                assert_err!(block_on(tx.send(2)));
            }
        });

        let th_rx = thread::spawn(move || {
            drop(rx);
        });

        th_tx.join().unwrap();
        th_rx.join().unwrap();
    });
}

#[test]
fn spsc_lock_change_rx_task() {
    loom::model(|| {
        let (mut tx, mut rx) = channel(NonZeroUsize::new(1).unwrap());

        let th_tx = thread::spawn(move || {
            block_on(tx.send(1)).unwrap();
        });

        let th_rx = thread::spawn(move || {
            let ready = block_on(future::poll_fn(|cx| {
                match Pin::new(&mut rx).poll_next(cx) {
                    Poll::Ready(val) => {
                        assert_eq!(Some(1), val);
                        Poll::Ready(true)
                    }
                    Poll::Pending => Poll::Ready(false),
                }
            }));

            if ready {
                None
            } else {
                Some(rx)
            }
        });

        let rx = th_rx.join().unwrap();

        if let Some(mut rx) = rx {
            assert_eq!(Some(1), block_on(rx.next()));
            assert_eq!(None, block_on(rx.next()));
        }

        th_tx.join().unwrap();
    })
}

#[test]
fn spsc_lock_change_tx_task() {
    loom::model(|| {
        let (mut tx, mut rx) = channel(NonZeroUsize::new(1).unwrap());

        let th_tx = thread::spawn(move || {
            let ready = block_on(future::poll_fn(|cx| {
                match Pin::new(&mut tx).poll_ready(cx) {
                    Poll::Ready(val) => {
                        assert_eq!(Ok(()), val);
                        Poll::Ready(true)
                    }
                    Poll::Pending => Poll::Ready(false),
                }
            }));

            if ready {
                Pin::new(&mut tx).start_send(1).unwrap();
                None
            } else {
                Some(tx)
            }
        });

        let th_rx = thread::spawn(move || {
            assert_eq!(Some(1), block_on(rx.next()));
            assert_eq!(None, block_on(rx.next()));
        });

        let tx = th_tx.join().unwrap();

        if let Some(mut tx) = tx {
            block_on(tx.send(1)).unwrap();
        }

        th_rx.join().unwrap();
    })
}
