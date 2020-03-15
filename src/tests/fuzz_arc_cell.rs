use crate::arc_cell::ArcCell;
use loom::{cell::CausalCell, sync::Arc, thread};

// TODO(philiphayes): https://github.com/tokio-rs/loom/pull/47

#[test]
#[ignore]
fn arc_cell_fuzz_basic() {
    loom::model(|| {
        let ac1 = Arc::new(ArcCell::new(Arc::new(CausalCell::new(1_u32))));
        let ac2 = Arc::clone(&ac1);

        let t1 = thread::spawn(move || {
            assert_eq!(1, ac1.get().with(|x| unsafe { *x }));
        });

        let t2 = thread::spawn(move || {
            assert_eq!(1, ac2.get().with(|x| unsafe { *x }));
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}
