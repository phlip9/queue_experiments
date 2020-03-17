use crate::{
    loom::sync::{Arc, Mutex},
    mutex_queue::{Queue, SendError},
};
use futures::{future, ready, sink::Sink, stream::Stream};
use std::{
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Debug)]
pub struct Sender<T> {
    inner: Arc<Mutex<Queue<T>>>,
    tx_idx: usize,
    capacity: usize,
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Arc<Mutex<Queue<T>>>,
    rx_idx: usize,
    capacity: usize,
}

// 150 Ks
// 2.3 Ks
// 2.45 Ks

pub fn channel<T>(capacity: NonZeroUsize) -> (Sender<T>, Receiver<T>) {
    let capacity = capacity
        .get()
        .checked_next_power_of_two()
        .expect("capacity is too large");

    let inner = Arc::new(Mutex::new(Queue::new(capacity)));

    let tx = Sender {
        inner: inner.clone(),
        tx_idx: 0,
        capacity,
    };

    let rx = Receiver {
        inner,
        rx_idx: 0,
        capacity,
    };

    (tx, rx)
}

#[inline]
fn inc_idx(idx: usize, capacity: usize) -> usize {
    debug_assert!(idx < capacity);
    debug_assert!(capacity.is_power_of_two());

    (idx + 1) & (capacity - 1)
}

// ============= Sender ============= //

impl<T> Sender<T> {
    /// Acquire an open sending slot.
    ///
    /// If the queue is at capacity, this will return `Poll::Pending` and
    /// register the passed `Waker` for wakeup when there is free space. When
    /// this returns `Poll::Ready(Ok(()))`, the sender is guaranteed for the
    /// next `Sender::do_send` to succeed without overwriting an existing
    /// element in the queue.
    pub fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        self.inner
            .lock()
            .unwrap()
            .poll_acquire_tx_slot(cx, self.tx_idx)
    }

    /// Append an element into the queue.
    ///
    /// If `Sender::poll_ready()` is not called before this, `do_send()`
    /// might overwrite an existing element.
    pub fn do_send(&mut self, val: T) {
        self.inner.lock().unwrap().do_send(self.tx_idx, val);
        self.tx_idx = inc_idx(self.tx_idx, self.capacity);
    }

    /// Acquire an open sending slot and send an element over the queue.
    ///
    /// This is more efficient than `SinkExt::send` as it only acquires
    /// the lock once in fast path.
    pub async fn send(&mut self, val: T) -> Result<(), SendError> {
        let mut val = Some(val);

        future::poll_fn(|cx: &mut Context<'_>| {
            let mut lock = self.inner.lock().unwrap();

            match lock.poll_acquire_tx_slot(cx, self.tx_idx) {
                Poll::Ready(Ok(())) => {
                    lock.do_send(self.tx_idx, val.take().unwrap());
                    self.tx_idx = inc_idx(self.tx_idx, self.capacity);
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }
}

impl<T> Sink<T> for Sender<T> {
    type Error = SendError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Sender::poll_ready(self.get_mut(), cx)
    }

    fn start_send(self: Pin<&mut Self>, val: T) -> Result<(), Self::Error> {
        Sender::do_send(self.get_mut(), val);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut lock = self.inner.lock().unwrap();

        lock.tx_shutdown();

        drop(lock);
    }
}

// ============= Receiver ============= //

impl<T> Receiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        future::poll_fn(|cx: &mut Context<'_>| self.poll_recv(cx)).await
    }

    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let res = ready!(self.inner.lock().unwrap().poll_recv(cx, self.rx_idx));
        self.rx_idx = inc_idx(self.rx_idx, self.capacity);
        Poll::Ready(res)
    }

    pub fn shutdown(&mut self) {
        self.inner.lock().unwrap().rx_shutdown();
    }
}

impl<T> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.get_mut().poll_recv(cx)
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
