extern crate loom;

#[allow(dead_code)]
#[path = "../src/bilock.rs"]
mod bilock;

use bilock::BiLock;
use loom::{future::block_on, thread};

#[test]
fn bilock_fuzz_contended_lock() {
    loom::model(|| {
        // use inner CausalCell so loom can check that BiLock actually provides
        // mutually exclusive mutable access.
        let (x, y) = BiLock::new(1);

        let th1 = thread::spawn(move || {
            {
                let mut lock = block_on(x.lock());
                *lock += 1;
            }
            {
                let mut lock = block_on(x.lock());
                *lock += 1;
            }
        });

        let th2 = thread::spawn(move || {
            {
                let mut lock = block_on(y.lock());
                *lock += 10;
            }
            {
                let mut lock = block_on(y.lock());
                *lock += 10;
            }
            y
        });

        th1.join().unwrap();
        let y = th2.join().unwrap();

        let v = *block_on(y.lock());
        assert_eq!(23, v);
    });
}
