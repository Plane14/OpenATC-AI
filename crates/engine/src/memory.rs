//! Flight-local request history. Requests and verified actions are separate facts.
use openatc_core::{crew::Action, state::PhaseCode};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

#[derive(Default)]
pub struct RequestMemory {
    next: u64,
    recent: VecDeque<Record>,
    counts: BTreeMap<String, u32>,
    confirmations: BTreeMap<u32, bool>,
}

#[derive(Serialize)]
struct Record {
    id: u64,
    role: String,
    phase: PhaseCode,
    request: String,
    outcome: String,
    reply: String,
    actions: Vec<ActionResult>,
}

#[derive(Serialize)]
struct ActionResult {
    sequence: u32,
    control: String,
    target: f64,
    status: &'static str,
}

impl RequestMemory {
    pub fn begin(&mut self, role: &str, phase: PhaseCode, text: &str) -> u64 {
        self.next = self.next.wrapping_add(1).max(1);
        let key = format!(
            "{role}:{}",
            text.to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
        let key = if self.counts.len() < 512 || self.counts.contains_key(&key) {
            key
        } else {
            format!("{role}:other requests")
        };
        let count = self.counts.entry(key).or_default();
        *count = count.saturating_add(1);
        // Never discard a request whose simulator acknowledgement is still pending.
        if self.recent.len() >= 256
            && let Some(i) = self.recent.iter().position(|r| r.outcome != "pending")
        {
            self.recent.remove(i);
        }
        self.recent.push_back(Record {
            id: self.next,
            role: role.into(),
            phase,
            request: text.chars().take(1000).collect(),
            outcome: "pending".into(),
            reply: String::new(),
            actions: Vec::new(),
        });
        self.next
    }

    pub fn finish(&mut self, id: u64, response: &Value, actions: &[Action]) {
        let Some(record) = self.recent.iter_mut().find(|r| r.id == id) else {
            return;
        };
        let result = &response["result"];
        record.reply = result["message"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(1000)
            .collect();
        record.outcome = if result["pending"] == true {
            "pending"
        } else if result["silent"] == true {
            "not received on radio"
        } else if result["accepted"] == true {
            "answered"
        } else {
            "not accepted"
        }
        .into();
        if result["pending"] == true {
            record.actions = actions
                .iter()
                .map(|a| ActionResult {
                    sequence: a.sequence,
                    control: a.control.clone(),
                    target: a.value,
                    status: "pending",
                })
                .collect();
        }
        let confirmed: Vec<_> = record
            .actions
            .iter()
            .filter_map(|a| {
                self.confirmations
                    .get(&a.sequence)
                    .map(|ok| (a.sequence, *ok))
            })
            .collect();
        for (sequence, success) in confirmed {
            self.confirm(sequence, success);
        }
    }

    pub fn confirm(&mut self, sequence: u32, success: bool) {
        self.confirmations.insert(sequence, success);
        if self.confirmations.len() > 128 {
            self.confirmations.pop_first();
        }
        for record in &mut self.recent {
            if let Some(action) = record.actions.iter_mut().find(|a| a.sequence == sequence) {
                action.status = if success {
                    "simulator confirmed"
                } else {
                    "not confirmed"
                };
                if record.actions.iter().all(|a| a.status != "pending") {
                    record.outcome = if record
                        .actions
                        .iter()
                        .all(|a| a.status == "simulator confirmed")
                    {
                        "controls confirmed"
                    } else {
                        "one or more controls not confirmed"
                    }
                    .into();
                }
                return;
            }
        }
    }

    pub fn cancel_pending(&mut self) {
        self.confirmations.clear();
        for record in &mut self.recent {
            if record.outcome == "pending" {
                record.outcome = "cancelled; not confirmed".into();
                for action in &mut record.actions {
                    if action.status == "pending" {
                        action.status = "cancelled; not confirmed";
                    }
                }
            }
        }
    }

    pub fn context(&self, role: &str) -> Value {
        let recent: Vec<_> = self
            .recent
            .iter()
            .rev()
            .filter(|r| r.role == role)
            .take(12)
            .collect();
        let repeated: Vec<_> = self
            .counts
            .iter()
            .filter(|(key, count)| key.starts_with(&format!("{role}:")) && **count > 1)
            .take(20)
            .map(|(request, count)| json!({"request":request,"count":count}))
            .collect();
        json!({"recentRequests":recent,"repeatedRequests":repeated})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_simulator_acknowledgement_is_not_lost() {
        let mut memory = RequestMemory::default();
        let id = memory.begin("ground", PhaseCode::Parked, "remove chocks");
        memory.confirm(7, true);
        memory.finish(
            id,
            &json!({"result":{"pending":true}}),
            &[Action {
                sequence: 7,
                control: "chocks".into(),
                ..Default::default()
            }],
        );
        assert_eq!(
            memory.context("ground")["recentRequests"][0]["outcome"],
            "controls confirmed"
        );
    }
    #[test]
    fn request_is_not_confirmation_and_partial_failure_is_retained() {
        let mut memory = RequestMemory::default();
        let id = memory.begin(
            "ground",
            PhaseCode::Parked,
            "remove chocks and external power",
        );
        let actions = [
            Action {
                sequence: 1,
                control: "chocks".into(),
                ..Default::default()
            },
            Action {
                sequence: 2,
                control: "external_power".into(),
                ..Default::default()
            },
        ];
        memory.finish(id, &json!({"result":{"pending":true}}), &actions);
        memory.confirm(1, true);
        assert_eq!(
            memory.context("ground")["recentRequests"][0]["outcome"],
            "pending"
        );
        memory.confirm(2, false);
        let record = &memory.context("ground")["recentRequests"][0];
        assert_eq!(record["actions"][0]["status"], "simulator confirmed");
        assert_eq!(record["actions"][1]["status"], "not confirmed");
        assert_eq!(record["outcome"], "one or more controls not confirmed");
        assert_eq!(memory.context("cabin")["recentRequests"], json!([]));
    }
}
