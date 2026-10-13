# Flight stories

Open **Traffic and missions → Missions** in the header. Choose a story before taxi, with the matching aircraft loaded. The browser filters by ICAO aircraft type, such as `A320` or `A20N`; parking category C is not an aircraft type. Suggested airports and durations help you choose a short flight. They do not file a plan, check current weather or establish that a procedure is available. Import or prepare your own IFR plan.

These stories simulate cabin events. They do not create real passengers, change aircraft systems, diagnose illness or teach medical treatment. For urgent incidents, coordinate with the cabin and make an explicit ATC emergency or urgency call as appropriate. Selecting a story response does not declare an emergency, change the destination or authorize a diversion. The controller still validates clearances and runway crossings.

## Editing and sharing

Use **Edit a copy**, then **Mission editor**. The form edits aircraft types, briefing, phase, delay, chance, urgency, seat, dependencies, dialogue and choices. **Advanced TOML source** also adds or removes events, choices and spoken aliases. Invalid drafts remain editable; **Validate and save** refuses invalid content.

Saved missions live in the engine's personal configuration directory under `missions/`. The catalogue loads these sources in order; a later file with the same mission ID overrides an earlier definition:

1. Built-in examples.
2. The installed plugin's `missions/` folder beside `bin/`.
3. `OPENATC_MISSIONS_DIR`, if set on the engine.
4. The personal configuration directory's `missions/` folder.

Refresh the catalogue after editing a file. Starting a mission takes a snapshot of its definition, so editing the file does not change an event halfway through a flight. New flight clears mission progress. The journal records events, skipped probability checks, choices and arrival at parking.

## Format

```toml
version = 1
id = "cabin-arrival-practice"
title = "A short cabin coordination flight"
briefing = "A fictional cabin update during a short IFR flight. Prepare your own current flight plan."
aircraft = ["A320", "A20N"]
departure = "KLAX"
destination = "KLGB"
duration_minutes = 25
media = []

[[events]]
id = "cabin_update"
phase = "cruise"
delay_seconds = 30
probability = 100
severity = "normal"
say = [
  "Captain, the cabin is settled. Please let us know when to prepare for arrival.",
  "Captain, everyone is settled for now. We are standing by for your arrival briefing.",
]

[[events.choices]]
id = "prepare"
label = "Please prepare the cabin for arrival."
aliases = ["Prepare for arrival"]
outcome = "preparing"
say = ["Of course, Captain. We will begin the arrival preparations."]
```

Supported phases: `parked`, `clearance`, `pushback`, `taxi`, `departure`, `cruise`, `arrival`, `approach`, `landed`, `taxi_in`. The delay counts unpaused time in that phase; an event waits for a quiet operational exchange. An event fires at most once. Chance is checked once when the event becomes eligible: 0 never fires and 100 always fires. Very short flights may never spend time in cruise; use a phase your flight actually reaches.

Add `after_event = "cabin_update"` and `after_outcome = "preparing"` to a later event to create a branch. Its delay still counts time in its chosen phase, not time since the response. A prerequisite must reference an earlier event and one of its defined outcomes. One event at a time waits for a captain's response. Choice IDs, full labels and explicit aliases work through the cabin channel; buttons use the same handler.

`severity` is `normal`, `urgent` or `emergency`. Urgent incident context remains available to the crew until the mission ends, including after a response. Routine jokes are suppressed while that context is active. A mission incident allows an explicit ATC emergency declaration even if practice emergencies are otherwise disabled; it does not declare one automatically.

`say` contains alternatives. With variety disabled the first line is used; with variety enabled an alternative is selected. `{seat}` uses the event's `seat`, such as `12A`. `{passengers}` uses the plan's passenger count, or `unconfirmed` when unavailable. These are the only substitution slots.

Limits keep user content bounded: 256 KiB per file, 64 events, 8 choices per event, 24 alternatives per line list, and 8 media references per event or briefing. IDs use lowercase letters, digits, hyphens or underscores. Unknown fields and unsupported slots are rejected.

## Future media

The editor can retain optional image, audio and video references, with a required caption or transcript:

```toml
[[media]]
kind = "video"
source = "assets/briefing.mp4"
transcript = "A text version of the briefing."
```

Use relative asset paths or HTTPS URLs. Parent-directory traversal and other URL schemes are rejected. This version stores and validates the references; it **does not download or play mission media**. All bundled stories work with text and synthesized dialogue alone.

## Included stories

| Story | Suggested route | Focus |
| --- | --- | --- |
| Coastal medical diversion | KLAX–KLGB | Serious passenger illness, intentions and follow-up |
| New arrival | YSSY–YSCB | Unexpected childbirth and cabin coordination |
| Rough air | KSFO–KMRY | Turbulence preparation |
| Parts shuttle | EGKK–EGHI | Short transport assignment |
| Nervous flyer | LFMN–LFML | Reassurance and passenger updates |
| Island reunion | LEPA–LEIB | Routine passenger service |
| Cabin smoke | NZCH–NZWN | Urgent cabin report and pilot decisions |
| Assisted arrival | YMML–YMHB | Passenger assistance on arrival |

The routes are suggestions, not validated operational dispatches. Complete diversion routing, automatic assistance dispatch, and mission media playback remain future work.
