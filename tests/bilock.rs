use futures::poll;
use queue_experiments::bilock::BiLock;
use tokio_test::{assert_pending, assert_ready, task};

#[tokio::test]
async fn bilock_basic() {
    let (x, y) = BiLock::new(1);

    {
        let mut lock = assert_ready!(poll!(x.lock()));
        assert_eq!(*lock, 1);
        *lock = 2;

        assert_pending!(poll!(y.lock()));
        assert_pending!(poll!(x.lock()));
    }

    assert_ready!(poll!(y.lock()));
    assert_ready!(poll!(x.lock()));

    {
        let lock = assert_ready!(poll!(y.lock()));
        assert_eq!(*lock, 2);
    }
}

#[test]
fn bilock_readiness() {
    let (x, y) = BiLock::new(1);
    let mut tx = task::spawn(x.lock());
    let mut ty = task::spawn(y.lock());

    let lock = assert_ready!(tx.poll());

    // can't acquire lock, since x holds it
    assert_pending!(ty.poll());

    // once x drops it, y can acquire
    drop(lock);
    assert!(ty.is_woken());
    assert_ready!(ty.poll());
}
