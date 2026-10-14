//! Apply a deliberate squawk selection without changing the flight crew's TCAS mode.
use std::collections::BTreeMap;
use xplane::data::{DataRead, DataReadWrite, ReadWrite, borrowed::DataRef};

enum Binding {
    Integer(DataRef<i32, ReadWrite>),
    Float(DataRef<f32, ReadWrite>),
}
impl Binding {
    fn set(&mut self, value: i32) {
        match self {
            Self::Integer(reference) => reference.set(value),
            Self::Float(reference) => reference.set(value as f32),
        }
    }
    fn matches(&self, value: i32) -> bool {
        match self {
            Self::Integer(reference) => reference.get() == value,
            Self::Float(reference) => reference.get() == value as f32,
        }
    }
}

pub(super) fn select(
    data: &mut xplane::data::DataApi,
    code: i32,
    controls: &openatc_core::transponder::Controls,
) -> Result<(), String> {
    let names = std::iter::once(&controls.code_dataref)
        .chain(controls.default_code_dataref.iter())
        .chain(controls.code_digits.iter())
        .chain(controls.power.iter().map(|power| &power.dataref));
    let mut bindings = BTreeMap::new();
    for name in names {
        let binding = data
            .find::<i32, _>(name)
            .ok()
            .and_then(|reference| reference.writeable().ok())
            .map(Binding::Integer)
            .or_else(|| {
                data.find::<f32, _>(name)
                    .ok()
                    .and_then(|reference| reference.writeable().ok())
                    .map(Binding::Float)
            });
        if let Some(binding) = binding {
            bindings.insert(name.clone(), binding);
        }
    }
    let writes = controls.writes(code, |name| bindings.contains_key(name))?;
    for (name, value) in &writes {
        bindings.get_mut(name).unwrap().set(*value);
    }
    if writes
        .iter()
        .any(|(name, value)| !bindings[name].matches(*value))
    {
        return Err("Aircraft did not accept the transponder selection".into());
    }
    Ok(())
}
