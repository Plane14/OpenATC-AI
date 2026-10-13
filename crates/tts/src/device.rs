//! ONNX device selection shared by the speech models.
use std::path::Path;

pub(crate) fn load(
    path: &Path,
    force_cpu: bool,
) -> Result<(ort::session::Session, String, String), String> {
    if !force_cpu {
        match accelerated(path) {
            Ok((session, name)) => {
                return Ok((
                    session,
                    name.into(),
                    "Accelerator initialized; unsupported operators may run on CPU".into(),
                ));
            }
            Err(error) => {
                eprintln!("TTS accelerator unavailable: {error}; using CPU");
                return Ok((cpu(path)?, "cpu".into(), error));
            }
        }
    }
    Ok((cpu(path)?, "cpu".into(), "Forced by option".into()))
}

pub(crate) fn cpu(path: &Path) -> Result<ort::session::Session, String> {
    ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(4)
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())
}

#[cfg(any(feature = "cuda", feature = "coreml"))]
fn accelerated(path: &Path) -> Result<(ort::session::Session, &'static str), String> {
    #[cfg(feature = "coreml")]
    let (provider, name) = (
        ort::ep::CoreML::default().build().error_on_failure(),
        "coreml",
    );
    #[cfg(all(feature = "cuda", not(feature = "coreml")))]
    let (provider, name) = (ort::ep::CUDA::default().build().error_on_failure(), "cuda");
    let session = ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(4)
        .map_err(|e| e.to_string())?
        .with_execution_providers([provider])
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())?;
    Ok((session, name))
}
#[cfg(not(any(feature = "cuda", feature = "coreml")))]
fn accelerated(_: &Path) -> Result<(ort::session::Session, &'static str), String> {
    Err("This ONNX Runtime build has no GPU provider".into())
}
