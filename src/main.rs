use bevy::{color::palettes::basic::PURPLE, prelude::*};
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::collections::VecDeque;
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

#[cfg(any(target_os = "windows", target_os = "linux"))]
const FFT_SIZE: usize = 2048;
#[cfg(any(target_os = "windows", target_os = "linux"))]
const FFT_HOP_SIZE: usize = FFT_SIZE / 2;
#[cfg(any(target_os = "windows", target_os = "linux"))]
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

#[cfg(target_os = "linux")]
fn start_audio_capture(audio_level: Res<AudioLevel>) {
    let latest = Arc::clone(&audio_level.latest);
    if let Err(error) = std::thread::Builder::new()
        .name("system-audio-capture".into())
        .spawn(move || {
            if let Err(error) = capture_system_audio(latest) {
                log::error!("System audio capture failed: {error}");
            }
        })
    {
        log::error!("Could not start system audio capture: {error}");
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn start_audio_capture(_audio_level: Res<AudioLevel>) {
    log::warn!("System audio capture is only supported on Windows and Linux.");
}

#[cfg(target_os = "windows")]
fn capture_system_audio(
    latest: Arc<[AtomicU32; FREQUENCY_BANDS]>,
) -> Result<(), wasapi::WasapiError> {
    use rustfft::FftPlanner;
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
    let mut planner = FftPlanner::<f32>::new();
    let mut analyzer = AudioAnalyzer::new(&mut planner);

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
            analyzer.push_sample((left + right) * 0.5, &latest);
        }
        bytes.clear();
    }
}

#[cfg(target_os = "linux")]
fn capture_system_audio(latest: Arc<[AtomicU32; FREQUENCY_BANDS]>) -> std::io::Result<()> {
    use std::io::{BufReader, Read};

    let mut child = std::process::Command::new("parec")
        .args([
            "--device=@DEFAULT_MONITOR@",
            "--format=float32le",
            "--rate=48000",
            "--channels=2",
            "--latency-msec=20",
            "--raw",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("could not start `parec` (install pulseaudio-utils): {error}"),
            )
        })?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut reader = BufReader::new(stdout);
    let mut analyzer = AudioAnalyzer::new(&mut rustfft::FftPlanner::new());
    let mut frame = [0_u8; 8];

    loop {
        if let Err(error) = reader.read_exact(&mut frame) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }

        let left = f32::from_le_bytes(frame[..4].try_into().expect("four-byte sample"));
        let right = f32::from_le_bytes(frame[4..].try_into().expect("four-byte sample"));
        analyzer.push_sample((left + right) * 0.5, &latest);
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
struct AudioAnalyzer {
    samples: VecDeque<f32>,
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    window_sum: f32,
    fft_buffer: Vec<rustfft::num_complex::Complex<f32>>,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
impl AudioAnalyzer {
    fn new(planner: &mut rustfft::FftPlanner<f32>) -> Self {
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
        }
    }

    fn push_sample(&mut self, sample: f32, latest: &[AtomicU32; FREQUENCY_BANDS]) {
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
                let frequency = bin as f32 * SAMPLE_RATE / FFT_SIZE as f32;
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
