extern crate loom;

#[allow(dead_code)]
#[path = "../src/bilock.rs"]
mod bilock;

use bilock::BiLock;
use loom::{future::block_on, sync::CausalCell, thread};

#[test]
fn bilock_fuzz_contended_lock() {
    loom::model(|| {
        // use inner CausalCell so loom can check that BiLock actually provides
        // mutually exclusive mutable access.
        let (x, y) = BiLock::new(CausalCell::new(1));

        let th1 = thread::spawn(move || {
            block_on(x.lock()).with_mut(|v| unsafe { *v += 1 });
            block_on(x.lock()).with_mut(|v| unsafe { *v += 1 });
            // block_on(x.lock()).with_mut(|v| unsafe { *v += 1 });
        });

        let th2 = thread::spawn(move || {
            block_on(y.lock()).with_mut(|v| unsafe { *v += 10 });
            block_on(y.lock()).with_mut(|v| unsafe { *v += 10 });
            // block_on(y.lock()).with_mut(|v| unsafe { *v += 10 });
            y
        });

        th1.join().unwrap();
        let y = th2.join().unwrap();

        let v = block_on(y.lock()).with_mut(|v| unsafe { *v });
        assert_eq!(23, v);
        // assert_eq!(34, v);
    });
}
