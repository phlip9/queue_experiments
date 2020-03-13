use crate::loom::sync::{atomic::AtomicUsize, Arc};
use std::{fmt, marker::PhantomData, mem, sync::atomic::Ordering};

/// A Cell for containing a strong reference
pub struct ArcCell<T> {
    inner: AtomicUsize,
    _marker: PhantomData<T>,
}

impl<T> ArcCell<T> {
    /// Constructs an ArcCell which initially points to `value`
    pub fn new(value: Arc<T>) -> ArcCell<T> {
        ArcCell {
            inner: AtomicUsize::new(unsafe { mem::transmute(value) }),
            _marker: PhantomData,
        }
    }

    fn take(&self) -> Arc<T> {
        loop {
            let ptr = self.inner.swap(0, Ordering::Acquire);
            if ptr != 0 {
                return unsafe { mem::transmute(ptr) };
            }
        }
    }

    fn put(&self, ptr: Arc<T>) {
        self.inner
            .store(unsafe { mem::transmute(ptr) }, Ordering::Release);
    }

    /// Get the pointer contained in this cell as it exists at this moment
    pub fn get(&self) -> Arc<T> {
        let ptr = self.take();
        let res = ptr.clone();
        self.put(ptr);
        res
    }

    /// Set the pointer for the next observer
    pub fn set(&self, value: Arc<T>) -> Arc<T> {
        let old = self.take();
        self.put(value);
        old
    }
}

impl<T: fmt::Debug> fmt::Debug for ArcCell<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        self.get().fmt(fmt)
    }
}
