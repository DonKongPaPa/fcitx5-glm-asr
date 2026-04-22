use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info};

pub struct AudioCapture {
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<Stream>,
    sample_rate: u32,
    channels: u16,
    volume: Arc<Mutex<f32>>,
    prev_waveform: Mutex<Vec<f32>>,
}

impl AudioCapture {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            samples: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            sample_rate: 0,
            channels: 0,
            volume: Arc::new(Mutex::new(0.0)),
            prev_waveform: Mutex::new(Vec::new()),
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

        let volume = self.volume.clone();
        *volume.lock().unwrap() = 0.0;

        let stream = match supported_config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut buf) = samples.lock() {
                        buf.extend_from_slice(data);
                    }
                    let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt();
                    if let Ok(mut v) = volume.lock() {
                        *v = rms;
                    }
                },
                |err| error!("Audio capture error: {}", err),
                None,
            )?,
            SampleFormat::I16 => {
                let samples_i16 = self.samples.clone();
                let vol_i16 = self.volume.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let converted: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                        if let Ok(mut buf) = samples_i16.lock() {
                            buf.extend(&converted);
                        }
                        let rms = (converted.iter().map(|s| s * s).sum::<f32>() / converted.len() as f32).sqrt();
                        if let Ok(mut v) = vol_i16.lock() {
                            *v = rms;
                        }
                    },
                    |err| error!("Audio capture error: {}", err),
                    None,
                )?
            }
            SampleFormat::U16 => {
                let samples_u16 = self.samples.clone();
                let vol_u16 = self.volume.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let converted: Vec<f32> = data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect();
                        if let Ok(mut buf) = samples_u16.lock() {
                            buf.extend(&converted);
                        }
                        let rms = (converted.iter().map(|s| s * s).sum::<f32>() / converted.len() as f32).sqrt();
                        if let Ok(mut v) = vol_u16.lock() {
                            *v = rms;
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

    pub fn current_volume(&self) -> f32 {
        *self.volume.lock().unwrap()
    }

    pub fn current_waveform(&self, target_samples: usize) -> Vec<f32> {
        let samples = self.samples.lock().unwrap();
        if samples.is_empty() {
            return Vec::new();
        }
        let len = samples.len();
        let recent = samples.len().min((self.sample_rate as usize).max(8000));
        let start = len.saturating_sub(recent);
        let window_size = (recent / target_samples).max(1);
        let mut result = Vec::with_capacity(target_samples);
        for chunk_start in (start..len).step_by(window_size) {
            let chunk_end = (chunk_start + window_size).min(len);
            let chunk = &samples[chunk_start..chunk_end];
            let peak = chunk.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            let mapped = if peak < 0.008 { 0.0 } else { peak };
            result.push(mapped);
            if result.len() >= target_samples {
                break;
            }
        }
        let min_val = result.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_val = result.iter().cloned().fold(0.0f32, f32::max);
        let range = max_val - min_val;
        if range > 0.002 {
            for v in &mut result {
                *v = (*v - min_val) / range;
            }
        } else {
            result.clear();
        }
        if result.len() >= 3 {
            let orig = result.clone();
            for i in 1..orig.len() - 1 {
                result[i] = (orig[i - 1] + orig[i] + orig[i + 1]) / 3.0;
            }
        }
        let mut prev = self.prev_waveform.lock().unwrap();
        if prev.len() == result.len() {
            for i in 0..result.len() {
                result[i] = prev[i] * 0.7 + result[i] * 0.3;
            }
        }
        *prev = result.clone();
        result
    }
}
