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
