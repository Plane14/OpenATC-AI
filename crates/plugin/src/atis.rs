//! Keep the simulator's ATIS from speaking over the plugin's broadcast.
use xplane::data::{DataRead, DataReadWrite, ReadOnly, ReadWrite, borrowed::DataRef};

#[derive(Default)]
struct Ownership {
    previous: Option<i32>,
}
impl Ownership {
    fn update(&mut self, current: i32, receive: bool) -> Option<i32> {
        if receive {
            self.previous.get_or_insert(current);
            (current != 0).then_some(0)
        } else {
            let previous = self.previous.take()?;
            // Leave an explicit change made by another plugin alone.
            (current == 0 && previous != 0).then_some(previous)
        }
    }
}

#[derive(Default)]
pub(super) struct NativeAtis {
    enabled: Option<DataRef<i32, ReadWrite>>,
    ownership: Ownership,
}
impl NativeAtis {
    pub fn bind(&mut self, data: &mut xplane::data::DataApi) {
        self.update(false);
        self.enabled = data
            .find("sim/atc/atis_enabled")
            .ok()
            .and_then(|value: DataRef<i32, ReadOnly>| value.writeable().ok());
    }
    pub fn update(&mut self, receive: bool) {
        if let Some(enabled) = self.enabled.as_mut()
            && let Some(value) = self.ownership.update(enabled.get(), receive)
        {
            enabled.set(value);
        }
    }
}
impl Drop for NativeAtis {
    fn drop(&mut self) {
        self.update(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_native_override_covers_both_receivers_and_restores_once() {
        let mut state = Ownership::default();
        assert_eq!(state.update(1, true), Some(0));
        assert_eq!(state.update(0, true), None);
        assert_eq!(state.update(0, true), None);
        assert_eq!(state.update(0, false), Some(1));
        assert_eq!(state.update(1, false), None);
    }

    #[test]
    fn preserves_disabled_native_atis_and_external_changes() {
        let mut state = Ownership::default();
        assert_eq!(state.update(0, true), None);
        assert_eq!(state.update(0, false), None);
        assert_eq!(state.update(1, true), Some(0));
        assert_eq!(state.update(1, false), None);
    }
}
