//! The GPU the harness renders with: real hardware first.
//!
//! `photo_ui::harness` prefers a software rasterizer by default, which keeps
//! snapshot tests alike across machines. The agent harness wants the frames
//! the user's GPU makes, and makes them fast, so it ranks discrete GPUs
//! first, then integrated ones, and a CPU rasterizer last.

use std::sync::{Arc, Mutex, PoisonError};

use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew, wgpu};
use serde_json::{Value, json};

/// The adapter that was chosen, for `ping` to report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterChoice {
    /// The adapter's name, as its driver gives it.
    pub name: String,
    /// The graphics API used (Metal, Vulkan, Dx12, Gl).
    pub backend: String,
    /// Discrete, integrated, virtual, CPU or other.
    pub device_type: String,
}

impl AdapterChoice {
    /// As `ping` reports it.
    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "backend": self.backend, "device_type": self.device_type })
    }
}

/// Where the choice is written once wgpu has made it.
pub type ChoiceSlot = Arc<Mutex<Option<AdapterChoice>>>;

/// A wgpu setup that picks the best hardware adapter and writes which one
/// it picked into `slot`.
pub fn hardware_first(slot: ChoiceSlot) -> WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    // Frames are read back synchronously, which WebGPU cannot do.
    setup
        .instance_descriptor
        .backends
        .remove(wgpu::Backends::BROWSER_WEBGPU);
    setup.native_adapter_selector = Some(Arc::new(move |adapters, _surface| {
        let best = adapters
            .iter()
            .min_by_key(|adapter| {
                let info = adapter.get_info();
                rank(info.device_type, info.backend)
            })
            .cloned()
            .ok_or_else(|| "no GPU adapter was found".to_owned())?;
        let info = best.get_info();
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(AdapterChoice {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
        });
        Ok(best)
    }));
    WgpuSetup::CreateNew(setup)
}

/// Lower is better: hardware before software, then the native APIs before
/// OpenGL.
fn rank(device: wgpu::DeviceType, backend: wgpu::Backend) -> (u8, u8) {
    let device = match device {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => 2,
        wgpu::DeviceType::Cpu => 3,
    };
    let backend = match backend {
        wgpu::Backend::Metal => 0,
        wgpu::Backend::Vulkan => 1,
        wgpu::Backend::Dx12 => 2,
        wgpu::Backend::Gl => 4,
        wgpu::Backend::BrowserWebGpu => 6,
        wgpu::Backend::Noop => 7,
    };
    (device, backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_is_ranked_above_a_software_rasterizer() {
        use wgpu::{Backend, DeviceType};
        let discrete = rank(DeviceType::DiscreteGpu, Backend::Vulkan);
        let integrated = rank(DeviceType::IntegratedGpu, Backend::Metal);
        let software = rank(DeviceType::Cpu, Backend::Vulkan);
        assert!(discrete < integrated && integrated < software);
        assert!(
            rank(DeviceType::IntegratedGpu, Backend::Dx12)
                < rank(DeviceType::IntegratedGpu, Backend::Gl)
        );
    }
}
