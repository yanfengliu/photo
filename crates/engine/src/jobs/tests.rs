use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use super::*;

const PATIENCE: Duration = Duration::from_secs(10);

type TestPool = Pool<u32, &'static str>;
type TestEvent = Event<u32, &'static str>;

fn pool(threads: usize) -> (TestPool, Receiver<TestEvent>) {
    let (sender, receiver) = mpsc::channel();
    (Pool::start(threads, &sender).unwrap(), receiver)
}

fn job(lane: Lane, value: u32) -> Job<u32, &'static str> {
    Job {
        lane,
        key: None,
        cancel: None,
        work: Box::new(move |_| value),
    }
}

/// A background job that tells `started` when it runs, then blocks until
/// `release` is sent (or the pool stops), so the jobs queued behind it can
/// be ordered deterministically.
fn gate() -> (Job<u32, &'static str>, Receiver<()>, Sender<()>) {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let work: Work<u32, &'static str> = Box::new(move |context| {
        let _ = started_tx.send(());
        while !context.should_stop() {
            if release_rx.recv_timeout(Duration::from_millis(5)).is_ok() {
                break;
            }
        }
        0
    });
    let job = Job {
        lane: Lane::Background,
        key: None,
        cancel: None,
        work,
    };
    (job, started_rx, release_tx)
}

fn finished(receiver: &Receiver<TestEvent>, count: usize) -> Vec<(JobId, u32)> {
    let mut out = Vec::new();
    while out.len() < count {
        match receiver.recv_timeout(PATIENCE).unwrap() {
            Event::Done {
                job,
                outcome: Outcome::Finished(value),
            } => out.push((job, value)),
            Event::Done { .. } => panic!("a job did not finish"),
            Event::Progress { .. } => {}
        }
    }
    out
}

#[test]
fn interactive_jobs_run_newest_first_and_before_background_jobs() {
    let (mut pool, events) = pool(1);
    let (blocker, started, release) = gate();
    pool.submit(blocker);
    started.recv_timeout(PATIENCE).unwrap();

    pool.submit(job(Lane::Background, 1));
    pool.submit(job(Lane::Interactive, 2));
    pool.submit(job(Lane::Background, 3));
    pool.submit(job(Lane::Interactive, 4));
    assert_eq!(pool.queued(), 4);
    release.send(()).unwrap();

    let order: Vec<u32> = finished(&events, 5)
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    assert_eq!(order, [0, 4, 2, 1, 3]);
}

#[test]
fn a_newer_request_with_the_same_key_replaces_the_queued_one() {
    let (mut pool, events) = pool(1);
    let (blocker, started, release) = gate();
    pool.submit(blocker);
    started.recv_timeout(PATIENCE).unwrap();

    let key = JobKey::Candidate(PathBuf::from("/a.jpg"));
    let keyed = |value| Job {
        key: Some(key.clone()),
        ..job(Lane::Interactive, value)
    };
    let first = pool.submit(keyed(1));
    assert!(first.replaced.is_none());
    let second = pool.submit(keyed(2));
    assert_eq!(second.replaced, Some(first.id));
    assert!(pool.promote(&key));
    release.send(()).unwrap();

    let done = finished(&events, 2);
    assert_eq!(done[1], (second.id, 2));
    assert_eq!(pool.queued(), 0);
}

#[test]
fn progress_arrives_before_the_outcome() {
    let (mut pool, events) = pool(2);
    pool.submit(Job {
        lane: Lane::Background,
        key: None,
        cancel: None,
        work: Box::new(|context| {
            context.progress("half way");
            7
        }),
    });
    match events.recv_timeout(PATIENCE).unwrap() {
        Event::Progress { progress, .. } => assert_eq!(progress, "half way"),
        Event::Done { .. } => panic!("the outcome overtook the progress"),
    }
    assert_eq!(finished(&events, 1)[0].1, 7);
}

#[test]
fn a_panicking_job_is_reported_and_the_worker_carries_on() {
    let (mut pool, events) = pool(1);
    pool.submit(Job {
        lane: Lane::Background,
        key: None,
        cancel: None,
        work: Box::new(|_| panic!("decoder exploded")),
    });
    pool.submit(job(Lane::Background, 9));
    match events.recv_timeout(PATIENCE).unwrap() {
        Event::Done {
            outcome: Outcome::Panicked(message),
            ..
        } => assert!(message.contains("decoder exploded"), "{message}"),
        _ => panic!("the panic was not reported"),
    }
    assert_eq!(finished(&events, 1)[0].1, 9);
}

#[test]
fn queued_jobs_of_a_cancelled_task_are_skipped() {
    let (mut pool, events) = pool(1);
    let (blocker, started, release) = gate();
    pool.submit(blocker);
    started.recv_timeout(PATIENCE).unwrap();

    let token = CancelToken::default();
    let ran = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&ran);
    pool.submit(Job {
        lane: Lane::Background,
        key: None,
        cancel: Some(token.clone()),
        work: Box::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            1
        }),
    });
    token.cancel();
    release.send(()).unwrap();

    assert_eq!(finished(&events, 1)[0].1, 0);
    match events.recv_timeout(PATIENCE).unwrap() {
        Event::Done {
            outcome: Outcome::Skipped,
            ..
        } => {}
        _ => panic!("the cancelled job was not skipped"),
    }
    assert_eq!(ran.load(Ordering::SeqCst), 0);
}

#[test]
fn the_wake_callback_runs_after_each_event_and_a_panic_in_it_is_contained() {
    let (mut pool, events) = pool(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    pool.set_wake(Some(Arc::new(move || {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("first wake fails");
        }
    })));
    pool.submit(job(Lane::Background, 1));
    pool.submit(job(Lane::Background, 2));
    finished(&events, 2);
    // The callback runs after the event is sent, so the second call can
    // still be on its way when the second event is received.
    let deadline = Instant::now() + PATIENCE;
    while calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn dropping_the_pool_abandons_queued_jobs_and_joins_every_thread() {
    let (mut pool, _events) = pool(3);
    let (blocker, started, _release) = gate();
    pool.submit(blocker);
    started.recv_timeout(PATIENCE).unwrap();
    let ran = Arc::new(AtomicUsize::new(0));
    for _ in 0..50 {
        let counted = Arc::clone(&ran);
        pool.submit(Job {
            lane: Lane::Background,
            key: None,
            cancel: None,
            work: Box::new(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                1
            }),
        });
    }
    let shared = Arc::downgrade(&pool.shared);

    let start = Instant::now();
    drop(pool);
    assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
    assert!(shared.upgrade().is_none(), "a worker thread outlived the pool");
    assert!(ran.load(Ordering::SeqCst) < 50);
}
