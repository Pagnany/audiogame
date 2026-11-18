use crate::audio::AudioLevel;
use bevy::prelude::Res;

pub(crate) fn start_audio_capture(_audio_level: Res<AudioLevel>) {
    log::warn!("System audio capture is only supported on Windows and Linux.");
}
