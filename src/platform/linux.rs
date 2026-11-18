use crate::audio::{AudioAnalyzer, AudioLevel, FREQUENCY_BANDS, SAMPLE_RATE};
use bevy::prelude::Res;
use std::process::Stdio;
use std::sync::{Arc, atomic::AtomicU32};

pub(crate) fn start_audio_capture(audio_level: Res<AudioLevel>) {
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
    let mut analyzer = AudioAnalyzer::new(&mut rustfft::FftPlanner::new(), SAMPLE_RATE);
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
