//! Generate the same radio sample in every loaded voice for local listening tests.
use openatc_tts::{SAMPLE_RATE, TtsEngine, kitten::Kitten, wav_bytes};
use std::{error::Error, path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 4 {
        return Err(
            "Usage: audition <kokoro|kitten> <model-directory> <output-directory> [--cpu]".into(),
        );
    }
    let root = PathBuf::from(&args[2]);
    let out = PathBuf::from(&args[3]);
    let cpu = args.iter().any(|s| s == "--cpu");
    std::fs::create_dir_all(&out)?;
    let text = "Charlie Golf Tango Lima Tango, cross runway two five right, report vacated. Contact Los Angeles Tower on one two zero decimal niner five zero.";
    let render = |voice: &str, samples: Vec<f32>, started: Instant| -> Result<(), Box<dyn Error>> {
        if samples.is_empty() || samples.iter().any(|v| !v.is_finite()) {
            return Err(format!("Invalid audio for {voice}").into());
        }
        println!(
            "{voice}: {:.2}s generation, {:.2}s audio",
            started.elapsed().as_secs_f64(),
            f64::from(u32::try_from(samples.len())?) / f64::from(SAMPLE_RATE)
        );
        std::fs::write(out.join(format!("{voice}.wav")), wav_bytes(&samples))?;
        Ok(())
    };
    match args[1].as_str() {
        "kitten" => {
            let mut engine = Kitten::load(&root, cpu)?;
            println!("Device: {:?}", engine.device());
            for voice in engine.voices().as_array().ok_or("Missing voices")? {
                let id = voice["id"].as_str().ok_or("Missing voice ID")?;
                let started = Instant::now();
                render(id, engine.synthesize(text, id, 1.0, 0.25, 0.1)?, started)?;
            }
        }
        "kokoro" => {
            let mut engine = TtsEngine::load_with_device(
                &root.join("kokoro-v1.0.onnx"),
                &root.join("voices-v1.0.bin"),
                cpu,
            )?;
            println!("Device: {} ({})", engine.device(), engine.device_reason());
            for id in engine.voice_names().into_iter().filter(|s| {
                s.starts_with("af_")
                    || s.starts_with("am_")
                    || s.starts_with("bf_")
                    || s.starts_with("bm_")
            }) {
                let started = Instant::now();
                render(&id, engine.synthesize(text, &id, 1.0, 0.25, 0.1)?, started)?;
            }
        }
        _ => return Err("Choose kokoro or kitten".into()),
    }
    Ok(())
}
