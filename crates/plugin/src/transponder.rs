//! Apply a deliberate squawk selection without changing the flight crew's TCAS mode.
use xplane::data::{DataRead, DataReadWrite, ReadOnly, borrowed::DataRef};

pub(super) fn select(
    data: &mut xplane::data::DataApi,
    code: i32,
    controls: &openatc_core::transponder::Controls,
) -> Result<(), String> {
    if !(0..=7777).contains(&code)
        || !format!("{code:04}")
            .bytes()
            .all(|digit| (b'0'..=b'7').contains(&digit))
    {
        return Err("Invalid transponder code".into());
    }
    let value: DataRef<i32, ReadOnly> = data
        .find(&controls.code_dataref)
        .map_err(|_| "Transponder code control unavailable")?;
    let mut value = value
        .writeable()
        .map_err(|_| "Transponder code control is read-only")?;
    value.set(code);
    // The two-position AUTO/ON switch is separate from the five-position mode selector.
    if let Some(power) = &controls.power {
        if let Ok(value) = data.find::<i32, _>(&power.dataref) {
            let mut selector = value
                .writeable()
                .map_err(|_| "Aircraft power selector is read-only")?;
            selector.set(power.on);
            if selector.get() != power.on {
                return Err("Code selected; aircraft did not accept the ON selector".into());
            }
        } else if let Ok(value) = data.find::<f32, _>(&power.dataref) {
            let mut selector = value
                .writeable()
                .map_err(|_| "Aircraft power selector is read-only")?;
            selector.set(power.on as f32);
            if selector.get() != power.on as f32 {
                return Err("Code selected; aircraft did not accept the ON selector".into());
            }
        }
    }
    if value.get() != code {
        return Err("Aircraft did not accept the transponder code".into());
    }
    Ok(())
}
