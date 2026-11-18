//! Device creation and adapter selection.
//!
//! The same code path serves a GPU, WARP (Windows) and lavapipe (Linux): when
//! `prefer_software` is set, a CPU adapter is requested first and the actual
//! adapter is reported so CI can prove which one rendered the frame.

use serde::{Deserialize, Serialize};

use crate::RenderError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterReport {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub software: bool,
}

impl AdapterReport {
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} · {} · {} · driver {}",
            self.name, self.backend, self.device_type, self.driver
        )
    }
}

#[derive(Debug)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: AdapterReport,
}

impl Gpu {
    pub fn new(prefer_software: bool) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(async {
            if prefer_software {
                if let Ok(adapter) = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::LowPower,
                        force_fallback_adapter: true,
                        compatible_surface: None,
                    })
                    .await
                {
                    return Ok(adapter);
                }
                // Some backends do not mark software adapters as fallbacks
                // (WARP and lavapipe are ordinary adapters there); pick one by
                // its device type instead.
                let adapters = instance.enumerate_adapters(wgpu::Backends::PRIMARY).await;
                for adapter in adapters {
                    if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
                        return Ok(adapter);
                    }
                }
            }
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
        })?;
        let info = adapter.get_info();
        let report = AdapterReport {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            driver: info.driver.clone(),
            software: info.device_type == wgpu::DeviceType::Cpu,
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("oxide-render"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            }))?;
        device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
            log::error!("wgpu: {error}");
        }));
        Ok(Self {
            device,
            queue,
            adapter: report,
        })
    }
}
