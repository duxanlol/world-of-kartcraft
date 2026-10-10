//! The demo's audio output: a minimal kira backend over cpal 0.17, the client's cpal (kira's own
//! cpal backend wants 0.18, and both link the platform audio library, which a workspace may link
//! once). The default output device, its default config, f32 samples.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use kira::backend::{Backend, Renderer};

pub struct CpalOut {
    device: cpal::Device,
    config: cpal::StreamConfig,
    stream: Option<cpal::Stream>,
}

impl Backend for CpalOut {
    type Settings = ();
    type Error = String;

    fn setup(_: (), _internal_buffer_size: usize) -> Result<(Self, u32), String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no audio output device")?;
        let config: cpal::StreamConfig = device
            .default_output_config()
            .map_err(|e| e.to_string())?
            .into();
        let rate = config.sample_rate;
        Ok((
            Self {
                device,
                config,
                stream: None,
            },
            rate,
        ))
    }

    fn start(&mut self, mut renderer: Renderer) -> Result<(), String> {
        let channels = self.config.channels;
        let stream = self
            .device
            .build_output_stream(
                &self.config,
                move |out: &mut [f32], _| {
                    renderer.on_start_processing();
                    renderer.process(out, channels);
                },
                |e| eprintln!("kart demo audio: {e}"),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);
        Ok(())
    }
}
