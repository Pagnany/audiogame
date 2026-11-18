use crate::audio::{AudioAnalyzer, AudioLevel, FREQUENCY_BANDS, SAMPLE_RATE};
use bevy::prelude::Res;
use std::collections::VecDeque;
use std::sync::{Arc, atomic::AtomicU32};

pub(crate) fn start_audio_capture(audio_level: Res<AudioLevel>) {
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
    let mut analyzer = AudioAnalyzer::new(&mut planner, SAMPLE_RATE);

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
