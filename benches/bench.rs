use criterion::{black_box, criterion_group, criterion_main, Bencher, Criterion, Throughput};
use futures::{
    channel::mpsc as futures_mpsc,
    executor::block_on,
    sink::{Sink, SinkExt},
    stream::{Stream, StreamExt},
};
use heck::SnakeCase;
use queue_experiments::spsc_lock;
use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
    thread,
};
use tokio::sync::mpsc as tokio_mpsc;

macro_rules! gen_bench_variants {
    // this macro expands by the $msg_sizes "array", then the inner macro expands by
    // the $queue "array".
    ($group: expr, $bench_fn: ident, $throughput: expr, $queue_sizes: expr, [$( $msg_sizes: ty),+], $rest: tt) => {{
        $(
            gen_bench_variants_inner!($group, $bench_fn, $throughput, $queue_sizes, $msg_sizes, $rest);
        )+
    }};
}

macro_rules! gen_bench_variants_inner {
    ($group: expr, $bench_fn: ident, $throughput: expr, $queue_sizes: expr, $msg_size: ty, [$($queue: ty),+]) => {{
        $(
            for queue_size in $queue_sizes.iter() {
                let queue_str = stringify! {$queue};
                // trim the '<_>' from the end
                let queue_str = &queue_str[..queue_str.len()-3].to_snake_case();

                let msg_size_str = stringify! {$msg_size};
                let msg_size_str = msg_size_str.to_snake_case();

                $group.throughput($throughput(queue_size));

                $group.bench_with_input(
                    format!(
                        "queue={}/msg_size={}/queue_size={}",
                        queue_str,
                        msg_size_str,
                        queue_size,
                    ),
                    &queue_size,
                    $bench_fn::<$msg_size, $queue>,
                );
            }
        )+
    }};
}

const CONTENDED_ITERS: usize = 500;

const SIZE_SMALL: usize = 8;
const SIZE_MEDIUM: usize = SIZE_SMALL * 64;
const SIZE_LARGE: usize = SIZE_MEDIUM * 64;

#[derive(Copy, Clone)]
struct Small([u8; SIZE_SMALL]);

#[derive(Copy, Clone)]
struct Medium([u8; SIZE_MEDIUM]);

#[derive(Copy, Clone)]
struct Large([u8; SIZE_LARGE]);

impl Default for Small {
    fn default() -> Self {
        Self([42u8; SIZE_SMALL])
    }
}

impl Default for Medium {
    fn default() -> Self {
        Self([42u8; SIZE_MEDIUM])
    }
}

impl Default for Large {
    fn default() -> Self {
        Self([42u8; SIZE_LARGE])
    }
}

struct SpscLock<T>(PhantomData<T>);
struct FuturesMpsc<T>(PhantomData<T>);
struct TokioMpsc<T>(PhantomData<T>);

struct TokioMpscSender<T> {
    inner: tokio_mpsc::Sender<T>,
}

impl<T> Sink<T> for TokioMpscSender<T> {
    type Error = ();

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        tokio_mpsc::Sender::poll_ready(&mut self.get_mut().inner, cx).map_err(|_| ())
    }

    fn start_send(self: Pin<&mut Self>, msg: T) -> Result<(), Self::Error> {
        self.get_mut().inner.try_send(msg).map_err(|_| ())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

trait QueueFactory<T: Send> {
    type Sender: Sink<T> + Send + Unpin;
    type Receiver: Stream<Item = T> + Send + Unpin;

    fn new(capacity: usize) -> (Self::Sender, Self::Receiver);
}

impl<T: Send> QueueFactory<T> for SpscLock<T> {
    type Sender = spsc_lock::Sender<T>;
    type Receiver = spsc_lock::Receiver<T>;

    fn new(capacity: usize) -> (Self::Sender, Self::Receiver) {
        spsc_lock::channel::<T>(capacity)
    }
}

impl<T: Send> QueueFactory<T> for FuturesMpsc<T> {
    type Sender = futures_mpsc::Sender<T>;
    type Receiver = futures_mpsc::Receiver<T>;

    fn new(capacity: usize) -> (Self::Sender, Self::Receiver) {
        futures_mpsc::channel::<T>(capacity)
    }
}

impl<T: Send> QueueFactory<T> for TokioMpsc<T> {
    type Sender = TokioMpscSender<T>;
    type Receiver = tokio_mpsc::Receiver<T>;

    fn new(capacity: usize) -> (Self::Sender, Self::Receiver) {
        // tokio_mpsc::channel::<T>(capacity)
        let (sender, receiver) = tokio_mpsc::channel::<T>(capacity);
        (TokioMpscSender { inner: sender }, receiver)
    }
}

fn bench_send_1_recv_1_inner<T, Q>(b: &mut Bencher, queue_size: &&usize)
where
    T: Default + Clone + Send,
    Q: QueueFactory<T>,
{
    let queue_size = **queue_size;
    let val = T::default();
    let (mut tx, mut rx) = Q::new(queue_size);

    b.iter(|| {
        let _ = block_on(tx.send(val.clone()));
        let _ = black_box(block_on(rx.next()));
    })
}

fn bench_send_1_recv_1(c: &mut Criterion) {
    let mut group = c.benchmark_group("send_1_recv_1");
    gen_bench_variants!(
        group,
        bench_send_1_recv_1_inner,
        |_| Throughput::Elements(1),
        // queue sizes
        [1_usize, 16, 128],
        // msg sizes
        // [Small, Medium, Large],
        [Small],
        // queue types
        // [SpscLock<_>, FuturesMpsc<_>, TokioMpsc<_>]
        [SpscLock<_>, TokioMpsc<_>]
    );
    group.finish();
}

fn bench_contended_inner<T, Q>(b: &mut Bencher, queue_size: &&usize)
where
    T: Default + Clone + Send + 'static,
    Q: QueueFactory<T>,
    Q::Sender: 'static,
{
    let queue_size = **queue_size;
    let val = T::default();
    let (bench_start_tx, bench_start_rx) = ::std::sync::mpsc::channel::<Q::Sender>();

    let sender_thread = thread::spawn(move || {
        for mut tx in bench_start_rx.iter() {
            for _ in 0..CONTENDED_ITERS {
                let _ = block_on(tx.send(val.clone()));
            }
        }
    });

    b.iter(|| {
        let (tx, mut rx) = Q::new(queue_size);

        bench_start_tx.send(tx).unwrap();

        while let Some(val) = block_on(rx.next()) {
            let _ = black_box(val);
        }
    });

    drop(bench_start_tx);

    sender_thread.join().unwrap();
}

fn bench_contended(c: &mut Criterion) {
    // add underscores so bench filtering doesn't collide with 'uncontended'
    let mut group = c.benchmark_group("__contended");
    gen_bench_variants!(
        group,
        bench_contended_inner,
        |_| Throughput::Elements(CONTENDED_ITERS as u64),
        // queue sizes
        [1_usize, 16, 128],
        // msg sizes
        // [Small, Medium, Large],
        [Small],
        // queue types
        // [SpscLock<_>, FuturesMpsc<_>, TokioMpsc<_>]
        [SpscLock<_>, TokioMpsc<_>]
    );
    group.finish();
}

fn bench_contended2_spsc_lock_inner<T>(b: &mut Bencher, queue_size: &&usize)
where
    T: Default + Clone + Send + 'static,
{
    let queue_size = **queue_size;
    let val = T::default();
    let (bench_start_tx, bench_start_rx) = ::std::sync::mpsc::channel::<spsc_lock::Sender<T>>();

    let sender_thread = thread::spawn(move || {
        for mut tx in bench_start_rx.iter() {
            for _ in 0..CONTENDED_ITERS {
                let _ = block_on(tx.send(val.clone()));
            }
        }
    });

    b.iter(|| {
        let (tx, mut rx) = spsc_lock::channel(queue_size);

        bench_start_tx.send(tx).unwrap();

        while let Some(val) = block_on(rx.next()) {
            let _ = black_box(val);
        }
    });

    drop(bench_start_tx);

    sender_thread.join().unwrap();
}

fn bench_contended2_tokio_mpsc_inner<T>(b: &mut Bencher, queue_size: &&usize)
where
    T: Default + Clone + Send + 'static,
{
    let queue_size = **queue_size;
    let val = T::default();
    let (bench_start_tx, bench_start_rx) = ::std::sync::mpsc::channel::<tokio_mpsc::Sender<T>>();

    let sender_thread = thread::spawn(move || {
        for mut tx in bench_start_rx.iter() {
            for _ in 0..CONTENDED_ITERS {
                let _ = block_on(tx.send(val.clone()));
            }
        }
    });

    b.iter(|| {
        let (tx, mut rx) = tokio_mpsc::channel(queue_size);

        bench_start_tx.send(tx).unwrap();

        while let Some(val) = block_on(rx.next()) {
            let _ = black_box(val);
        }
    });

    drop(bench_start_tx);

    sender_thread.join().unwrap();
}

fn bench_contended2(c: &mut Criterion) {
    let mut group = c.benchmark_group("__contended2");
    group.throughput(Throughput::Elements(CONTENDED_ITERS as u64));

    let queue_sizes = [1_usize, 16, 128];

    for queue_size in queue_sizes.iter() {
        group.bench_with_input(
            format!("queue=spsc_lock/msg_size=small/queue_size={}", queue_size),
            &queue_size,
            bench_contended2_spsc_lock_inner::<Small>,
        );
    }

    for queue_size in queue_sizes.iter() {
        group.bench_with_input(
            format!("queue=tokio_mpsc/msg_size=small/queue_size={}", queue_size),
            &queue_size,
            bench_contended2_tokio_mpsc_inner::<Small>,
        );
    }

    group.finish();
}

fn bench_uncontended_inner<T, Q>(b: &mut Bencher, queue_size: &&usize)
where
    T: Default + Clone + Send + 'static,
    Q: QueueFactory<T>,
    Q::Sender: 'static,
{
    let queue_size = **queue_size;
    let val = T::default();
    let (bench_start_tx, bench_start_rx) = ::std::sync::mpsc::channel::<Q::Sender>();
    let (send_done_tx, send_done_rx) = ::std::sync::mpsc::channel::<()>();

    let sender_thread = thread::spawn(move || {
        for mut tx in bench_start_rx.iter() {
            for _ in 0..queue_size {
                let _ = block_on(tx.send(val.clone()));
            }

            send_done_tx.send(()).unwrap();
        }
    });

    b.iter(|| {
        let (tx, mut rx) = Q::new(queue_size);
        bench_start_tx.send(tx).unwrap();
        send_done_rx.recv().unwrap();

        for _ in 0..queue_size {
            let _ = black_box(block_on(rx.next()).unwrap());
        }
    });

    drop(bench_start_tx);

    sender_thread.join().unwrap();
}

fn bench_uncontended(c: &mut Criterion) {
    let mut group = c.benchmark_group("uncontended");
    gen_bench_variants!(
        group,
        bench_uncontended_inner,
        |queue_size: &usize| Throughput::Elements(*queue_size as u64),
        // queue sizes
        [128_usize, 512],
        // msg sizes
        // [Small, Medium, Large],
        [Small],
        // queue types
        // [SpscLock<_>, FuturesMpsc<_>, TokioMpsc<_>]
        [SpscLock<_>, TokioMpsc<_>]
    );
    group.finish();
}

// test axis:
// bench x impl x msg size x channel size?

criterion_group!(
    bench,
    bench_send_1_recv_1,
    bench_contended,
    bench_contended2,
    bench_uncontended
);
criterion_main!(bench);
