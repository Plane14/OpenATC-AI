//! Speech worker and discovery of the selected model's loaded voices.
use crate::{TtsJob, compute::DeviceStatus};
use openatc_tts::{TtsEngine, fx::apply_fx, kitten::Kitten, wav_bytes};
use std::{path::PathBuf, sync::mpsc};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum, PartialEq, Eq)]
pub enum Provider {
    #[default]
    Kokoro,
    Kitten,
}
impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Kokoro => "kokoro",
            Self::Kitten => "kitten",
        }
    }
}

pub struct Config {
    pub provider: Provider,
    pub force_cpu: bool,
    pub root: PathBuf,
}

pub struct Status {
    pub provider: String,
    pub device: DeviceStatus,
    pub voices: serde_json::Value,
}

enum Engine {
    Kokoro(Box<TtsEngine>),
    Kitten(Box<Kitten>),
}

fn load(config: &Config) -> Result<(Engine, Status), String> {
    let engine = match config.provider {
        Provider::Kokoro => Engine::Kokoro(Box::new(TtsEngine::load_with_device(
            &config.root.join("tts/kokoro-v1.0.onnx"),
            &config.root.join("tts/voices-v1.0.bin"),
            config.force_cpu,
        )?)),
        Provider::Kitten => Engine::Kitten(Box::new(Kitten::load(
            &config.root.join("tts/kitten"),
            config.force_cpu,
        )?)),
    };
    let (device, voices) = match &engine {
        Engine::Kokoro(e) => (
            DeviceStatus::new(e.device(), e.device_reason()),
            serde_json::json!(e.voice_names()),
        ),
        Engine::Kitten(e) => (DeviceStatus::new(e.device().0, e.device().1), e.voices()),
    };
    Ok((
        engine,
        Status {
            provider: config.provider.name().into(),
            device,
            voices,
        },
    ))
}

pub fn run(
    config: &Config,
    jobs: mpsc::Receiver<TtsJob>,
    ready: &mpsc::Sender<Result<Status, String>>,
) {
    let (mut engine, status) = match load(config) {
        Ok(value) => value,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(status));
    for job in jobs {
        let started = std::time::Instant::now();
        let samples = match &mut engine {
            Engine::Kokoro(e) => e.synthesize(
                &job.text,
                &job.voice,
                job.speed,
                job.sentence_pause,
                job.clause_pause,
            ),
            Engine::Kitten(e) => e.synthesize(
                &job.text,
                &job.voice,
                job.speed,
                job.sentence_pause,
                job.clause_pause,
            ),
        };
        let result = samples.map(|samples| {
            let seconds = f64::from(u32::try_from(samples.len()).unwrap_or(u32::MAX))
                / f64::from(openatc_tts::SAMPLE_RATE);
            eprintln!(
                "TTS provider={} voice={} generation={:.2}s audio={seconds:.2}s",
                config.provider.name(),
                job.voice,
                started.elapsed().as_secs_f64()
            );
            let seed = format!("{}|{}", job.text, job.voice);
            wav_bytes(&apply_fx(&samples, openatc_tts::SAMPLE_RATE, job.fx, &seed))
        });
        if let Err(error) = &result {
            eprintln!(
                "TTS failed: voice={} text={:?} error={error}",
                job.voice, job.text
            );
        }
        let _ = job.reply.send(result);
    }
}
