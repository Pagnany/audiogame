use bevy::{color::palettes::basic::PURPLE, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

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

#[derive(Resource, Default)]
struct AudioLevel {
    latest: Arc<AtomicU32>,
    smoothed: f32,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    commands.spawn((
        Mesh2d(meshes.add(Rectangle::default())),
        MeshMaterial2d(materials.add(Color::from(PURPLE))),
        Transform::default().with_scale(Vec3::splat(128.)),
    ));
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
fn capture_system_audio(latest: Arc<AtomicU32>) -> Result<(), wasapi::WasapiError> {
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
    loop {
        match event.wait_for_event(1_000) {
            Ok(()) => {}
            Err(wasapi::WasapiError::EventTimeout) => continue,
            Err(error) => return Err(error),
        }

        capture.read_from_device_to_deque(&mut bytes)?;
        let mut sum_of_squares = 0.0_f64;
        let mut sample_count = 0_u64;
        let (first, second) = bytes.as_slices();

        for sample in first
            .chunks_exact(4)
            .chain(second.chunks_exact(4))
            .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte sample")))
            .filter(|sample| sample.is_finite())
        {
            sum_of_squares += f64::from(sample) * f64::from(sample);
            sample_count += 1;
        }
        bytes.clear();

        let rms = if sample_count == 0 {
            0.0
        } else {
            (sum_of_squares / sample_count as f64).sqrt() as f32
        };
        latest.store(rms.to_bits(), Ordering::Relaxed);
    }
}

fn animate_mesh(
    time: Res<Time>,
    mut audio_level: ResMut<AudioLevel>,
    mut mesh: Query<&mut Transform, With<Mesh2d>>,
) {
    let rms = f32::from_bits(audio_level.latest.load(Ordering::Relaxed));
    let target = (rms * 5.0).clamp(0.0, 1.0);
    let response = if target > audio_level.smoothed {
        18.0
    } else {
        4.0
    };
    audio_level.smoothed +=
        (target - audio_level.smoothed) * (response * time.delta_secs()).clamp(0.0, 1.0);

    if let Ok(mut transform) = mesh.single_mut() {
        transform.scale.x = 128.0 * (1.0 + audio_level.smoothed * 0.4);
        transform.scale.y = 128.0 * (1.0 + audio_level.smoothed * 0.8);
    }
}
