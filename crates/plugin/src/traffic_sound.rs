//! Traffic aircraft sound emission using XPLMPlayPCMOnBus.
//! Generates simple engine tones; later can load real WAV files from X-Plane Resources.

use std::sync::atomic::{AtomicBool, Ordering};
use xplane_sys as xp;

use openatc_core::traffic::animation::{Animation, Channel};

/// Engine sound type determines base frequency and character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSoundType {
    /// High-bypass turbofan (airliners) - lower, deeper tone
    JetMulti,
    /// Low-bypass turbofan (bizjets, older jets) - higher whine
    JetSingle,
    /// Turboprop - propeller + turbine whine
    Turboprop,
    /// Piston engine - distinct chugging
    Piston,
}

impl EngineSoundType {
    /// Base frequency in Hz for the engine tone.
    fn base_frequency(&self) -> f32 {
        match self {
            EngineSoundType::JetMulti => 80.0,    // Deep rumble
            EngineSoundType::JetSingle => 150.0,  // Higher whine
            EngineSoundType::Turboprop => 60.0,   // Prop + turbine
            EngineSoundType::Piston => 40.0,      // Low chug
        }
    }

    /// Harmonic content multiplier - how many harmonics to add for richness.
    fn harmonics(&self) -> usize {
        match self {
            EngineSoundType::JetMulti => 6,
            EngineSoundType::JetSingle => 8,
            EngineSoundType::Turboprop => 10,
            EngineSoundType::Piston => 12,
        }
    }

    /// Whether this engine type uses reverser sound.
    fn has_reverser(&self) -> bool {
        matches!(self, EngineSoundType::JetMulti | EngineSoundType::JetSingle | EngineSoundType::Turboprop)
    }
}

/// Sound state for one aircraft.
pub struct TrafficSound {
    engine_channel: *mut xp::FMOD_CHANNEL,
    aircraft_id: u32,
    engine_type: EngineSoundType,
    is_playing: bool,
    // Discrete sound state tracking
    last_gear_ratio: f32,
    last_flap_ratio: f32,
    tire_roll_channel: Option<*mut xp::FMOD_CHANNEL>,
    reverser_channel: Option<*mut xp::FMOD_CHANNEL>,
    // Audio buffer for engine tone (generated once, looped)
    engine_buffer: Vec<i16>,
    buffer_size: u32,
    sample_rate: i32,
    /// Master volume multiplier from config (public for runtime updates)
    pub volume_multiplier: f32,
}

impl TrafficSound {
    /// Create a new traffic sound for an aircraft.
    pub fn new(
        aircraft_id: u32,
        engine_type: EngineSoundType,
        position: [f32; 3],
        volume_multiplier: f32,
    ) -> Result<Self, String> {
        // Generate engine tone buffer (2 seconds at 48kHz, stereo)
        let sample_rate = 48000;
        let duration_secs = 2.0;
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        let engine_buffer = generate_engine_tone(engine_type, sample_rate, num_samples);
        let buffer_size = (engine_buffer.len() * 2) as u32; // bytes (i16 = 2 bytes)

        let mut sound = Self {
            engine_channel: std::ptr::null_mut(),
            aircraft_id,
            engine_type,
            is_playing: false,
            last_gear_ratio: 1.0,
            last_flap_ratio: 0.0,
            tire_roll_channel: None,
            reverser_channel: None,
            engine_buffer,
            buffer_size,
            sample_rate,
            volume_multiplier,
        };

        // Start engine sound
        sound.start_engine_sound(position)?;
        Ok(sound)
    }

    /// Start the continuous engine sound loop.
    fn start_engine_sound(&mut self, position: [f32; 3]) -> Result<(), String> {
        let pos = xp::FMOD_VECTOR {
            x: position[0],
            y: position[1],
            z: position[2],
        };
        let vel = xp::FMOD_VECTOR { x: 0.0, y: 0.0, z: 0.0 };

        // Play on ExteriorEnvironment bus (for AI traffic)
        let channel = unsafe {
            xp::XPLMPlayPCMOnBus(
                self.engine_buffer.as_mut_ptr() as *mut std::ffi::c_void,
                self.buffer_size,
                xp::FMOD_SOUND_FORMAT_PCM16,
                self.sample_rate,
                2, // stereo
                1, // loop
                xp::XPLMAudioBus::ExteriorEnvironment,
                None,
                std::ptr::null_mut(),
            )
        };

        if channel.is_null() {
            return Err("Failed to start engine sound".into());
        }

        // Set initial position
        let result = unsafe { xp::XPLMSetAudioPosition(channel, &pos, &vel) };
        if result != xp::FMOD_OK {
            eprintln!("Warning: Failed to set initial audio position: {:?}", result);
        }

        // Set volume with multiplier
        let result = unsafe { xp::XPLMSetAudioVolume(channel, self.volume_multiplier) };
        if result != xp::FMOD_OK {
            eprintln!("Warning: Failed to set initial audio volume: {:?}", result);
        }

        self.engine_channel = channel;
        self.is_playing = true;
        Ok(())
    }

    /// Update 3D position of all active sounds.
    pub fn update_position(&mut self, position: [f32; 3]) {
        if self.engine_channel.is_null() {
            return;
        }
        let pos = xp::FMOD_VECTOR {
            x: position[0],
            y: position[1],
            z: position[2],
        };
        let vel = xp::FMOD_VECTOR { x: 0.0, y: 0.0, z: 0.0 };
        unsafe { xp::XPLMSetAudioPosition(self.engine_channel, &pos, &vel) };

        if let Some(ch) = self.tire_roll_channel {
            unsafe { xp::XPLMSetAudioPosition(ch, &pos, &vel) };
        }
        if let Some(ch) = self.reverser_channel {
            unsafe { xp::XPLMSetAudioPosition(ch, &pos, &vel) };
        }
    }

    /// Update engine power (0.0-1.0) - maps to volume and pitch.
    pub fn set_engine_power(&mut self, power: f32) {
        if self.engine_channel.is_null() {
            return;
        }
        // Volume: 0.0 to 1.0 based on power, with minimum idle volume
        let volume = (0.15 + 0.85 * power.clamp(0.0, 1.0)) * self.volume_multiplier;
        unsafe { xp::XPLMSetAudioVolume(self.engine_channel, volume) };

        // Pitch: base frequency * (0.8 + 0.6 * power) for realistic engine spool
        let pitch_hz = self.engine_type.base_frequency() * (0.8 + 0.6 * power.clamp(0.0, 1.0));
        unsafe { xp::XPLMSetAudioPitch(self.engine_channel, pitch_hz) };
    }

    /// Check and trigger discrete sound events (gear, flaps, tire roll, reverser).
    pub fn check_discrete_events(
        &mut self,
        animation: &Animation,
        on_ground: bool,
        knots: f64,
        stage: openatc_core::traffic::motion::Stage,
    ) {
        // Gear sound
        let gear_ratio = animation.channel(Channel::Gear);
        if (gear_ratio - self.last_gear_ratio).abs() > 0.05 {
            self.play_gear_sound();
            self.last_gear_ratio = gear_ratio;
        }

        // Flap sound
        let flap_ratio = animation.channel(Channel::Flaps);
        if (flap_ratio - self.last_flap_ratio).abs() > 0.05 {
            self.play_flap_sound();
            self.last_flap_ratio = flap_ratio;
        }

        // Tire roll sound (ground roll)
        self.update_tire_roll(on_ground, knots, animation);

        // Reverser sound (rollout with reverse thrust)
        self.update_reverser(stage, knots, animation);
    }

    /// Play gear deployment/retraction sound once.
    fn play_gear_sound(&mut self) {
        // Generate a short mechanical thunk/clunk
        let buffer = generate_gear_sound(48000);
        let size = (buffer.len() * 2) as u32;
        let channel = unsafe {
            xp::XPLMPlayPCMOnBus(
                buffer.as_ptr() as *mut std::ffi::c_void,
                size,
                xp::FMOD_SOUND_FORMAT_PCM16,
                48000,
                2,
                0, // no loop
                xp::XPLMAudioBus::ExteriorEnvironment,
                None,
                std::ptr::null_mut(),
            )
        };
        if !channel.is_null() {
            unsafe { xp::XPLMSetAudioVolume(channel, 0.5 * self.volume_multiplier) };
            // Channel will auto-cleanup via callback in real implementation
            // For now we leak it (short sound, acceptable for v1)
        }
    }

    /// Play flap deployment/retraction sound once.
    fn play_flap_sound(&mut self) {
        // Generate a short hydraulic whir
        let buffer = generate_flap_sound(48000);
        let size = (buffer.len() * 2) as u32;
        let channel = unsafe {
            xp::XPLMPlayPCMOnBus(
                buffer.as_ptr() as *mut std::ffi::c_void,
                size,
                xp::FMOD_SOUND_FORMAT_PCM16,
                48000,
                2,
                0, // no loop
                xp::XPLMAudioBus::ExteriorEnvironment,
                None,
                std::ptr::null_mut(),
            )
        };
        if !channel.is_null() {
            unsafe { xp::XPLMSetAudioVolume(channel, 0.4 * self.volume_multiplier) };
        }
    }

    /// Update tire roll sound (looping on ground above 3 kts).
    fn update_tire_roll(&mut self, on_ground: bool, knots: f64, animation: &Animation) {
        let should_play = on_ground && knots > 3.0;

        if should_play && self.tire_roll_channel.is_none() {
            // Start tire roll sound
            let buffer = generate_tire_roll_sound(48000);
            let size = (buffer.len() * 2) as u32;
            let channel = unsafe {
                xp::XPLMPlayPCMOnBus(
                    buffer.as_ptr() as *mut std::ffi::c_void,
                    size,
                    xp::FMOD_SOUND_FORMAT_PCM16,
                    48000,
                    2,
                    1, // loop
                    xp::XPLMAudioBus::ExteriorEnvironment,
                    None,
                    std::ptr::null_mut(),
                )
            };
            if !channel.is_null() {
                self.tire_roll_channel = Some(channel);
            }
        } else if !should_play && self.tire_roll_channel.is_some() {
            // Stop tire roll sound
            if let Some(ch) = self.tire_roll_channel.take() {
                unsafe { xp::XPLMStopAudio(ch) };
            }
        }

        // Update tire roll volume based on wheel RPM (ground speed)
        if let Some(ch) = self.tire_roll_channel {
            let wheel_rpm = animation.channel(Channel::WheelAngle).abs(); // 0-360 deg per frame
            let volume = (0.1 + 0.4 * (wheel_rpm / 360.0).min(1.0)) * self.volume_multiplier;
            unsafe { xp::XPLMSetAudioVolume(ch, volume) };
        }
    }

    /// Update reverser sound (jet/turboprop only, during rollout at high speed).
    fn update_reverser(&mut self, stage: openatc_core::traffic::motion::Stage, knots: f64, animation: &Animation) {
        if !self.engine_type.has_reverser() {
            return;
        }

        let is_rollout = matches!(stage, openatc_core::traffic::motion::Stage::Rollout);
        let engine_spooled = animation.channel(Channel::Engine) > 0.3;
        let high_speed = knots > 80.0;

        let should_play = is_rollout && engine_spooled && high_speed;

        if should_play && self.reverser_channel.is_none() {
            // Start reverser sound
            let buffer = generate_reverser_sound(48000);
            let size = (buffer.len() * 2) as u32;
            let channel = unsafe {
                xp::XPLMPlayPCMOnBus(
                    buffer.as_ptr() as *mut std::ffi::c_void,
                    size,
                    xp::FMOD_SOUND_FORMAT_PCM16,
                    48000,
                    2,
                    1, // loop
                    xp::XPLMAudioBus::ExteriorEnvironment,
                    None,
                    std::ptr::null_mut(),
                )
            };
            if !channel.is_null() {
                self.reverser_channel = Some(channel);
            }
        } else if (!should_play || knots < 40.0) && self.reverser_channel.is_some() {
            // Stop reverser sound (cancelled around 60-80 kts)
            if let Some(ch) = self.reverser_channel.take() {
                unsafe { xp::XPLMStopAudio(ch) };
            }
        }

        // Update reverser volume based on reverse thrust ratio
        if let Some(ch) = self.reverser_channel {
            let reverse_ratio = animation.channel(Channel::Engine) * 0.8; // simulated
            let volume = reverse_ratio.clamp(0.0, 1.0) * self.volume_multiplier;
            unsafe { xp::XPLMSetAudioVolume(ch, volume) };
        }
    }

    /// Stop all sounds for this aircraft.
    pub fn stop(&mut self) {
        if !self.engine_channel.is_null() {
            unsafe { xp::XPLMStopAudio(self.engine_channel) };
            self.engine_channel = std::ptr::null_mut();
        }
        if let Some(ch) = self.tire_roll_channel.take() {
            unsafe { xp::XPLMStopAudio(ch) };
        }
        if let Some(ch) = self.reverser_channel.take() {
            unsafe { xp::XPLMStopAudio(ch) };
        }
        self.is_playing = false;
    }
}

impl Drop for TrafficSound {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Generate a continuous engine tone buffer using additive synthesis.
fn generate_engine_tone(engine_type: EngineSoundType, sample_rate: i32, num_samples: usize) -> Vec<i16> {
    let base_freq = engine_type.base_frequency();
    let num_harmonics = engine_type.harmonics();
    let mut buffer = Vec::with_capacity(num_samples * 2); // stereo

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let mut sample = 0.0f32;

        // Fundamental + harmonics for richness
        for h in 1..=num_harmonics {
            let harmonic_freq = base_freq * h as f32;
            let amplitude = 1.0 / (h as f32).powf(1.2); // Roll off higher harmonics
            sample += amplitude * (2.0 * std::f32::consts::PI * harmonic_freq * t).sin();
        }

        // Normalize and add slight variation for realism
        sample = sample / num_harmonics as f32;
        sample *= 0.3; // Keep headroom

        // Convert to stereo i16
        let val = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        buffer.push(val); // Left
        buffer.push(val); // Right
    }

    buffer
}

/// Generate gear deployment/retraction sound (mechanical clunk).
fn generate_gear_sound(sample_rate: i32) -> Vec<i16> {
    let duration = 0.5; // seconds
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut buffer = Vec::with_capacity(num_samples * 2);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        // Sharp transient with decay
        let envelope = (-t * 15.0).exp();
        let sample = envelope * (2.0 * std::f32::consts::PI * 120.0 * t).sin() * 0.5;
        let val = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        buffer.push(val);
        buffer.push(val);
    }

    buffer
}

/// Generate flap movement sound (hydraulic whir).
fn generate_flap_sound(sample_rate: i32) -> Vec<i16> {
    let duration = 1.0; // seconds
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut buffer = Vec::with_capacity(num_samples * 2);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        // Sweeping tone with noise-like quality
        let freq = 200.0 + 400.0 * t; // sweep up
        let envelope = 1.0 - t; // decay
        let sample = envelope * (2.0 * std::f32::consts::PI * freq * t).sin() * 0.3;
        let val = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        buffer.push(val);
        buffer.push(val);
    }

    buffer
}

/// Generate tire roll sound (low rumble).
fn generate_tire_roll_sound(sample_rate: i32) -> Vec<i16> {
    let duration = 1.0; // 1 second loop
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut buffer = Vec::with_capacity(num_samples * 2);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        // Low frequency rumble with some texture
        let sample = 0.15 * (2.0 * std::f32::consts::PI * 40.0 * t).sin()
            + 0.08 * (2.0 * std::f32::consts::PI * 80.0 * t).sin()
            + 0.04 * (2.0 * std::f32::consts::PI * 120.0 * t).sin();
        let val = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        buffer.push(val);
        buffer.push(val);
    }

    buffer
}

/// Generate thrust reverser sound (loud roar).
fn generate_reverser_sound(sample_rate: i32) -> Vec<i16> {
    let duration = 0.5; // seconds
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut buffer = Vec::with_capacity(num_samples * 2);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        // Broadband noise-like roar
        let sample = 0.4 * (2.0 * std::f32::consts::PI * 100.0 * t).sin()
            + 0.3 * (2.0 * std::f32::consts::PI * 250.0 * t).sin()
            + 0.2 * (2.0 * std::f32::consts::PI * 500.0 * t).sin()
            + 0.1 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
        let val = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        buffer.push(val);
        buffer.push(val);
    }

    buffer
}

/// Check if X-Plane version supports sound API (12.04+).
pub fn is_sound_api_available() -> bool {
    // XPLM_API_VERSION >= 400 means X-Plane 12+
    // Sound API added in 12.04, but we'll check at runtime via function pointer
    // For now, assume available if XPLM400 feature is enabled
    cfg!(feature = "XPLM400")
}