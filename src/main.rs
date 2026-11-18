use audio::AudioLevel;
use bevy::{color::palettes::basic::PURPLE, prelude::*};
use platform::start_audio_capture;

mod audio;
mod platform;

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

fn animate_mesh(
    time: Res<Time>,
    mut audio_level: ResMut<AudioLevel>,
    mut meshes: Query<(&mut Transform, &FrequencyBand), With<Mesh2d>>,
) {
    let latest = std::sync::Arc::clone(&audio_level.latest);
    for (band, smoothed) in audio_level.smoothed.iter_mut().enumerate() {
        let target = f32::from_bits(latest[band].load(std::sync::atomic::Ordering::Relaxed));
        let response = if target > *smoothed { 18.0 } else { 4.0 };
        *smoothed += (target - *smoothed) * (response * time.delta_secs()).clamp(0.0, 1.0);
    }

    for (mut transform, band) in &mut meshes {
        let height = 8.0 + audio_level.smoothed[band.0] * 120.0;
        transform.scale.y = height;
        transform.translation.y = -60.0 + height / 2.0;
    }
}
