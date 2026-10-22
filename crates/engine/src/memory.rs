//! Flight-local request history. Requests and verified actions are separate facts.
use openatc_core::{crew::Action, state::PhaseCode};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

/// Flight-local social evidence, kept separately from aircraft permissions.
#[derive(Default)]
pub struct SocialMemory {
    quotes: BTreeMap<String, VecDeque<String>>,
    replies: BTreeMap<String, VecDeque<String>>,
    jokes: VecDeque<String>,
}

impl SocialMemory {
    pub fn clear(&mut self) {
        self.quotes.clear();
        self.replies.clear();
        self.jokes.clear();
    }

    pub fn remember(&mut self, role: &str, sources: &[String], proposed: &[String]) {
        if !["ground", "cabin", "copilot"].contains(&role) {
            return;
        }
        let quotes = self.quotes.entry(role.to_owned()).or_default();
        for quote in proposed.iter().take(12).map(|quote| quote.trim()) {
            if quote.chars().count() < 4
                || quote.chars().count() > 240
                || !sources.iter().any(|source| source.contains(quote))
            {
                continue;
            }
            let mut evidence = quote.to_owned();
            if quote.chars().count() < 40
                && let Some(source) = sources.iter().find(|source| source.contains(quote))
            {
                let index = source.find(quote).unwrap();
                let start = source[..index]
                    .rfind(['.', '!', '?', '\n'])
                    .map_or(0, |index| index + 1);
                let end = source[index + quote.len()..]
                    .char_indices()
                    .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n'))
                    .map_or(source.len(), |(offset, c)| {
                        index + quote.len() + offset + c.len_utf8()
                    });
                let sentence = source[start..end].trim();
                if sentence.chars().count() <= 240 {
                    evidence.clear();
                    evidence.push_str(sentence);
                }
            }
            quotes.retain(|previous| previous != &evidence);
            quotes.push_back(evidence);
            while quotes.len() > 128 {
                quotes.pop_front();
            }
        }
    }

    pub fn record_reply(&mut self, role: &str, reply: &str) {
        if !["ground", "cabin", "copilot"].contains(&role) {
            return;
        }
        let replies = self.replies.entry(role.to_owned()).or_default();
        replies.push_back(reply.chars().take(600).collect());
        while replies.len() > 8 {
            replies.pop_front();
        }
    }

    pub fn record_joke(&mut self, joke: &str) {
        if joke.trim().is_empty() {
            return;
        }
        self.jokes.push_back(joke.chars().take(600).collect());
        while self.jokes.len() > 128 {
            self.jokes.pop_front();
        }
    }

    pub fn repeated_joke(&self, candidate: &str) -> bool {
        let words = |text: &str| {
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let candidate = words(candidate);
        if candidate.len() < 4 {
            return false;
        }
        self.jokes.iter().any(|previous| {
            let full = words(previous);
            let punchline = previous
                .split_once('?')
                .map_or(previous.as_str(), |(_, answer)| answer)
                .split(['.', '!', '\n'])
                .find(|part| !part.trim().is_empty())
                .unwrap_or(previous);
            let punchline = words(punchline);
            candidate == full
                || (punchline.len() >= 4
                    && candidate
                        .windows(4)
                        .any(|part| part == &punchline[punchline.len() - 4..]))
        })
    }

    pub fn context(&self, role: &str, request: &str) -> Value {
        let words = request
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() > 2)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut quotes = self
            .quotes
            .get(role)
            .into_iter()
            .flatten()
            .rev()
            .collect::<Vec<_>>();
        quotes.sort_by_key(|quote| {
            std::cmp::Reverse(
                words
                    .iter()
                    .filter(|word| quote.to_lowercase().contains(word.as_str()))
                    .count(),
            )
        });
        let quotes = quotes.into_iter().take(12).cloned().collect::<Vec<_>>();
        let replies = self
            .replies
            .get(role)
            .into_iter()
            .flatten()
            .rev()
            .take(4)
            .cloned()
            .collect::<Vec<_>>();
        json!({"captainQuotesNewestFirst":quotes,"recentCrewRepliesNewestFirst":replies,"recentJokesNewestFirst":self.jokes.iter().rev().take(8).map(|joke|joke.chars().rev().take(200).collect::<String>().chars().rev().collect::<String>()).collect::<Vec<_>>()})
    }
}

#[derive(Default)]
pub struct RequestMemory {
    next: u64,
    recent: VecDeque<Record>,
    counts: BTreeMap<String, u32>,
    confirmations: BTreeMap<u32, bool>,
    completed_replies: BTreeMap<u32, String>,
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
    /// An aircraft change invalidates crew continuity without erasing radio history.
    pub fn forget_crew(&mut self) {
        self.cancel_pending();
        self.recent
            .retain(|record| !matches!(record.role.as_str(), "ground" | "cabin" | "copilot"));
        self.counts.retain(|key, _| {
            !matches!(key.split(':').next(), Some("ground" | "cabin" | "copilot"))
        });
        self.confirmations.clear();
        self.completed_replies.clear();
    }

    /// Pair the actual spoken acknowledgement with the request that produced its action.
    pub fn reply_for_action(&mut self, sequence: u32, reply: &str) {
        let reply = reply.chars().take(1000).collect::<String>();
        if let Some(record) = self.recent.iter_mut().find(|record| {
            record
                .actions
                .iter()
                .any(|action| action.sequence == sequence)
        }) {
            record.reply = reply;
        } else {
            self.completed_replies.insert(sequence, reply);
            while self.completed_replies.len() > 256 {
                self.completed_replies.pop_first();
            }
        }
    }

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
        for action in &record.actions {
            if let Some(reply) = self.completed_replies.remove(&action.sequence) {
                record.reply = reply;
            }
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
    fn aircraft_change_drops_crew_history_but_retains_controller_context() {
        let mut memory = RequestMemory::default();
        memory.begin("copilot", PhaseCode::Cruise, "My sister is Nora.");
        memory.begin("ground", PhaseCode::Parked, "Miso is my dog.");
        memory.begin("atc", PhaseCode::Cruise, "Request climb.");
        memory.forget_crew();
        for role in ["ground", "cabin", "copilot"] {
            assert!(
                memory.context(role)["recentRequests"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(
            memory.context("atc")["recentRequests"][0]["request"],
            "Request climb."
        );
    }
    #[test]
    fn social_memory_retains_only_exact_evidence_and_retrieves_it_after_long_chats() {
        let mut memory = SocialMemory::default();
        let source = "My sister’s name is Nora and we are meeting in Hobart.".to_owned();
        memory.remember(
            "copilot",
            std::slice::from_ref(&source),
            &[source.clone(), "My sister is Emily".into()],
        );
        for index in 0..100 {
            let text = format!("My unrelated preference number {index} is blue.");
            memory.remember(
                "copilot",
                std::slice::from_ref(&text),
                std::slice::from_ref(&text),
            );
        }
        let context = memory.context(
            "copilot",
            "What is my sister’s name and where are we meeting?",
        );
        assert_eq!(context["captainQuotesNewestFirst"][0], source);
        assert!(!context.to_string().contains("Emily"));
        assert!(
            memory.context("cabin", "sister")["captainQuotesNewestFirst"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        memory.clear();
        assert!(
            memory.context("copilot", "sister")["captainQuotesNewestFirst"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn short_name_quotes_keep_their_source_relation_and_joke_comparison_ignores_shared_setups() {
        let mut memory = SocialMemory::default();
        let source = "My sister’s name is Nora and we are meeting in Hobart.".to_owned();
        memory.remember("copilot", std::slice::from_ref(&source), &["Nora".into()]);
        assert_eq!(
            memory.context("copilot", "sister")["captainQuotesNewestFirst"][0],
            source
        );
        memory.record_joke("Why did the airplane visit the doctor? It had a runway nose.");
        assert!(
            memory.repeated_joke("WHY did the airplane visit the doctor? It had a runway nose!")
        );
        assert!(!memory.repeated_joke(
            "Why did the airplane bring a suitcase? It was carrying emotional baggage."
        ));
        memory.record_joke("Why don't scientists trust atoms? Because they make up everything.");
        assert!(memory.repeated_joke("I never trust an atom, because they make up everything!"));
        assert!(memory.repeated_joke(
            "Why don't scientists trust atoms? Because they make up everything. Enjoy the flight!"
        ));
    }

    #[test]
    fn social_evidence_is_bounded_and_newer_corrections_are_first() {
        let mut memory = SocialMemory::default();
        for index in 0..160 {
            let text = format!("My meeting is now at place {index}.");
            memory.remember(
                "ground",
                std::slice::from_ref(&text),
                std::slice::from_ref(&text),
            );
            memory.record_reply("ground", &"é".repeat(900));
        }
        assert_eq!(memory.quotes["ground"].len(), 128);
        assert_eq!(memory.replies["ground"].len(), 8);
        assert_eq!(
            memory.context("ground", "meeting")["captainQuotesNewestFirst"][0],
            "My meeting is now at place 159."
        );
        assert_eq!(memory.replies["ground"][0].chars().count(), 600);
    }

    #[test]
    fn early_simulator_acknowledgement_is_not_lost() {
        let mut memory = RequestMemory::default();
        let id = memory.begin("ground", PhaseCode::Parked, "remove chocks");
        memory.confirm(7, true);
        memory.reply_for_action(7, "Chocks removed. Miso is still enjoying the garden.");
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
        assert_eq!(
            memory.context("ground")["recentRequests"][0]["reply"],
            "Chocks removed. Miso is still enjoying the garden."
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
