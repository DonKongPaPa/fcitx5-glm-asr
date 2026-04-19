pub fn resample_to_mono_s16le(
    samples: &[f32],
    source_rate: u32,
    source_channels: u16,
    target_rate: u32,
) -> Vec<i16> {
    let mono = if source_channels > 1 {
        samples
            .chunks_exact(source_channels as usize)
            .map(|chunk| {
                let sum: f32 = chunk.iter().sum();
                sum / source_channels as f32
            })
            .collect::<Vec<f32>>()
    } else {
        samples.to_vec()
    };

    let ratio = target_rate as f64 / source_rate as f64;
    let input_len = mono.len();
    let output_len = ((input_len as f64) * ratio) as usize;

    let mut output = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let src_pos = (i as f64) / ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;

        let s0 = mono.get(idx).copied().unwrap_or(0.0f32);
        let s1 = mono.get(idx + 1).copied().unwrap_or(0.0f32);
        let interpolated = s0 + (s1 - s0) * frac as f32;

        let clamped = interpolated.clamp(-1.0, 1.0);
        output.push((clamped * 32767.0) as i16);
    }

    output
}

pub fn samples_to_wav(
    samples: &[i16],
    sample_rate: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut buf, spec)?;
        let mut sample_writer = writer.get_i16_writer(samples.len() as u32);
        for &s in samples {
            sample_writer.write_sample(s);
        }
        sample_writer.flush()?;
        writer.finalize()?;
    }

    Ok(buf.into_inner())
}
