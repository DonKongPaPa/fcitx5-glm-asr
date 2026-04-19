use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info};

pub struct AudioCapture {
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<Stream>,
    sample_rate: u32,
    channels: u16,
}

impl AudioCapture {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            samples: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            sample_rate: 0,
            channels: 0,
        })
    }

    pub fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let supported_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;

        info!(
            "Audio device: {}, format: {:?}",
            device
                .description()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            supported_config
        );

        let config = supported_config.config();
        self.sample_rate = config.sample_rate;
        self.channels = config.channels;

        let samples = self.samples.clone();
        samples.lock().unwrap().clear();

        let stream = match supported_config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut buf) = samples.lock() {
                        buf.extend_from_slice(data);
                    }
                },
                |err| error!("Audio capture error: {}", err),
                None,
            )?,
            SampleFormat::I16 => {
                let samples_i16 = self.samples.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = samples_i16.lock() {
                            buf.extend(data.iter().map(|&s| s as f32 / 32768.0));
                        }
                    },
                    |err| error!("Audio capture error: {}", err),
                    None,
                )?
            }
            SampleFormat::U16 => {
                let samples_u16 = self.samples.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = samples_u16.lock() {
                            buf.extend(data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0));
                        }
                    },
                    |err| error!("Audio capture error: {}", err),
                    None,
                )?
            }
            fmt => return Err(format!("Unsupported sample format: {:?}", fmt).into()),
        };

        stream.play()?;
        self.stream = Some(stream);
        info!(
            "Audio capture started ({}Hz, {}ch)",
            self.sample_rate, self.channels
        );
        Ok(())
    }

    pub fn stop(&mut self) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        if let Some(stream) = self.stream.take() {
            drop(stream);
        }
        let samples = self.samples.lock().unwrap();
        let result = samples.clone();
        debug!("Captured {} samples", result.len());
        Ok(result)
    }

    pub fn is_recording(&self) -> bool {
        self.stream.is_some()
    }

    pub fn source_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn source_channels(&self) -> u16 {
        self.channels
    }
}
