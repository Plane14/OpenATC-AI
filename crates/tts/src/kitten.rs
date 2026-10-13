//! Kitten Mini 0.8 synthesis with the upstream ONNX token and style format.
use ndarray::{Array1, Array2};
use ort::value::Tensor;
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

#[derive(Deserialize)]
struct Config {
    model_file: String,
    voices: String,
    #[serde(default)]
    voice_aliases: BTreeMap<String, String>,
    #[serde(default)]
    speed_priors: BTreeMap<String, f32>,
}

/// One loaded Kitten model and its voice styles.
pub struct Kitten {
    session: ort::session::Session,
    config: Config,
    styles: BTreeMap<String, Array2<f32>>,
    vocab: BTreeMap<char, i64>,
    tokenizer: regex::Regex,
    device: String,
    reason: String,
}

impl Kitten {
    /// Load Kitten Mini 0.8 assets from a local directory.
    ///
    /// # Errors
    /// Returns missing assets, invalid voice tables or model initialization errors.
    pub fn load(directory: &Path, force_cpu: bool) -> Result<Self, String> {
        let config: Config = serde_json::from_slice(
            &std::fs::read(directory.join("config.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        for file in [&config.model_file, &config.voices] {
            if Path::new(file).components().count() != 1
                || file.starts_with('.')
                || file.contains(['/', '\\'])
            {
                return Err("Kitten assets must be filenames within the model directory".into());
            }
        }
        let (session, device, reason) =
            super::device::load(&directory.join(&config.model_file), force_cpu)?;
        let mut archive = ndarray_npy::NpzReader::new(
            std::fs::File::open(directory.join(&config.voices)).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut styles = BTreeMap::new();
        for name in config.voice_aliases.values() {
            let array: Array2<f32> = archive
                .by_name(name)
                .map_err(|e| format!("Kitten voice {name}: {e}"))?;
            if array.nrows() == 0 || array.ncols() != 256 || array.iter().any(|v| !v.is_finite()) {
                return Err(format!("Invalid Kitten style table for {name}"));
            }
            styles.insert(name.clone(), array);
        }
        if styles.is_empty() {
            return Err("Kitten voice catalogue is empty".into());
        }
        let mut engine = Self {
            session,
            config,
            styles,
            vocab: vocabulary(),
            tokenizer: regex::Regex::new(r"\w+|[^\w\s]").map_err(|e| e.to_string())?,
            device,
            reason,
        };
        // CUDA loads some libraries on the first inference, after session creation.
        if engine.device != "cpu"
            && let Err(error) = engine.synthesize("Radio check.", "", 1.0, 0.0, 0.0)
        {
            eprintln!("Kitten GPU warm-up failed: {error}; using CPU");
            engine.session = super::device::cpu(&directory.join(&engine.config.model_file))?;
            engine.device = "cpu".into();
            engine.reason = error;
        }
        Ok(engine)
    }

    /// Preset identifiers, names and English language metadata for discovery.
    #[must_use]
    pub fn voices(&self) -> serde_json::Value {
        serde_json::json!(self.config.voice_aliases.keys().map(|name| serde_json::json!({
            "id": format!("kitten-{}", name.to_lowercase()), "name": name, "language": "en-US"
        })).collect::<Vec<_>>())
    }

    /// Selected execution provider and the reason for it.
    #[must_use]
    pub fn device(&self) -> (&str, &str) {
        (&self.device, &self.reason)
    }

    /// Synthesize English text into 24 kHz mono samples.
    ///
    /// # Errors
    /// Returns invalid input, unknown voices, phonemizer or model errors.
    pub fn synthesize(
        &mut self,
        text: &str,
        voice: &str,
        speed: f32,
        sentence_pause: f32,
        clause_pause: f32,
    ) -> Result<Vec<f32>, String> {
        if text.trim().is_empty() || text.len() > super::MAX_TEXT {
            return Err("Speech text is empty or too long".into());
        }
        let requested = voice.strip_prefix("kitten-").unwrap_or(voice);
        let (_, key) = self
            .config
            .voice_aliases
            .iter()
            .find(|(name, _)| {
                name.eq_ignore_ascii_case(if requested.is_empty() {
                    "Bella"
                } else {
                    requested
                })
            })
            .ok_or_else(|| format!("Unknown Kitten voice {voice:?}; refresh /v1/voices"))?;
        let style = &self.styles[key];
        let prior = self.config.speed_priors.get(key).copied().unwrap_or(1.0);
        let chunks = chunks(text);
        let mut audio = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            let phonemes = espeak_rs::text_to_phonemes(chunk, "en-us", None, true, false)
                .map_err(|e| e.to_string())?
                .join(" ");
            let phonemes = self
                .tokenizer
                .find_iter(&phonemes)
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let mut ids = vec![0];
            ids.extend(phonemes.chars().filter_map(|c| self.vocab.get(&c).copied()));
            ids.extend([10, 0]);
            if ids.len() > 512 {
                return Err("Kitten phoneme chunk exceeds model context".into());
            }
            let row = chunk.chars().count().min(style.nrows() - 1);
            let style = style.slice(ndarray::s![row..row + 1, ..]).to_owned();
            let ids = Array2::from_shape_vec((1, ids.len()), ids).map_err(|e| e.to_string())?;
            let outputs = self.session.run(ort::inputs![
                "input_ids" => Tensor::from_array(ids).map_err(|e| e.to_string())?,
                "style" => Tensor::from_array(style).map_err(|e| e.to_string())?,
                "speed" => Tensor::from_array(Array1::from_vec(vec![speed.clamp(0.5, 2.0) * prior])).map_err(|e| e.to_string())?
            ]).map_err(|e| e.to_string())?;
            let samples = outputs[0]
                .try_extract_array::<f32>()
                .map_err(|e| e.to_string())?;
            // The model appends 5,000 samples of padding to each chunk.
            let count = samples.len().saturating_sub(5000);
            if count == 0 || samples.iter().any(|v| !v.is_finite()) {
                return Err("Kitten returned invalid audio".into());
            }
            audio.extend(samples.iter().take(count).copied());
            if index + 1 < chunks.len() {
                let pause = super::pause_after(chunk, sentence_pause, clause_pause);
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                let count = (pause.clamp(0.0, 1.0) * super::SAMPLE_RATE as f32) as usize;
                audio.extend(std::iter::repeat_n(0.0, count));
            }
        }
        Ok(audio)
    }
}

fn vocabulary() -> BTreeMap<char, i64> {
    let symbols = concat!(
        "$",
        ";:,.!?¡¿—…\"«»\"\" ",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
        "ɑɐɒæɓʙβɔɕçɗɖðʤəɘɚɛɜɝɞɟʄɡɠɢʛɦɧħɥʜɨɪʝɭɬɫɮʟɱɯɰŋɳɲɴøɵɸθœɶʘɹɺɾɻʀʁɽʂʃʈʧʉʊʋⱱʌɣɤʍχʎʏʑʐʒʔʡʕʢǀǁǂǃˈˌːˑʼʴʰʱʲʷˠˤ˞↓↑→↗↘'̩'ᵻ"
    );
    symbols.chars().zip(0_i64..).collect()
}

fn chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.chars().count() + word.chars().count() + 1 > 180 && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
        if word.ends_with(['.', '?', '!', ';']) {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    #[test]
    fn chunks_preserve_frequencies_and_words() {
        let text = "Contact Tower on 120.950. Hold short of runway two five left.";
        let result = super::chunks(text);
        assert_eq!(result.len(), 2);
        assert_eq!(result.join(" "), text);
        let long = "Charlie Golf Tango Lima Tango ".repeat(30);
        assert_eq!(super::chunks(&long).join(" "), long.trim());
    }
    #[test]
    fn vocabulary_matches_model_indices() {
        let vocabulary = super::vocabulary();
        assert_eq!(vocabulary[&'$'], 0);
        assert_eq!(vocabulary[&'…'], 10);
        assert_eq!(vocabulary[&'A'], 17);
        assert_eq!(vocabulary[&'ᵻ'], 177);
    }
}
