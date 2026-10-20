//! 声音闪避的纯 Rust 信号处理；固定 32 kHz / 四阶 1 kHz 高通。
use anyhow::{Result, ensure};
use num_complex::Complex64;
use std::f64::consts::PI;

pub const SAMPLE_RATE: u32 = 32_000;
pub const WINDOW_SAMPLES: usize = 16_000;

/// 只接收有界 RIFF PCM16 / PCM24 / PCM32 / Float32 WAV，不调用外部解码器。
pub fn read_wav(bytes: &[u8]) -> Result<Vec<f64>> {
    ensure!(
        (44..=16 * 1024 * 1024).contains(&bytes.len())
            && &bytes[..4] == b"RIFF"
            && &bytes[8..12] == b"WAVE",
        "不是有界 RIFF WAV"
    );
    let u16_at = |i: usize| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    ensure!(u32_at(4) as usize + 8 == bytes.len(), "WAV 长度不匹配");
    let mut format = None;
    let mut data = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let size = u32_at(pos + 4) as usize;
        let begin = pos + 8;
        ensure!(size <= bytes.len() - begin, "WAV 数据块截断");
        match &bytes[pos..pos + 4] {
            b"fmt " => {
                ensure!(format.is_none() && size >= 16, "WAV 格式块重复或截断");
                format = Some((
                    u16_at(begin),
                    u16_at(begin + 2),
                    u32_at(begin + 4),
                    u16_at(begin + 12),
                    u16_at(begin + 14),
                ));
            }
            b"data" => {
                ensure!(data.is_none(), "WAV 数据块重复");
                data = Some(&bytes[begin..begin + size]);
            }
            _ => {}
        }
        pos = begin + size + size % 2;
    }
    let (kind, channels, rate, align, bits) =
        format.ok_or_else(|| anyhow::anyhow!("WAV 缺失格式"))?;
    let data = data.ok_or_else(|| anyhow::anyhow!("WAV 缺失数据"))?;
    ensure!(
        (1..=32).contains(&channels)
            && [16, 24, 32].contains(&bits)
            && ((kind == 1) || (kind == 3 && bits == 32)),
        "不支持的 WAV 格式"
    );
    ensure!(
        align == channels * bits / 8 && data.len().is_multiple_of(align as usize),
        "WAV 帧尺寸不匹配"
    );
    let samples: Vec<f32> = data
        .chunks_exact((bits / 8) as usize)
        .map(|b| match (kind, bits) {
            (3, 32) => f32::from_le_bytes(b.try_into().unwrap()),
            (_, 16) => i16::from_le_bytes(b.try_into().unwrap()) as f32 / 32768.0,
            (_, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) as f64 / 2147483648.0) as f32,
            _ => (i32::from_le_bytes(b.try_into().unwrap()) as f64 / 2147483648.0) as f32,
        })
        .collect();
    ensure!(
        samples.len() <= rate as usize * channels as usize,
        "声音模板超过一秒"
    );
    resample_mono(&samples, channels as usize, rate)
}

/// 带限窗化 sinc 重采样；坐标和窗口固定，便于回放。
pub fn resample_mono(interleaved: &[f32], channels: usize, rate: u32) -> Result<Vec<f64>> {
    ensure!(
        (1..=32).contains(&channels) && (8_000..=384_000).contains(&rate),
        "音频通道数或采样率无效"
    );
    ensure!(
        interleaved.len().is_multiple_of(channels) && interleaved.len() <= 32 * 384_000,
        "PCM 长度无效或超过一秒"
    );
    ensure!(
        interleaved.iter().all(|s| s.is_finite()),
        "PCM 包含非有限采样"
    );
    let mono: Vec<f64> = interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().map(|s| *s as f64).sum::<f64>() / channels as f64)
        .collect();
    if rate == SAMPLE_RATE {
        return Ok(mono);
    }
    let output = mono.len() as u64 * SAMPLE_RATE as u64 / rate as u64;
    let cutoff = (SAMPLE_RATE as f64 / rate as f64).min(1.0);
    let mut result = Vec::with_capacity(output as usize);
    for index in 0..output {
        let position = index as f64 * rate as f64 / SAMPLE_RATE as f64;
        let center = position.floor() as isize;
        let mut sum = 0.0;
        let mut norm = 0.0;
        for offset in -32..=32 {
            let source = center + offset;
            if source < 0 || source >= mono.len() as isize {
                continue;
            }
            let distance = position - source as f64;
            if distance.abs() > 32.0 {
                continue;
            }
            let x = PI * distance * cutoff;
            let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
            let window = 0.42
                + 0.5 * (PI * distance / 32.0).cos()
                + 0.08 * (2.0 * PI * distance / 32.0).cos();
            let weight = sinc * window * cutoff;
            sum += mono[source as usize] * weight;
            norm += weight;
        }
        result.push(if norm.abs() > 1e-12 { sum / norm } else { 0.0 });
    }
    Ok(result)
}

pub fn highpass_coefficients() -> ([f64; 5], [f64; 5]) {
    let warped = (PI * 1000.0 / SAMPLE_RATE as f64).tan();
    let mut coefficients = vec![Complex64::new(1.0, 0.0)];
    for index in 0..4 {
        let angle = PI * (2 * index + 5) as f64 / 8.0;
        let analog = warped / Complex64::from_polar(1.0, angle);
        let pole = (1.0 + analog) / (1.0 - analog);
        let mut next = vec![Complex64::new(0.0, 0.0); coefficients.len() + 1];
        for (i, value) in coefficients.iter().enumerate() {
            next[i] += value;
            next[i + 1] -= value * pole;
        }
        coefficients = next;
    }
    // 高通在 Nyquist 的增益为一。
    let a: [f64; 5] = std::array::from_fn(|i| coefficients[i].re);
    let k = a
        .iter()
        .enumerate()
        .map(|(i, a)| a * if i % 2 == 0 { 1.0 } else { -1.0 })
        .sum::<f64>()
        / 16.0;
    let b = [k, -4.0 * k, 6.0 * k, -4.0 * k, k];
    (b, a)
}

pub fn filter_wave(input: &[f64]) -> Result<Vec<f64>> {
    ensure!(
        input.len() > 15 && input.len() <= 320_000 && input.iter().all(|x| x.is_finite()),
        "音频滤波需要 16..320000 个有限样本"
    );
    let (b, a) = highpass_coefficients();
    let mut padded = Vec::with_capacity(input.len() + 30);
    padded.extend((1..=15).rev().map(|i| 2.0 * input[0] - input[i]));
    padded.extend_from_slice(input);
    padded.extend(
        (input.len() - 16..input.len() - 1)
            .rev()
            .map(|i| 2.0 * input[input.len() - 1] - input[i]),
    );
    let steady = b.iter().sum::<f64>() / a.iter().sum::<f64>();
    let mut zi = [0.0; 4];
    zi[0] = steady - b[0];
    for i in 1..4 {
        zi[i] = zi[i - 1] - b[i] + a[i] * steady;
    }
    fn pass(data: &mut [f64], b: &[f64; 5], a: &[f64; 5], zi: [f64; 4]) {
        let mut state = zi.map(|v| v * data[0]);
        for value in data {
            let x = *value;
            let y = b[0] * x + state[0];
            for i in 0..3 {
                state[i] = b[i + 1] * x + state[i + 1] - a[i + 1] * y;
            }
            state[3] = b[4] * x - a[4] * y;
            *value = y;
        }
    }
    pass(&mut padded, &b, &a, zi);
    padded.reverse();
    pass(&mut padded, &b, &a, zi);
    padded.reverse();
    Ok(padded[15..15 + input.len()].to_vec())
}

fn standardize(input: &[f64]) -> Vec<f64> {
    let mean = input.iter().sum::<f64>() / input.len() as f64;
    let deviation =
        (input.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / input.len() as f64).sqrt();
    let divisor = if deviation < 10.0 * f64::EPSILON {
        1.0
    } else {
        deviation
    };
    input.iter().map(|x| x / divisor).collect()
}

fn fft(values: &mut [Complex64], inverse: bool) {
    let n = values.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            values.swap(i, j);
        }
    }
    let mut size = 2;
    while size <= n {
        let root = Complex64::from_polar(1.0, if inverse { 2.0 } else { -2.0 } * PI / size as f64);
        for start in (0..n).step_by(size) {
            let mut factor = Complex64::new(1.0, 0.0);
            for offset in 0..size / 2 {
                let even = values[start + offset];
                let odd = values[start + offset + size / 2] * factor;
                values[start + offset] = even + odd;
                values[start + offset + size / 2] = even - odd;
                factor *= root;
            }
        }
        size *= 2;
    }
    if inverse {
        for value in values {
            *value /= n as f64;
        }
    }
}

/// 对齐旧 correlate(max_length, min_length, mode='same')，分母始终为模板长度。
pub fn max_correlation(filtered_template: &[f64], audio: &[f64]) -> Result<f64> {
    ensure!(
        !filtered_template.is_empty()
            && filtered_template.len() <= 320_000
            && filtered_template.iter().all(|x| x.is_finite()),
        "声音模板无效"
    );
    let template = standardize(filtered_template);
    let wave = standardize(&filter_wave(audio)?);
    let (long, short) = if template.len() > wave.len() {
        (&template, &wave)
    } else {
        (&wave, &template)
    };
    let n = (long.len() + short.len() - 1).next_power_of_two();
    let mut a = vec![Complex64::new(0.0, 0.0); n];
    let mut b = a.clone();
    for (to, from) in a.iter_mut().zip(long) {
        to.re = *from;
    }
    for (to, from) in b.iter_mut().zip(short.iter().rev()) {
        to.re = *from;
    }
    fft(&mut a, false);
    fft(&mut b, false);
    for (a, b) in a.iter_mut().zip(b) {
        *a *= b;
    }
    fft(&mut a, true);
    let start = (short.len() - 1) / 2;
    let value = a[start..start + long.len()]
        .iter()
        .map(|v| v.re / template.len() as f64)
        .fold(f64::NEG_INFINITY, f64::max);
    ensure!(value.is_finite(), "声音相关性计算溢出");
    Ok(value)
}

pub struct AudioDodge {
    template: Vec<f64>,
    window: Vec<f64>,
    pub threshold: f64,
    last_event: Option<f64>,
}
impl AudioDodge {
    pub fn new(template: &[f64], threshold: f64) -> Result<Self> {
        ensure!(
            threshold.is_finite() && (0.0..=10.0).contains(&threshold),
            "声音阈值无效"
        );
        Ok(Self {
            template: filter_wave(template)?,
            window: vec![0.0; WINDOW_SAMPLES],
            threshold,
            last_event: None,
        })
    }
    pub fn reset(&mut self) {
        self.window.fill(0.0);
        self.last_event = None;
    }
    pub fn append(&mut self, samples: &[f64]) -> Result<()> {
        ensure!(
            samples.len() <= SAMPLE_RATE as usize && samples.iter().all(|s| s.is_finite()),
            "声音块无效"
        );
        let samples = &samples[samples.len().saturating_sub(WINDOW_SAMPLES)..];
        self.window.rotate_left(samples.len());
        self.window[WINDOW_SAMPLES - samples.len()..].copy_from_slice(samples);
        Ok(())
    }
    pub fn check(&mut self, now: f64) -> Result<(f64, bool)> {
        ensure!(
            now.is_finite() && now >= 0.0 && self.last_event.is_none_or(|last| now >= last),
            "声音时钟无效"
        );
        let correlation = max_correlation(&self.template, &self.window)?;
        let hit =
            correlation > self.threshold && self.last_event.is_none_or(|last| now - last >= 0.1);
        if hit {
            self.last_event = Some(now);
            self.window.fill(0.0);
        }
        Ok((correlation, hit))
    }
}
