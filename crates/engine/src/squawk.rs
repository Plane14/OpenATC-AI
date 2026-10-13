//! Discrete codes for simulated IFR flights, retained across controller handoffs.
use rand::Rng;
use std::collections::VecDeque;

#[derive(Default)]
pub struct Allocator {
    recent: VecDeque<u16>,
}

impl Allocator {
    pub fn next(&mut self) -> String {
        // This is a simulator pool, not a country's operational code allocation.
        let start = rand::rng().random_range(0o1001..0o7000);
        let code = (0..0o5777)
            .map(|offset| 0o1001 + (start - 0o1001 + offset) % 0o5777)
            .find(|code| !matches!(code, 0o1200 | 0o2000 | 0o4000) && !self.recent.contains(code))
            .expect("the recent history is smaller than the code pool");
        self.recent.push_back(code);
        if self.recent.len() > 256 {
            self.recent.pop_front();
        }
        format!("{code:04o}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn codes_are_octal_and_do_not_repeat_between_recent_flights() {
        let mut allocator = super::Allocator::default();
        let mut codes = std::collections::BTreeSet::new();
        for _ in 0..256 {
            let code = allocator.next();
            assert_eq!(code.len(), 4);
            assert!(code.bytes().all(|b| (b'0'..=b'7').contains(&b)));
            assert!(
                ![
                    "0000", "1000", "1200", "2000", "4000", "7000", "7500", "7600", "7700"
                ]
                .contains(&code.as_str())
            );
            assert!(codes.insert(code));
        }
    }
}
