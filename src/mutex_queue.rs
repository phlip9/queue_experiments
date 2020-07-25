use std::task::{Context, Poll, Waker};

/// Assumes any producer or consumer has mutually exclusive access to this queue
/// when manipulating it.
#[derive(Debug)]
pub(crate) struct Queue<T> {
    is_shutdown: bool,
    queue: Vec<Option<T>>,
    tx_waker: MaybeWaker,
    rx_waker: MaybeWaker,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    Closed,
}

#[derive(Debug)]
struct MaybeWaker(Option<Waker>);

impl<T: Unpin> Unpin for Queue<T> {}

impl<T> Queue<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0);
        debug_assert!(capacity.is_power_of_two());

        let mut queue = Vec::with_capacity(capacity);

        for _ in 0..capacity {
            queue.push(None);
        }

        Self {
            is_shutdown: false,
            queue,
            tx_waker: MaybeWaker::new(None),
            rx_waker: MaybeWaker::new(None),
        }
    }

    fn is_tx_slot_available(&self, tx_idx: usize) -> bool {
        debug_assert!(tx_idx < self.queue.len());
        debug_assert!(!self.is_shutdown);

        self.queue[tx_idx].is_none()
    }

    fn is_empty(&self, rx_idx: usize) -> bool {
        debug_assert!(rx_idx < self.queue.len());

        self.queue[rx_idx].is_none()
    }

    pub fn poll_acquire_tx_slot(
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

    pub fn do_send(&mut self, tx_idx: usize, val: T) {
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

    pub fn poll_recv(&mut self, cx: &mut Context<'_>, rx_idx: usize) -> Poll<Option<T>> {
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

    pub fn do_recv(&mut self, rx_idx: usize) -> T {
        debug_assert!(rx_idx < self.queue.len());
        debug_assert!(self.queue[rx_idx].is_some());

        let val = self.queue[rx_idx].take().expect("Assumes not empty");
        self.tx_waker.wake();
        val
    }

    pub fn tx_shutdown(&mut self) {
        if !self.is_shutdown {
            self.is_shutdown = true;
            self.rx_waker.wake();
        }
    }

    pub fn rx_shutdown(&mut self) {
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
