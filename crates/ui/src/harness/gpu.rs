//! Finding a GPU to render with, without panicking when there is none.
//!
//! `egui_kittest` creates its wgpu renderer with an `expect`, so a machine
//! with no usable adapter (a bare CI container) would abort the whole run.
//! Here the render state is created up front and any failure becomes a value
//! the harness can report from [`super::Harness::render`].

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use eframe::egui_wgpu::{RenderState, RendererOptions, WgpuConfiguration, WgpuSetup};
use egui_kittest::TestRenderer;

/// Try to create the wgpu state the test renderer draws with.
///
/// Returns the reason as text when no adapter or device can be had.
pub(super) fn create_render_state(setup: WgpuSetup) -> Result<RenderState, String> {
    let instance = block_on(setup.new_instance());
    let config = WgpuConfiguration {
        wgpu_setup: setup,
        ..Default::default()
    };
    block_on(RenderState::create(
        &config,
        &instance,
        None,
        RendererOptions::PREDICTABLE,
    ))
    .map_err(|err| err.to_string())
}

/// The renderer used when there is no GPU: it keeps the harness running (all
/// interaction and accessibility queries still work) and refuses to render.
pub(super) struct NullRenderer;

impl TestRenderer for NullRenderer {
    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        delta.clear();
    }

    fn render(
        &mut self,
        _ctx: &egui::Context,
        _output: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        Err("no GPU adapter is available".to_owned())
    }
}

/// Drive a future to completion on the current thread.
///
/// wgpu's native adapter and device requests resolve without an executor, so
/// parking between polls is all that is needed.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_returns_the_value_of_a_ready_future() {
        assert_eq!(block_on(async { 6 * 7 }), 42);
    }

    #[test]
    fn block_on_waits_for_a_future_woken_from_another_thread() {
        let (sender, receiver) = std::sync::mpsc::channel::<u8>();
        let shared = Arc::new(std::sync::Mutex::new(None::<Waker>));
        let for_thread = Arc::clone(&shared);
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            sender.send(9).unwrap();
            if let Some(waker) = for_thread.lock().unwrap().take() {
                waker.wake();
            }
        });
        let value = block_on(std::future::poll_fn(|context| match receiver.try_recv() {
            Ok(value) => Poll::Ready(value),
            Err(_) => {
                *shared.lock().unwrap() = Some(context.waker().clone());
                // Close the race with the sender: it may have sent between the
                // failed receive and the waker being stored.
                match receiver.try_recv() {
                    Ok(value) => Poll::Ready(value),
                    Err(_) => Poll::Pending,
                }
            }
        }));
        worker.join().unwrap();
        assert_eq!(value, 9);
    }
}
