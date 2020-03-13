use crate::loom::sync::{Arc, Mutex};
use futures::{future, ready, sink::Sink, stream::Stream};
use std::{
    pin::Pin,
    task::{Context, Poll, Waker},
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

#[derive(Debug)]
struct MaybeWaker(Option<Waker>);

#[derive(Debug)]
struct Queue<T> {
    is_shutdown: bool,
    queue: Vec<Option<T>>,
    tx_waker: MaybeWaker,
    rx_waker: MaybeWaker,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    Closed,
}

// 150 Ks
// 2.3 Ks
// 2.45 Ks

pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(capacity > 0, "cannot have a zero-capacity spsc channel");

    let capacity = capacity
        .checked_next_power_of_two()
        .expect("capacity is too large");

    let mut queue = Vec::with_capacity(capacity);

    for _ in 0..capacity {
        queue.push(None);
    }

    let inner = Arc::new(Mutex::new(Queue {
        is_shutdown: false,
        queue,
        tx_waker: MaybeWaker::new(None),
        rx_waker: MaybeWaker::new(None),
    }));

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

// ============= Queue ============= //

impl<T> Queue<T> {
    fn is_tx_slot_available(&self, tx_idx: usize) -> bool {
        debug_assert!(tx_idx < self.queue.len());
        debug_assert!(!self.is_shutdown);

        self.queue[tx_idx].is_none()
    }

    fn is_empty(&self, rx_idx: usize) -> bool {
        debug_assert!(rx_idx < self.queue.len());

        self.queue[rx_idx].is_none()
    }

    fn poll_acquire_tx_slot(
        &mut self,
        cx: &mut Context<'_>,
        tx_idx: usize,
    ) -> Poll<Result<(), SendError>> {
        if self.is_shutdown {
            Poll::Ready(Err(SendError::Closed))
        } else if self.is_tx_slot_available(tx_idx) {
            Poll::Ready(Ok(()))
        } else {
            self.tx_waker.register_by_ref(cx.waker());
            Poll::Pending
        }
    }

    fn do_send(&mut self, tx_idx: usize, val: T) {
        debug_assert!(tx_idx < self.queue.len());
        debug_assert!(self.queue[tx_idx].is_none());

        // If using `SinkExt::send`, acquiring a tx slot and actually sending the
        // value happen in two separate steps, so rx can shutdown between them.
        // If this happens, we just silently drop the value, since the rx will
        // never receive it anyway.
        if !self.is_shutdown {
            self.queue[tx_idx] = Some(val);
            self.rx_waker.wake();
        }
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, rx_idx: usize) -> Poll<Option<T>> {
        debug_assert!(rx_idx < self.queue.len());

        if self.is_empty(rx_idx) {
            if self.is_shutdown {
                return Poll::Ready(None);
            } else {
                self.rx_waker.register_by_ref(cx.waker());
                return Poll::Pending;
            }
        } else {
            let val = self.do_recv(rx_idx);
            Poll::Ready(Some(val))
        }
    }

    fn do_recv(&mut self, rx_idx: usize) -> T {
        debug_assert!(rx_idx < self.queue.len());
        debug_assert!(self.queue[rx_idx].is_some());

        let val = self.queue[rx_idx].take().expect("Assumes not empty");
        self.tx_waker.wake();
        val
    }

    fn tx_shutdown(&mut self) {
        if !self.is_shutdown {
            self.is_shutdown = true;
            self.rx_waker.wake();
        }
    }

    fn rx_shutdown(&mut self) {
        if !self.is_shutdown {
            self.is_shutdown = true;
            self.tx_waker.wake();
        }
    }
}

impl MaybeWaker {
    fn new(waker: Option<Waker>) -> Self {
        Self(waker)
    }

    fn wake(&mut self) {
        self.0.take().map(Waker::wake);
    }

    #[allow(dead_code)]
    fn register(&mut self, waker: Waker) {
        self.0 = Some(waker);
    }

    fn register_by_ref(&mut self, waker_ref: &Waker) {
        self.0 = Some(waker_ref.clone());
    }
}
