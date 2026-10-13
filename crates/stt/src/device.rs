//! Prefer discrete graphics without relying on driver enumeration order.
#![allow(unsafe_code)]
use std::ffi::CStr;
use whisper_rs::whisper_rs_sys as ffi;

pub fn preferred_gpu() -> Option<(i32, String)> {
    // GGML owns these device handles for the lifetime of its backend registry.
    unsafe {
        ffi::ggml_backend_load_all();
        let mut index = 0;
        let mut choices = Vec::new();
        for slot in 0..ffi::ggml_backend_dev_count() {
            let device = ffi::ggml_backend_dev_get(slot);
            if device.is_null() {
                continue;
            }
            let kind = ffi::ggml_backend_dev_type(device);
            let discrete = kind == ffi::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU;
            if !discrete && kind != ffi::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU {
                continue;
            }
            let description = ffi::ggml_backend_dev_description(device);
            let name = if description.is_null() {
                format!("GPU {index}")
            } else {
                CStr::from_ptr(description).to_string_lossy().into_owned()
            };
            let (mut free, mut total) = (0, 0);
            ffi::ggml_backend_dev_memory(device, &raw mut free, &raw mut total);
            choices.push((discrete, free, index, name));
            index += 1;
        }
        choices
            .into_iter()
            .max_by_key(|(discrete, free, _, _)| (*discrete, *free))
            .map(|(_, _, index, name)| (index, name))
    }
}
