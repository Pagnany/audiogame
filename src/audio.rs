use bevy::prelude::Resource;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use std::collections::VecDeque;

const FFT_SIZE: usize = 2048;
const FFT_HOP_SIZE: usize = FFT_SIZE / 2;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) const SAMPLE_RATE: f32 = 48_000.0;
pub(crate) const FREQUENCY_BANDS: usize = 3;

#[derive(Resource)]
pub(crate) struct AudioLevel {
    pub(crate) latest: Arc<[AtomicU32; FREQUENCY_BANDS]>,
    pub(crate) smoothed: [f32; FREQUENCY_BANDS],
}

impl Default for AudioLevel {
    fn default() -> Self {
        let latest = Arc::new(std::array::from_fn(|_| AtomicU32::new(0)));
        #[cfg(target_arch = "wasm32")]
        crate::platform::wasm::set_audio_levels(Arc::clone(&latest));

        Self {
            latest,
            smoothed: [0.0; FREQUENCY_BANDS],
        }
    }
}

pub(crate) struct AudioAnalyzer {
    samples: VecDeque<f32>,
    fft: Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    window_sum: f32,
    fft_buffer: Vec<rustfft::num_complex::Complex<f32>>,
    sample_rate: f32,
}

impl AudioAnalyzer {
    pub(crate) fn new(planner: &mut rustfft::FftPlanner<f32>, sample_rate: f32) -> Self {
        use rustfft::num_complex::Complex;

        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|index| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / FFT_SIZE as f32).cos()
            })
            .collect();

        Self {
            samples: VecDeque::with_capacity(FFT_SIZE * 2),
            fft: planner.plan_fft_forward(FFT_SIZE),
            window_sum: window.iter().sum(),
            window,
            fft_buffer: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            sample_rate,
        }
    }

    pub(crate) fn push_sample(&mut self, sample: f32, latest: &[AtomicU32; FREQUENCY_BANDS]) {
        self.samples
            .push_back(if sample.is_finite() { sample } else { 0.0 });

        while self.samples.len() >= FFT_SIZE {
            for (index, sample) in self.samples.iter().take(FFT_SIZE).enumerate() {
                self.fft_buffer[index] =
                    rustfft::num_complex::Complex::new(sample * self.window[index], 0.0);
            }
            self.fft.process(&mut self.fft_buffer);

            let mut band_power = [0.0_f32; FREQUENCY_BANDS];
            for (bin, value) in self
                .fft_buffer
                .iter()
                .enumerate()
                .take(FFT_SIZE / 2)
                .skip(1)
            {
                let frequency = bin as f32 * self.sample_rate / FFT_SIZE as f32;
                let band = if frequency < 250.0 {
                    0
                } else if frequency < 2_000.0 {
                    1
                } else {
                    2
                };
                let amplitude = value.norm() * 2.0 / self.window_sum;
                band_power[band] += amplitude * amplitude * 0.5;
            }

            for (band, power) in band_power.into_iter().enumerate() {
                let level = (power.sqrt() * 5.0).clamp(0.0, 1.0);
                latest[band].store(level.to_bits(), Ordering::Relaxed);
            }

            for _ in 0..FFT_HOP_SIZE {
                self.samples.pop_front();
            }
        }
    }
}
