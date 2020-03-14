pub(crate) mod sync {
    pub(crate) use std::sync::{atomic, Arc};

    use parking_lot;
    use std::sync::LockResult;

    #[derive(Debug)]
    pub(crate) struct Mutex<T>(parking_lot::Mutex<T>);

    impl<T> Mutex<T> {
        pub(crate) fn new(data: T) -> Mutex<T> {
            Mutex(parking_lot::Mutex::new(data))
        }

        pub(crate) fn lock(&self) -> LockResult<parking_lot::MutexGuard<T>> {
            Ok(self.0.lock())
        }
    }
}

pub(crate) mod cell {
    use std::cell::UnsafeCell;

    #[derive(Debug)]
    pub(crate) struct CausalCell<T>(UnsafeCell<T>);

    impl<T> CausalCell<T> {
        pub(crate) const fn new(data: T) -> CausalCell<T> {
            CausalCell(UnsafeCell::new(data))
        }

        pub(crate) fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*const T) -> R,
        {
            f(self.0.get())
        }

        pub(crate) fn with_mut<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*mut T) -> R,
        {
            f(self.0.get())
        }
    }
}
