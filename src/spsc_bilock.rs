use crate::loom::future::block_on;
use crate::{
    bilock::BiLock,
    mutex_queue::{Queue, SendError},
};
use futures::{future, ready, sink::Sink, stream::Stream};
use std::{
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Debug)]
pub struct Sender<T: Unpin> {
    inner: BiLock<Queue<T>>,
    tx_idx: usize,
    capacity: usize,
}

#[derive(Debug)]
pub struct Receiver<T: Unpin> {
    inner: BiLock<Queue<T>>,
    rx_idx: usize,
    capacity: usize,
}

pub fn channel<T: Unpin>(capacity: NonZeroUsize) -> (Sender<T>, Receiver<T>) {
    let capacity = capacity
        .get()
        .checked_next_power_of_two()
        .expect("capacity is too large");

    let (tx_lock, rx_lock) = BiLock::new(Queue::new(capacity));

    let tx = Sender {
        inner: tx_lock,
        tx_idx: 0,
        capacity,
    };

    let rx = Receiver {
        inner: rx_lock,
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

impl<T: Unpin> Sender<T> {
    /// Acquire an open sending slot.
    ///
    /// If the queue is at capacity, this will return `Poll::Pending` and
    /// register the passed `Waker` for wakeup when there is free space. When
    /// this returns `Poll::Ready(Ok(()))`, the sender is guaranteed for the
    /// next `Sender::do_send` to succeed without overwriting an existing
    /// element in the queue.
    pub fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        let mut lock = ready!(self.inner.poll_lock(cx));
        lock.poll_acquire_tx_slot(cx, self.tx_idx)

        // self.inner
        //     .lock()
        //     .unwrap()
        //     .poll_acquire_tx_slot(cx, self.tx_idx)
    }

    /// Append an element into the queue.
    ///
    /// If `Sender::poll_ready()` is not called before this, `do_send()`
    /// might overwrite an existing element.
    pub fn do_send(&mut self, val: T) {
        // TODO(philiphayes): figure out cleaner way
        let mut lock = block_on(self.inner.lock());
        lock.do_send(self.tx_idx, val);
        self.tx_idx = inc_idx(self.tx_idx, self.capacity);

        // self.inner.lock().unwrap().do_send(self.tx_idx, val);
        // self.tx_idx = inc_idx(self.tx_idx, self.capacity);
    }

    /// Acquire an open sending slot and send an element over the queue.
    ///
    /// This is more efficient than `SinkExt::send` as it only acquires
    /// the lock once in fast path.
    pub async fn send(&mut self, val: T) -> Result<(), SendError> {
        let mut val = Some(val);

        future::poll_fn(|cx: &mut Context<'_>| {
            let mut lock = ready!(self.inner.poll_lock(cx));

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

impl<T: Unpin> Sink<T> for Sender<T> {
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

impl<T: Unpin> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut lock = block_on(self.inner.lock());

        lock.tx_shutdown();

        drop(lock);
    }
}

// ============= Receiver ============= //

impl<T: Unpin> Receiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        future::poll_fn(|cx: &mut Context<'_>| self.poll_recv(cx)).await
    }

    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut lock = ready!(self.inner.poll_lock(cx));
        let res = ready!(lock.poll_recv(cx, self.rx_idx));
        self.rx_idx = inc_idx(self.rx_idx, self.capacity);
        Poll::Ready(res)
    }

    pub fn shutdown(&mut self) {
        let mut lock = block_on(self.inner.lock());
        lock.rx_shutdown();
    }
}

impl<T: Unpin> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.get_mut().poll_recv(cx)
    }
}

impl<T: Unpin> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
