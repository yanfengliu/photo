//! The develop session's render worker: one thread that always works on the
//! newest request.
//!
//! Each purpose (the canvas, the before view, the preset hover preview) has
//! one waiting slot. A new request takes the slot, dropping the one waiting
//! there, and cancels the render of the same purpose that is running, which
//! photo-develop abandons within milliseconds. The canvas goes first. Each
//! purpose keeps a renderer of its own, so moving a slider reuses the stage
//! caches of the canvas renders before it.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::DevelopSettings;
use photo_core::image::Rgba8Image;
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer, Source};
use photo_imaging::{Histogram, View};

use crate::jobs::lock;

/// What a render is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Purpose {
    /// The canvas: the photo as edited.
    Main,
    /// The before view.
    Before,
    /// The preset hover preview.
    Look,
}

impl Purpose {
    /// In the order the worker serves them.
    const ALL: [Self; 3] = [Self::Main, Self::Before, Self::Look];

    fn index(self) -> usize {
        match self {
            Self::Main => 0,
            Self::Before => 1,
            Self::Look => 2,
        }
    }
}

/// One render to make.
pub(crate) struct Order {
    pub(crate) id: u64,
    pub(crate) purpose: Purpose,
    /// The session the render belongs to.
    pub(crate) generation: u64,
    pub(crate) source: Source,
    pub(crate) settings: DevelopSettings,
    /// The settings version the render shows.
    pub(crate) version: u64,
    pub(crate) view: View,
    pub(crate) quality: Quality,
}

/// A finished render.
pub(crate) struct Frame {
    pub(crate) image: Rgba8Image,
    pub(crate) histogram: Histogram,
    pub(crate) stage_times: Vec<(&'static str, Duration)>,
    pub(crate) render_time: Duration,
}

/// How an order ended.
pub(crate) enum Outcome {
    /// Boxed: the histogram makes a frame large.
    Rendered(Box<Frame>),
    /// A newer order of the same purpose stopped it.
    Cancelled,
    /// The renderer panicked, with this message.
    Failed(String),
}

/// What the worker hands back for every order it took.
pub(crate) struct Delivery {
    pub(crate) id: u64,
    pub(crate) purpose: Purpose,
    pub(crate) generation: u64,
    pub(crate) version: u64,
    pub(crate) view: View,
    pub(crate) quality: Quality,
    pub(crate) outcome: Outcome,
}

struct Running {
    purpose: Purpose,
    cancel: CancelToken,
}

struct Slots {
    waiting: [Option<Order>; 3],
    running: Option<Running>,
    shutdown: bool,
}

struct Mailbox {
    slots: Mutex<Slots>,
    ready: Condvar,
}

/// The worker thread and its mailbox.
pub(crate) struct RenderWorker {
    mailbox: Arc<Mailbox>,
    thread: Option<JoinHandle<()>>,
}

impl RenderWorker {
    /// Start the thread. `deliver` is called on it once for every order it
    /// takes, after the render has ended one way or another.
    pub(crate) fn start(deliver: impl Fn(Delivery) + Send + 'static) -> io::Result<Self> {
        let mailbox = Arc::new(Mailbox {
            slots: Mutex::new(Slots {
                waiting: [None, None, None],
                running: None,
                shutdown: false,
            }),
            ready: Condvar::new(),
        });
        let shared = Arc::clone(&mailbox);
        let thread = thread::Builder::new()
            .name("photo-engine-render".to_string())
            .spawn(move || serve(&shared, &deliver))?;
        Ok(Self {
            mailbox,
            thread: Some(thread),
        })
    }

    /// Queue an order. The order of the same purpose waiting is dropped and
    /// its id returned (it will never be delivered); the render of the same
    /// purpose running is cancelled.
    pub(crate) fn order(&self, order: Order) -> Option<u64> {
        let dropped = {
            let mut slots = lock(&self.mailbox.slots);
            if let Some(running) = &slots.running
                && running.purpose == order.purpose
            {
                running.cancel.cancel();
            }
            let slot = order.purpose.index();
            slots.waiting[slot].replace(order).map(|order| order.id)
        };
        self.mailbox.ready.notify_one();
        dropped
    }

    /// Drop every waiting order and cancel the running render; returns the
    /// ids of the orders dropped.
    pub(crate) fn clear(&self) -> Vec<u64> {
        let mut slots = lock(&self.mailbox.slots);
        if let Some(running) = &slots.running {
            running.cancel.cancel();
        }
        slots
            .waiting
            .iter_mut()
            .filter_map(|slot| slot.take().map(|order| order.id))
            .collect()
    }
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        {
            let mut slots = lock(&self.mailbox.slots);
            slots.shutdown = true;
            if let Some(running) = &slots.running {
                running.cancel.cancel();
            }
            slots.waiting = [None, None, None];
        }
        self.mailbox.ready.notify_all();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("the render worker ended with a panic");
        }
    }
}

fn serve(mailbox: &Mailbox, deliver: &impl Fn(Delivery)) {
    let mut renderers = [Renderer::new(), Renderer::new(), Renderer::new()];
    while let Some((order, cancel)) = next(mailbox) {
        let renderer = &mut renderers[order.purpose.index()];
        let outcome = render(renderer, &order, &cancel);
        if matches!(outcome, Outcome::Failed(_)) {
            // A panic may have left the caches half-written.
            *renderer = Renderer::new();
        }
        lock(&mailbox.slots).running = None;
        deliver(Delivery {
            id: order.id,
            purpose: order.purpose,
            generation: order.generation,
            version: order.version,
            view: order.view,
            quality: order.quality,
            outcome,
        });
    }
}

/// Wait for the next order, canvas first; `None` at shutdown.
fn next(mailbox: &Mailbox) -> Option<(Order, CancelToken)> {
    let mut slots = lock(&mailbox.slots);
    loop {
        if slots.shutdown {
            return None;
        }
        let ready = Purpose::ALL
            .into_iter()
            .find_map(|purpose| slots.waiting[purpose.index()].take());
        if let Some(order) = ready {
            let cancel = CancelToken::new();
            slots.running = Some(Running {
                purpose: order.purpose,
                cancel: cancel.clone(),
            });
            return Some((order, cancel));
        }
        slots = mailbox
            .ready
            .wait(slots)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

fn render(renderer: &mut Renderer, order: &Order, cancel: &CancelToken) -> Outcome {
    let request = RenderRequest {
        source: &order.source,
        settings: &order.settings,
        view: order.view,
        quality: order.quality,
        space: RgbSpace::Srgb,
    };
    let start = Instant::now();
    match catch_unwind(AssertUnwindSafe(|| renderer.render_rgba8(&request, cancel))) {
        Ok(Some(image)) => Outcome::Rendered(Box::new(Frame {
            histogram: photo_develop::histogram(&image),
            stage_times: renderer.last_stage_times().to_vec(),
            render_time: start.elapsed(),
            image,
        })),
        Ok(None) => Outcome::Cancelled,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no further detail".to_string());
            log::error!("a develop render panicked: {message}");
            Outcome::Failed(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use photo_core::image::{SourceColor, SourceImage};

    use super::*;

    const PATIENCE: Duration = Duration::from_secs(30);

    fn source(width: u32, height: u32) -> Source {
        let pixels = (0..width * height * 3)
            .map(|i| (i * 97 % 65_536) as u16)
            .collect();
        let color = SourceColor::Rgb(RgbSpace::Srgb);
        Source::new(SourceImage::new(width, height, pixels, color).unwrap())
    }

    fn order(id: u64, purpose: Purpose, source: &Source, exposure: f32) -> Order {
        let mut settings = DevelopSettings::default();
        settings.tone.exposure = exposure;
        let view = photo_develop::fit_view(source, &settings, 64, 64);
        Order {
            id,
            purpose,
            generation: 1,
            source: source.clone(),
            settings,
            version: id,
            view,
            quality: Quality::Draft,
        }
    }

    #[test]
    fn the_newest_order_of_a_purpose_wins_and_every_taken_order_is_delivered() {
        let (sender, deliveries) = mpsc::channel();
        let worker = RenderWorker::start(move |delivery| {
            let _ = sender.send(delivery);
        })
        .unwrap();
        let photo = source(640, 480);
        let mut dropped = Vec::new();
        for id in 1..=20 {
            dropped.extend(worker.order(order(id, Purpose::Main, &photo, id as f32 / 10.0)));
        }
        worker.order(order(21, Purpose::Before, &photo, 0.0));

        let mut seen = Vec::new();
        let mut rendered = Vec::new();
        while !(seen.contains(&20) && seen.contains(&21)) {
            let delivery = deliveries.recv_timeout(PATIENCE).unwrap();
            seen.push(delivery.id);
            if let Outcome::Rendered(frame) = delivery.outcome {
                assert_eq!(frame.image.width, delivery.view.width());
                assert_eq!(
                    frame.histogram.pixel_count(),
                    u64::from(frame.image.width * frame.image.height)
                );
                rendered.push(delivery.id);
            }
        }
        assert!(rendered.contains(&20) && rendered.contains(&21));
        assert!(!dropped.is_empty(), "waiting orders were replaced");
        for id in &dropped {
            assert!(!seen.contains(id), "a dropped order was delivered");
        }
        assert_eq!(
            seen.len() + dropped.len(),
            21,
            "every order is delivered or dropped"
        );
    }

    #[test]
    fn clearing_drops_what_waits_and_dropping_the_worker_joins_it() {
        let (sender, _deliveries) = mpsc::channel();
        let worker = RenderWorker::start(move |delivery| {
            let _ = sender.send(delivery);
        })
        .unwrap();
        let photo = source(32, 32);
        worker.order(order(1, Purpose::Look, &photo, 0.0));
        let cleared = worker.clear();
        assert!(cleared.len() <= 1);
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
