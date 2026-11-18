use crate::audio::{AudioAnalyzer, AudioLevel, FREQUENCY_BANDS};
use bevy::prelude::Res;
use std::cell::RefCell;
use std::sync::{Arc, atomic::AtomicU32};

thread_local! {
    static WEB_AUDIO_ANALYZER: RefCell<Option<AudioAnalyzer>> = const { RefCell::new(None) };
    static WEB_AUDIO_LEVELS: RefCell<Option<Arc<[AtomicU32; FREQUENCY_BANDS]>>> = const { RefCell::new(None) };
}

pub(crate) fn start_audio_capture(_audio_level: Res<AudioLevel>) {}

pub(crate) fn set_audio_levels(levels: Arc<[AtomicU32; FREQUENCY_BANDS]>) {
    WEB_AUDIO_LEVELS.with(|audio_levels| *audio_levels.borrow_mut() = Some(levels));
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_audio_analysis(sample_rate: f32) -> Result<(), wasm_bindgen::JsValue> {
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(wasm_bindgen::JsValue::from_str(
            "Invalid audio sample rate.",
        ));
    }

    WEB_AUDIO_ANALYZER.with(|analyzer| {
        let mut planner = rustfft::FftPlanner::new();
        *analyzer.borrow_mut() = Some(AudioAnalyzer::new(&mut planner, sample_rate));
    });
    Ok(())
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn process_audio_samples(left: &[f32], right: &[f32]) -> Result<(), wasm_bindgen::JsValue> {
    if !right.is_empty() && left.len() != right.len() {
        return Err(wasm_bindgen::JsValue::from_str(
            "Audio channels have different lengths.",
        ));
    }

    let latest = WEB_AUDIO_LEVELS
        .with(|levels| levels.borrow().clone())
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("Audio analyzer is not initialized."))?;
    WEB_AUDIO_ANALYZER.with(|analyzer| {
        let mut analyzer = analyzer.borrow_mut();
        let analyzer = analyzer
            .as_mut()
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("Audio analyzer is not initialized."))?;

        if right.is_empty() {
            for &sample in left {
                analyzer.push_sample(sample, &latest);
            }
        } else {
            for (&left, &right) in left.iter().zip(right) {
                analyzer.push_sample((left + right) * 0.5, &latest);
            }
        }
        Ok(())
    })
}
