use bevy::{color::palettes::basic::PURPLE, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

const FFT_SIZE: usize = 2048;
const FFT_HOP_SIZE: usize = FFT_SIZE / 2;
const SAMPLE_RATE: f32 = 48_000.0;
const FREQUENCY_BANDS: usize = 3;

fn main() {
    #[cfg(target_os = "windows")]
    unsafe {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }

    App::new()
        .add_plugins(DefaultPlugins)
        .init_resource::<AudioLevel>()
        .add_systems(Startup, (setup, start_audio_capture))
        .add_systems(Update, animate_mesh)
        .run();
}

#[derive(Resource)]
struct AudioLevel {
    latest: Arc<[AtomicU32; FREQUENCY_BANDS]>,
    smoothed: [f32; FREQUENCY_BANDS],
}

impl Default for AudioLevel {
    fn default() -> Self {
        Self {
            latest: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            smoothed: [0.0; FREQUENCY_BANDS],
        }
    }
}

#[derive(Component)]
struct FrequencyBand(usize);

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    for (index, x) in [-80.0, 0.0, 80.0].into_iter().enumerate() {
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::default())),
            MeshMaterial2d(materials.add(Color::from(PURPLE))),
            Transform::from_xyz(x, -60.0, 0.0).with_scale(Vec3::new(48.0, 8.0, 1.0)),
            FrequencyBand(index),
        ));
    }
}

#[cfg(target_os = "windows")]
fn start_audio_capture(audio_level: Res<AudioLevel>) {
    let latest = Arc::clone(&audio_level.latest);
    if let Err(error) = std::thread::Builder::new()
        .name("system-audio-capture".into())
        .spawn(move || {
            if let Err(error) = wasapi::initialize_mta().ok() {
                log::error!("Could not initialize WASAPI: {error}");
                return;
            }

            if let Err(error) = capture_system_audio(latest) {
                log::error!("System audio capture failed: {error}");
            }
        })
    {
        log::error!("Could not start system audio capture: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn start_audio_capture(_audio_level: Res<AudioLevel>) {
    log::warn!("System audio capture is only supported on Windows.");
}

#[cfg(target_os = "windows")]
fn capture_system_audio(
    latest: Arc<[AtomicU32; FREQUENCY_BANDS]>,
) -> Result<(), wasapi::WasapiError> {
    use rustfft::{FftPlanner, num_complex::Complex};
    use std::collections::VecDeque;
    use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator.get_default_device(&Direction::Render)?;
    let mut audio_client = device.get_iaudioclient()?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
    let (_, min_period) = audio_client.get_device_period()?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_period,
    };

    // A render endpoint initialized for capture enables WASAPI loopback.
    audio_client.initialize_client(&format, &Direction::Capture, &mode)?;
    let event = audio_client.set_get_eventhandle()?;
    let capture = audio_client.get_audiocaptureclient()?;
    audio_client.start_stream()?;

    let mut bytes = VecDeque::new();
    let mut samples = VecDeque::with_capacity(FFT_SIZE * 2);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|index| {
            0.5 - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / FFT_SIZE as f32).cos()
        })
        .collect();
    let window_sum: f32 = window.iter().sum();
    let mut fft_buffer = vec![Complex::new(0.0, 0.0); FFT_SIZE];

    loop {
        match event.wait_for_event(1_000) {
            Ok(()) => {}
            Err(wasapi::WasapiError::EventTimeout) => continue,
            Err(error) => return Err(error),
        }

        capture.read_from_device_to_deque(&mut bytes)?;
        bytes.make_contiguous();
        let (first, second) = bytes.as_slices();

        for sample in first.chunks_exact(8).chain(second.chunks_exact(8)) {
            let left = f32::from_le_bytes(sample[..4].try_into().expect("four-byte sample"));
            let right = f32::from_le_bytes(sample[4..].try_into().expect("four-byte sample"));
            let mono = (left + right) * 0.5;
            samples.push_back(if mono.is_finite() { mono } else { 0.0 });
        }
        bytes.clear();

        while samples.len() >= FFT_SIZE {
            for (index, sample) in samples.iter().take(FFT_SIZE).enumerate() {
                fft_buffer[index] = Complex::new(sample * window[index], 0.0);
            }
            fft.process(&mut fft_buffer);

            let mut band_power = [0.0_f32; FREQUENCY_BANDS];
            for (bin, value) in fft_buffer.iter().enumerate().take(FFT_SIZE / 2).skip(1) {
                let frequency = bin as f32 * SAMPLE_RATE / FFT_SIZE as f32;
                let band = if frequency < 250.0 {
                    0
                } else if frequency < 2_000.0 {
                    1
                } else {
                    2
                };
                let amplitude = value.norm() * 2.0 / window_sum;
                band_power[band] += amplitude * amplitude * 0.5;
            }

            for (band, power) in band_power.into_iter().enumerate() {
                let level = (power.sqrt() * 5.0).clamp(0.0, 1.0);
                latest[band].store(level.to_bits(), Ordering::Relaxed);
            }

            for _ in 0..FFT_HOP_SIZE {
                samples.pop_front();
            }
        }
    }
}

fn animate_mesh(
    time: Res<Time>,
    mut audio_level: ResMut<AudioLevel>,
    mut meshes: Query<(&mut Transform, &FrequencyBand), With<Mesh2d>>,
) {
    let latest = Arc::clone(&audio_level.latest);
    for (band, smoothed) in audio_level.smoothed.iter_mut().enumerate() {
        let target = f32::from_bits(latest[band].load(Ordering::Relaxed));
        let response = if target > *smoothed { 18.0 } else { 4.0 };
        *smoothed += (target - *smoothed) * (response * time.delta_secs()).clamp(0.0, 1.0);
    }

    for (mut transform, band) in &mut meshes {
        let height = 8.0 + audio_level.smoothed[band.0] * 120.0;
        transform.scale.y = height;
        transform.translation.y = -60.0 + height / 2.0;
    }
}
