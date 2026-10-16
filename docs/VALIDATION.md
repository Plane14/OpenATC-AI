# Validation

## Release build gate repairs — 16 October 2026

The shipped-configuration test now validates transponder settings with their own loader and leaves callout policy validation to the plugin tests; other aircraft TOML files still undergo strict aircraft-profile validation. Role instruction catalogues and text copies now agree on trailing newlines. The exact shared CI test command passes locally: 415 tests passed, 25 environment-dependent tests ignored, with no failures. Formatting also passes.

Manual native preflight run [38813386715](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38813386715) on revision `652c680` passed Linux x64, Intel Mac and Apple Silicon packaging checks. Windows failed before compilation because Chocolatey could not find the requested Inno Setup 6.4.3 package. Both Windows jobs now locate the compiler supplied by the Windows 2022 runner, export its absolute path through `ISCC` and let the actual installer preflight validate compatibility, without a Chocolatey download or downgrade. The subsequent Windows-only run [38814250978](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38814250978) on `4c12d69` found the compiler but rejected its zero file-version metadata before compilation. That metadata check is now removed: the actual fixture compilation validates compiler compatibility and reports its engine version. Workflow YAML and identical compiler-selection blocks pass local structural checks; Windows compilation still requires a fresh native preflight on this revision. Windows-only run [38814868497](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38814868497) on `d0e80cd` passed compiler discovery and installer tests, then failed linking the packaging executable because Git Bash selected Git’s Unix `link.exe`. Preflight now pins the Visual Studio linker by absolute path and runs Windows packaging in PowerShell, matching the full native build’s linker selection. That correction still requires a native rerun. These fixture checks do not establish application installation or runtime acceptance. Independent preflight results are retained when another platform fails. Manual full workflow runs upload artifacts without publishing a tag or release; see `RELEASE_PACKAGING.md`.

## Native Windows VM packaging checks — 16 October 2026

A Windows 11 x64 VM now has Visual Studio Build Tools 2022 (17.14.41), Rust 1.99.1 with the MSVC toolchain and Inno Setup 6.7.3. All five installer discovery/transaction tests pass natively. Initial fixture compilation exposed quotes around individual wizard-image list entries being treated as filename characters; the template now uses the compiler's comma-separated path format. Both plugin and AI-server installer fixtures then compile successfully with the real Inno compiler. Compiler discovery also covers per-user installations, as used on this VM, alongside machine installations.

This establishes native fixture compilation, not installation, upgrades, removal, signing or simulator acceptance. The locked native release build of plugin, engine and packaging tools also passes, followed by all four real HTTP-engine flight replay arrangements (separate controller duties, Ground/Tower, combined Tower, and COM2). A real plugin Setup.exe and ZIP were then generated successfully for interactive installer review. The AI-server check is still a fixture compile; its actual runtime build is separate. Hosted Windows preflight must still run against the latest committed corrections.

## Crew request and context checks — 16 October 2026

Shared requests explicitly reserve 512 output tokens for plain and structured replies. Dedicated crew dialogue uses compact role/fact context and ordered user/assistant exchanges, independently of the current-message control interpreter. Token counting and generation use the same message list. Context checks remove older complete exchanges and optional evidence; an essential request that cannot fit is refused. Role-scoped continuity retains bounded exact captain quotes, with relevance retrieval and flight/aircraft resets. Generated social summaries do not grant aircraft authority. Two current-request interpretations agree on control proposals; disagreements receive a separate quoted-source check. Rust retains profile, range, observation freshness and simulator acknowledgement requirements.

Focused regressions cover long-history retrieval, context exhaustion, stale flight/aircraft completions, current-message authority, requested jokes with unrelated questions, delayed observations, identical budget/generation framing and early simulator acknowledgements. Mixed conversational replies are paired with their eventual acknowledged result in request history. A joke retry retains the factual answer; normalized joke boundaries prevent trailing whitespace from causing the pacing filter to remove a setup. Conversation failures report unavailable rather than inventing an acknowledgement.

The expanded real Qwen2.5 Instruct fixture has 13 mixed-action, conversation and recall cases. Some runs pass every action and required-word recall assertion, but manual review still finds omitted requests, personal-pronoun errors, repeated control claims and unsuitable filler in other runs. Those assertions are not full conversation acceptance. The fixture now also checks that the sandwich inside the first mixed door/lighting sentence receives an answer. Real-model fixtures use synthetic observations and acknowledgements; they do not establish native aircraft control or audio acceptance.

Nine additional real-model samples cover a 43-minute flight before departure, in cruise and after landing, plus an unspecified duration, across the available crew roles. Initial runs exposed the copilot treating TaxiIn as waiting for takeoff and inventing an hour when duration was absent. With readable phase names, explicit current ground/airborne/after-landing facts, total planned minutes and an explicit unavailable-duration note, the final rerun describes those states and short duration correctly and acknowledges missing duration. Ground is sampled only before departure; separate connection tests enforce headset disconnection during taxi/flight. This is sampled wording coverage, not a guarantee over every future reply or missing-telemetry condition.

The current focused conversation-routing suite passes 19 tests, with the live-server test opt-in; request-memory six, shared transport six and narration two tests pass. Crew, radio and four clearance-to-parking HTTP-engine replay arrangements pass. The latest pipeline build is installed for Linux testing with a recoverable backup of the preceding installation. It includes the earlier flight repairs; conversation quality and native acceptance remain open. Native speech, aircraft controls, frame time and Windows/macOS acceptance remain separate requirements. Copilot turns in the real-server samples still take substantially longer than simple cabin/ramp dialogue; latency and the manual conversation-quality findings remain open. The strengthened five-case cabin rerun still fails two conversational-coverage checks despite correct aircraft actions, so it does not qualify as complete pipeline acceptance.

## Focused flight checks — 16 October 2026

Fifteen targeted tests passed for window layout, advance speed notices and crossing warnings, compound instructions, crew pacing and ramp disconnection, taxiway pronunciation, callout thresholds, descent readbacks and ATIS reception. The installed Linux plugin matches the tested release artifact. Native window transitions, audible callout latency/overlap and taxiway pronunciation remain simulator acceptance checks.

## Flight repairs — 16 October 2026

The latest Linux build includes the following repairs:

- Restore saved floating-window bounds after applying UI-scale limits; preserve the compact/full anchor and splitter state.
- Combine descent clearances with due published speed restrictions, including the crossing fix and exact or maximum speed qualifier.
- Validate complete level/speed readbacks against the current clearance sequence, including spoken numbers and climb/descent direction.
- Avoid reporting a climb back to the ILS intercept altitude during final Tower check-in.
- Retain editable crew pacing guidance, taxiway pronunciation, ATIS reception updates and independent takeoff-callout playback.

The locked Linux release build, focused readback/descent tests and four HTTP flight-replay arrangements pass. The replays cover a published STAR speed restriction, descent, approach clearance, Tower transfer, landing, runway exit, crossing and parking. Native window, speech and aircraft-specific checks for this build remain pending. A previous ToLiss Nice–Milan flight through parking was completed successfully. Generated traffic and the wider AI pipeline remain in development; generated traffic is disabled by default.

Operational wording is in `speech/runtime/responses.toml`. Full-file overrides must retain the required situation IDs and slot contracts; compare customised files with the shipped defaults when upgrading.

## Moving nearby display selection — 15 October 2026 continuation

Actual generated::step now uses generated/display.rs to publish a bounded nearby subset to native rendering/TCAS, based on CURRENT valid player position. Editable operations.toml defaults: visible_aircraft_limit63 (SDK hard maximum63), display_enter_nm40, display_leave_nm50, display_replacement_margin_nm2. Existing selected IDs can remain to the outer radius; a small distance advantage avoids slot churn but a substantially nearer target replaces them. Stable IDs resolve ties, duplicate/invalid coordinates reject and invalid own position clears display selection. Successful scene replacement clears retained visibility. Older personal operation files inherit the new values; invalid count/ranges/NaN/margins reject.

Only session.generated_traffic is culled. Runtime.frames and the movement world retain every prepared aircraft, and ALL generated targets continue into engine separation/wake observations even when visually absent. Existing renderer removes absent instances and unused assets; re-entering flights preserve their ID/model/state. This fixes display slot turnover; it does NOT implement new-flight generation/replenishment, move a flight to another route, or establish native pop-in/FPS/TCAS acceptance. Current prepare_scene still prepares origin/destination-bound surface aircraft; production Runtime.flights remains empty. Generating credible flights ahead/enroute/destination and managing persistent lifecycle retirement/re-entry remain required.

Focused checks cover production display turnover, retained distant observations, range and capacity hysteresis, stable ties and invalid coordinates. Generated-flight permission, recovery, crossing, runway-report and worker-reset checks also pass. Release engine/plugin builds pass; native visibility, audio, FPS and platform acceptance remains open.

Remaining integration includes aircraft-specific decoded terminals and joined nominal surfaces, protected airborne trajectories, feasible speed schedules and recovery at every clearance boundary. Complete flights also require typed controller clearances, radio/taxi spans, landing/gate ownership and atomic installation. Protection and recovery must be real planning inputs. Plausible routes, operators, hubs, fleets, liveries and callsigns remain required, alongside wake/separation, GA VFR, pilot initiative/chat, animation and native/platform acceptance.

## Selected runway surfaces and joined nominal arrivals — 15 October 2026 continuation


Production preparation now constructs generated/runway.rs::Surface from BOTH selected physical runway-end elevations, displaced-threshold distances and installed runway geometry. Missing/conflicting/invalid elevations reject, as do slopes beyond editable operations.toml runway_slope_limit_percent (default2%). Reciprocal directions agree at the same physical point. Airport maximum elevation is not the runway datum. This is a NOMINAL linear surface, not surveyed pavement or native terrain proof.

Actual prepare_scene now calls landing::prepare for successful aircraft-specific decoded inputs, retaining nominal_arrivals by traffic ID. The real published final threshold crossing stays airborne. Its published descent angle leads into a gradual parabolic roundout to the selected sloping runway, then the existing protected rollout/rounded exit and compatible free-stand taxi route. The entire threshold-to-parking sequence is compiled together to check all joins/rate/budgets before retaining it. Nominal prepared arrivals cannot select the same stand. This is not runtime gate ownership, landing permission, a complete flyable flight, or a missed approach. Expired endpoint weather clears both decoded inputs and nominal arrivals.

handling.toml supplies flare_height_feet/touchdown_sink_fpm per piston, turboprop, regional jet, jet and heavy jet. Older override files inherit these two fields from bundled category defaults; ranges validated, no literal wording/default policy table in Rust. Chosen handling numbers are nominal family visual envelopes, NOT aircraft-certified landing technique or performance. Flare currently keeps constant GS, so weight, detailed energy/speed loss and native animation/terrain acceptance remain open. Outside the runway, exit heights use the projected runway datum; taxi graph nodes have no surveyed heights, so the taxi plan retains the exit datum continuously. Do not describe that as terrain following.

The departure producer had retained airport maximum height after the arrival repair. Actual departure::plan now requires the same selected runway Surface and fresh Environment; rotation speed/roll length converge against density at the resulting rotation height. The runway roll uses nominal surface heights, while lineup interpolates from the exact taxi-hold datum to the aligned runway point. Existing pavement, entry steering, remaining runway and named-resource checks remain. Unknown/mismatched surface, stale weather, unconverged speed or excessive rate rejects. No new movement grant.

Focused checks cover reciprocal and displaced runway heights, invalid inputs, five aircraft-family roundouts and older handling overrides. Installed Nice A320 and Cannes C172 fixtures cover joined arrivals and runway-exit geometry. The integrated Nice/Milan/Bluebell fixture creates jet and turboprop nominal arrivals with distinct stands, continuous threshold/flare/rollout/exit/taxi joins, weather-based departure speed and synthetic sloping-runway departure geometry. Weather is explicitly synthetic and calm. Release builds pass; this does not establish native terrain, aircraft performance or rendering acceptance.

Primary sources reviewed for the preceding flare work: FAA Airplane Flying Handbook ch9 https://www.faa.gov/sites/faa.gov/files/regulations_policies/handbooks_manuals/aviation/airplane_handbook/10_afh_ch9.pdf and ch16 https://www.faa.gov/sites/faa.gov/files/regulations_policies/handbooks_manuals/aviation/airplane_handbook/17_afh_ch16.pdf; Airbus landing guidance https://safetyfirst.airbus.com/app/themes/mh_newsdesk/documents/archives/a320-prevention-of-tailstrikes.pdf . These support a gradual roundout and different aircraft handling, not certification of these nominal defaults. No new radio wording/procedural separation minimum was added.

Production still does not populate Runtime.flights. Complete integration requires protected SID/enroute/STAR/approach trajectories, feasible speed/performance schedules, published missed approaches and flyable recovery at every airborne authority boundary, including flare and late go-around. Typed station-duty clearances, permissions, taxi spans, runway reports, runtime stand ownership and atomic installation also remain necessary. Flare stays airborne until touchdown; protection and recovery cannot be bypassed or published fixes discarded. Wake/separation, regional policy, GA VFR, continuous traffic coverage, distinct routes, pilot initiative/chat and native rendering/audio/FPS/platform acceptance remain open.

## Production aircraft terminal inputs — 15 October 2026

The actual scene preparation worker now decodes selected SID/STAR/approach records for each model-compatible aircraft from its real nominal rotation point, own aircraft envelope and fresh endpoint weather. It retains bounded typed inputs by traffic ID, checks equipment/ceiling/coordinate joins and preserves published restrictions and IAS/weather metadata. Selected-runway threshold elevations prevent the airport maximum height from incorrectly rejecting a lower final crossing; missing or conflicting threshold records reject. `terminal_intercept_limit_nm` is editable with backward-compatible policy defaults.

The installed Nice/Milan mixed jet/turboprop data fixture passes 1, synthetic airborne checks5, scene checks5 with2licensed-dataignored, policy1 and release engine build pass.

These are production constraint inputs, not a complete flyable flight factory or native acceptance. Joint terrain-protected trajectories, speed scheduling, every-boundary recovery, threshold-to-touchdown/flare and runway surface elevations, landing/exit/taxi/gate/controller assembly remain unfinished. Generated traffic stays disabled by default; installed build unchanged.

## Generated taxi clearance limits — 15 October 2026

Complete-flight taxi clearances can now explicitly cover several contiguous physical TaxiIn segments with one readback. Validation rejects a span across another instruction, controller, crossing, runway report or parking limit. The entire covered corridor is reserved before transmission, without moving the aircraft, and only the completed readback grants its movements. A competing aircraft cannot reserve the downstream lane; movements beyond the stated limit remain available.

Source checks: motion 11 passed, full-flight 7 passed, legacy generated 5 passed, 2 licensed-data tests ignored; release engine build passed. No installation or native acceptance. The actual production full-flight factory remains unwired and must construct these spans from real ground orders alongside protected airborne/recovery/landing planning. Generated traffic remains disabled by default; mixed-fleet and GA-VFR acceptance remains pending.

## Generated full-flight permissions and runway occupancy — 15 October 2026

A stronger negative movement test reproduced taxi-in starting without a readback after complete-flight installation removed legacy ground orders. The worker now requires explicit full-flight movement permissions and prevents adjacent taxi edges from inheriting a prior segment's approval. Crossings obtain the correct airport from the complete flight, retain duration-based conflict checks, and cancel/reissue when conflicting traffic appears during a readback or before entry. Typed runway reports must match a real protected runway before installation. External native/player runway occupancy now blocks the named runway resource as well as taxi cells, preventing a landing grant through an occupied runway.

The regression reproduces the former permission failure. Six checks pass, including the synthetic arrival crossing, runway report, controller and movement flow. Five legacy checks pass with two licensed-data checks ignored; the release engine builds. These results do not supply the missing production full-flight factory or establish native traffic, audio, FPS or platform acceptance. Generated traffic remains disabled by default.

## Generated arrival reports and runway exit clearance — 15 October 2026

The complete-flight consumer now requires typed runway-exit reports and emits the existing editable pilot report once on the runway controller frequency before Ground handoff or taxi-in permission. Observed aircraft dimensions and published holding lines gate the report; unknown or occupied runway states cannot progress. Landing preparation uses that same clearance geometry for its graph endpoint and rounded exit, so a point just outside pavement cannot be mistaken for a fully vacated aircraft. Editable aircraft dimensions are retained instead of an unknown-ICAO fallback.

Focused controller/movement/transcript checks pass 3; installed Nice A320 and Cannes C172 arrival-to-parking data fixtures pass 2. The separate holding-line check passes 1, covering the tail still inside the clearance boundary. Release engine build passes; diffcheck clean. Source only; no installation or native acceptance. The actual production full-flight factory, mixed operations including GA VFR, and native visibility/audio/FPS/platform scope remain unfinished. Generated traffic stays disabled by default. Reference: [FAA airport holding-position markings](https://www.faa.gov/air_traffic/publications/atpubs/aim_html/chap2_section_3.html).

# Traffic crossing altitude and airspeed coupling — 15 October 2026

Focused core trajectory checks pass 2, generated airborne-section checks pass 4, terminal decoder checks pass 3 with one licensed-data test then explicitly run and passed against installed Nice BASI8A. The new regression reproduces an IAS mismatch when the former fixed preferred-level ground speed is reused after vertical scheduling lowers a crossing, then verifies converged correction for nominal65/110/170kt cases. Caller protection checks and incompatible input rejection remain active. Validation logs are retained locally.

Release engine build passes; source only, not installed. This is not a complete generated airborne producer/controller or performance model, and does not establish native aircraft visibility, speech, FPS or platform acceptance. Native simulator acceptance remains separate.

# Barometric altitude and QNH wording — 13 October 2026

Core97/engine65/plugin18/UI28 tests pass, with environment-specific tests ignored as listed in the log. The 18,311.7 geometric / 18,000 indicated and pressure-altitude regression produces no altitude warnings, including variation within the existing 300-foot band. A real 500-foot pressure-altitude error requires the existing 30-second delay and then warns; descent toward the assigned level suppresses repeat correction. Optional invalid altimeter readings are omitted at the plugin boundary. Older telemetry falls back to its existing geometric reading. QNH1022 omits units, QNH993 retains hectopascals.

Final release-engine radio and three-controller full-flight replays, release build and Linux dependency audit pass. Validation logs are retained locally. Native ToLiss altimeter readings and speech remain user acceptance checks; X-Plane was not launched. Region-level transition settings remain the source for pressure-altitude selection; airport-specific transition levels are not added in this repair.

# Crew selection and LFMN stand guidance — 13 October 2026

Plugin checks pass: 17 tests, one installed-scenery test separately run. Repeated ground/cabin calls retain their recipient and explicit radio selection restores ATC. The real-engine crew suite grants start-up/pushback approval, remains stationary in Pushback, requests external power and chock removal, acknowledges both simulator actions and verifies one Ground confirmation; the same request is refused at 2 knots.

The installed LFMN regression reproduces the reported 43.655764, 7.202480 stand, 154.5 metres from its approved taxiway route. It now samples 11 arrows toward the 04L crossing hold on the way to 04R. Every arrow remains on the published route; the unverified apron connector remains unmarked. Validation logs are retained locally. Native button interaction and rendered simulator arrow visibility still require acceptance in X-Plane; these checks do not launch it.

# Center frequency and check-in regression — 13 October 2026

Legacy `FREQ` and airport frequency records restore the truncated last digit; tests cover 118.025, 125.275, 125.725 and 136.975 MHz while preserving modern 118.020 and 118.030 channel records. An engine regression loads synthetic controller scenery, crosses an Approach boundary, receives a 118.025 Center handoff, completes the copilot readback, tunes, checks in and receives Center's climb instruction. A UI regression verifies check-in, radio check, frequency, altitude and direct requests remain available in Departure on Center, while surface requests stay hidden.

The installed-scenery regression passes at 33.914847, -118.384915, 3,600 feet MSL on 118.025 MHz and resolves LOS ANGELES Center. Validation logs are retained locally. This reads scenery and simulates telemetry; it does not launch X-Plane or prove native speech playback.

# Taxi movement, assistance replies and window transitions — 13 October 2026

Engine tests pass (62 passed, one environment-dependent test ignored), including stationary taxi starts beyond the warning delay, speed fluctuations without displacement, movement before readback, fresh clearance sequences, a real deviation followed by a stop, and busy-radio/controller gating. Typed `yes` passes through the real request handler with the AI endpoint deliberately unavailable for both departure and arrival taxi. Replacement routes retain the intervening runway hold, appear as pending readback and support copilot readback. Stale questions, another frequency and incomplete readbacks cannot be accepted with a short answer.

UI tests pass (27 passed, one installed-engine test ignored). Actual click events exercise compact/expanded controls with the pointer left over the button and moved out/back in. Wheel and resize events verify latest-message following and preservation of an earlier reading position. Desktop captures exercise collapse and expansion with a populated transcript. A stale desktop viewport after an immediate host resize was found during capture and fixed; this does not establish native X-Plane rendering or FPS.

The release-engine clearance-to-gate replay passes all three controller configurations. The HTTP radio replay rejects a deliberately crew-shaped classifier response, returns an ATC clarification and preserves both clearances. Linux release build and dependency/export audit pass.  X-Plane was not started; native simulator and Windows/macOS runtime acceptance remain outstanding.

# Header and UI audit — 13 October 2026

The UI suite passes 25 tests, with the installed-engine integration test intentionally ignored in this UI-only pass; all 15 plugin tests pass. New interaction checks click every navigation icon and pop-out at 520/960 base widths and 0.85/1.0/1.5 text scales. They verify that icons stay inside the window and that pop-out works outside ATC. A separate input test edits the actual callsign field while the engine is unavailable.

Desktop renders cover all ten pages and all eleven settings sections, including narrow windows and enlarged Light-theme controls. Populated KLAX radio/taxi and KLGB arrival views use the standalone engine with installed scenery and synthetic parked telemetry. UI captures use an isolated settings directory with local inactive AI/speech endpoints, preserving the user's settings. Map labels and long settings/form labels were corrected after inspecting renders. Header navigation is now vector icons; the earlier text page strip is superseded. Settings retain wrapping text tabs.

Desktop validation records include page and settings contact sheets, individual captures, test results and release-build results. The debug desktop capture can inject clicks using `OPENATC_PREVIEW_CLICKS` (`seconds:x:y`, separated by semicolons) and delay capture with `OPENATC_PREVIEW_DELAY`; these controls are excluded from release builds. These checks do not establish native simulator rendering, real flight operation or FPS. X-Plane was not started.

# Release repair checks — 13 October 2026, 10:35 UTC

The Linux core, engine, UI, plugin, settings and xtask suites pass. New regressions exercise invalid optional simulator instruments, preservation of required position validation, retained telemetry errors across empty polls, recovery on a valid sample, settings migration and clickable wrapped tabs. The original rejected native sample was not captured; field-specific diagnostics now identify any remaining rejection instead of reporting only "Invalid telemetry".

The release engine passes clearance-to-gate replays for separate Delivery/Ground/Tower, Ground/Tower and combined Tower service. Radio, compound crew and speech integrations pass. Installed KLAX scenery checks identify KLAX as the occupied airport, expose published Delivery services, update request availability immediately after COM1 changes and hide start-up requests once engines run. The installed parallel-runway holding/crossing regression also passes. Linux release builds and dependency/export audits pass; existing vendored SDK warnings remain.

Desktop captures were inspected for narrow STT/TTS settings, Light-mode Traffic settings/model installation and the KLAX radio list. Page navigation and settings sections are visible wrapping tabs. Background scene capture and post-processing were removed. These checks establish desktop layout, not native simulator rendering or performance.

Native flight/audio/ToLiss/FPS and Windows/macOS runtime acceptance was unavailable for this historical release-candidate check. Generated CSL traffic and the other unfinished items in [the roadmap](ROADMAP.md) remain separate work; this result does not establish completion of the roadmap. Bluebell installation was outside this check.

The full-flight report is `packages/flight-validation/report.json`.

# Resumed flight and traffic work — 13 October 2026

Linux checks pass for the changed core, engine, UI, plugin, settings and developer tooling. The combined run has 94 core unit tests, 57 engine tests, 22 UI tests, 14 plugin tests, seven settings tests and three xtask tests, plus core integration suites. A further focused regression checks rejection of non-finite motion/flight-clock input without replacing the last valid state. Environment-dependent tests remain ignored unless explicitly run. Clippy completes with existing/advisory warnings; the checkout is not warning-free.

The release-engine flight replay passes separate Delivery/Ground/Tower, Ground/Tower and combined Tower configurations from clearance to the destination gate. It covers both runway crossings, sector handoffs, mission/emergency exchanges and copilot readbacks. The replay now supplies A320 type/dimensions and verifies that Tower retains the flight while the aircraft centre has exited but the tail still overlaps the runway. The installed KLAX scenery regression also passes. Release radio, compound crew and speech integrations and the Linux dependency audit pass.

Focused checks exercise two simultaneous encounters, an urgent second aircraft during a pending readback, reused surveillance IDs, missing observations and paused telemetry. Information-only advisories preserve the operational readback. Prepared copilot playback is idempotent; stale completion sequences and frequency changes cannot accept another reply. Retuning also removes the old prepared reply so it cannot leave monitoring permanently busy.

Wake checks cover FAA CWT aircraft pairs, full-length versus intersection/opposite departures, qualifying parallel runways, simulator-time pause behaviour, Piper/A320 approach order, displaced thresholds and a spacing violation that survives the leader's touchdown/loss from the feed. They are not proof of complete wake coverage: intersecting flight paths, preceding low/missed approaches and national variations remain open. Whole-aircraft runway checks are conservative reference-point envelopes, not calibrated per-aircraft collision meshes.

The new movement core is tested for protected resource ownership, explicit crossing/continuation permission, impossible climb/discontinuous route rejection, reset generations, pause and clock changes. It is not yet connected to generated flights or a renderer. No native X-Plane flight, audible radio exchange, ToLiss control acceptance, native FPS measurement, or Windows/macOS runtime acceptance was performed in this pass. Bluebell remains uninstalled for the user's installer test. See [the unfinished implementation work](ROADMAP.md).

# Appearance, settings and editable traffic profiles — 13 October 2026

Linux unit/integration suites pass for core, engine, settings, UI and plugin. The core unit suite has 85 passing tests, engine 51, plugin 14, settings 7 and UI 22; environment-dependent tests remain ignored where their fixtures are unavailable. New checks cover profile references/overrides, seeded route/model eligibility, worker saturation and cancelled queued work, own-aircraft climb prediction when target vertical speed is unknown, and saved appearance defaults/ranges. Clippy completes with advisory warnings; this is not a warning-free checkout.

Light/Dark desktop appearance captures were inspected at wide and minimum-width layouts. Those captures establish layout only, not native simulator appearance, pop-out/VR transitions or FPS. The background post-processing implementation from that pass has since been removed after native flickering reports.

The three editable traffic profile files, validation service and model-compatible selector are implemented. They do not establish generated-aircraft movement or complete traffic separation. Bluebell remains uninstalled for the user's installer test.

The Linux release plugin/engine build passes. `test-flight` and `test-speech` pass against that release engine, including the three controller configurations from clearance through departure crossings, sector handoffs, approach/landing, arrival crossings and parking. These are telemetry replays, not a native simulator flight.

# Traffic foundation and multicore task reconciliation — 13 October 2026

The public [roadmap](ROADMAP.md) records the remaining scope, including explicit Linux/Windows/macOS multicore work and acceptance checks. The current CPU pool is used by scenery/navigation loading and CSL indexing. Tests verify conservative automatic limits, explicit caps, concurrent execution of independent jobs and survival after a job panics. This does not establish generated-traffic simulation or a native FPS benefit.

The latest Linux tests pass: core 81, engine 49, settings 7 and UI 21; two environment-specific tests remain ignored. New tests cover predictive encounters, heading-based clock positions, unknown/invalid data, sighting negation, lost-feed versus no-factor replies, preserving a pending readback and retaining IFR clearance through repeated corrections. Synthetic CSL packages verify model matching and path boundaries; Bluebell has not been reinstalled for these checks.

The debug engine builds, the plugin passes `cargo check`, and the real-engine `test-flight` and `test-speech` replays pass. Clippy completes for core, engine, UI and settings with advisory warnings; this is not a warning-free checkout. The flight replay covers three controller configurations from clearance through arrival runway crossings and parking. These checks use simulated telemetry, not a live simulator flight. No new binaries have been installed in X-Plane during this pass.

Custom CSL rendering and generated movement, simultaneous encounter management, urgent advisory preemption, complete wake/separation rules and airborne holding are still unfinished. Windows/macOS execution, UI appearance, audible exchanges and simulator frame time require native acceptance.

# Crew, missions, traffic and final hold — 13 October 2026

The Linux release plugin and engine build and dependency audit pass. Core, engine, settings, UI, plugin and xtask tests pass; the final focused checks also cover parked/finished plan import and headless Traffic/Missions pages. Clippy completes with advisory warnings, including existing vendored SDK warnings; this is not a warning-free build.

The release-engine `test-flight` replay passes with separate Delivery/Ground/Tower, Ground/Tower and combined Tower duties. It covers departure and arrival runway crossings, holding-point handoff, takeoff, sector handoffs, approach and landing permission, observed runway exit and taxi to the assigned gate. It also saves, loads and runs a medical mission, selects an event response, declares an emergency explicitly and finishes the story at parking. The current report and transcripts are in `packages/flight-validation/report.json`.

The installed-scenery KLAX regression passes at the reported 25L holding position (33.938318, -118.383084), at 0.19 knots. It verifies a handoff to that airport's published Tower. That coordinate already matched the existing holding geometry; the fix also addresses the monitoring/handoff path rather than assuming marker size was the sole cause.

Release radio/speech integration and real-engine crew integration pass. Radio checks now use the dynamically assigned squawk, verify that it survives taxi and that successive flights do not reuse recent codes. Phrase-variety checks remove the squawk before comparing wording, so different numbers alone cannot make the test pass. Crew checks exercise compound actions/services and acknowledgement-driven results, including partial failures. The plan-import regression preserves a preflight crew exchange, resets completed-flight state and keeps message IDs monotonic.

New deterministic tests cover taxi conflict hold/resume, runway-hold priority, approach speed limits and an unable response, short-final go-around while the radio is busy, truthful observed-traffic replies, occupied-gate reassignment, aircraft size, mission probability/phase/pause/choices, cabin incidents and optional terrain-checked shortcut offers. Full-flight replay exposed an ordinary taxi-conflict hold taking priority over runway-crossing monitoring; the corrected path passes the replay.

These checks drive synthetic telemetry and acknowledgements through the real engine. They do **not** establish live ToLiss controls, audible turn-taking, native AI ownership/count behaviour, renderer appearance, FPS or Windows/macOS runtime compatibility. Native AI uses X-Plane flight models. Custom Bluebell CSL rendering/installation, complete traffic separation, full diversion routing and mission media playback remain unfinished. See [remaining work](ROADMAP.md).

# Earlier full-flight replay and observed compliance — 13 October 2026

`cargo xtask test-flight target/debug/open-atc-engine speech` passed three real-engine HTTP flights with separate Delivery/Ground/Tower, Ground/Tower and Tower-only surface duties. Synthetic scenery provides two intervening runways, three regional sector boundaries and an arrival gate. The replay checks clearance/readback, start/pushback, departure crossing, vacated report, onward taxi, Tower transfer, takeoff permission, airborne handoffs, approach/Tower/landing permissions, touchdown, rollout, observed runway exit, arrival crossing and taxi to Gate C1. Traffic blocks both a departure and arrival crossing; movement permission remains pending until readback. Busy-radio telemetry cannot interrupt the prepared copilot exchange. Reports include each checkpoint and final transcript under `packages/flight-validation/`.

The replay found and fixed station indexing that depended on opening Channels, regional check-in trying to load a Center identifier as an airport, and service redirection that announced a Tower frequency without updating the copilot's tuning target. Taxi-to-parking instructions no longer tell the aircraft to await further routing at its assigned gate.

New engine tests cover traffic appearing after landing clearance, a pending landing readback, short-final conflict checks inside the retry interval, and preventing immediate approach re-clearance after a go-around. Routine runway-exit calls wait for rollout below 40 knots. These checks use the [FAA landing and exiting procedures](https://www.faa.gov/air_traffic/publications/atpubs/ATC_html/chap3_section_10.html) as a reference; complete separation, local exceptions and missed-approach leg guidance are not implemented.

Observed compliance tests cover squawk adjustment time, busy radio, pause, correction, unknown/invalid/emergency codes, altitude-reporting mode, permission for the actual runway, unauthorized takeoff and landing, and delayed taxi assistance from the issuing controller. The plugin reads the documented X-Plane code/mode datarefs; missing observations remain absent. Phrase references include [FAA transponder procedures](https://www.faa.gov/air_traffic/publications/atpubs/ATC_html/chap5_section_2.html). These reminders are optional Realism settings and do not replace operational clearance checks.

The core, engine, settings, UI, plugin and xtask test run passed. Environment-dependent tests remain ignored. Ground arrow instances are now reused on updates; live simulator rendering and FPS, aircraft-specific telemetry and audio timing still need acceptance. No Windows/macOS CI run or live simulator flight is claimed for this revision.

Follow-me route tests cover corners, speed bounds, distance, pause, readback, arrival crossing limits and removal at parking. The renderer resolves X-Plane's installed vehicle through the scenery library, uses one instance and does not acquire AI aircraft. The client regression holds a command response open while flooding telemetry updates: the newest position must still reach the server, without queueing every old sample. Snapshot readers share immutable data and retain a consistent old snapshot across reset. Live follow-me rendering and actual frame-time measurements remain unverified.

The Linux release plugin and engine build, dependency audit and release-engine full-flight replay passed after this work. Static map cache replacement is tested, including reloading the same airport and clearing the airport. Clippy completes with advisory warnings. New vehicle SDK imports are present in the bundled Windows XPLM import library. A full Windows cross-check from Fedora stopped in a native dependency because MSVC's `lib.exe` is unavailable; this does not establish Windows compilation or linking.

SimBrief low-level import: regression tests and Linux release build pass. A live download for the supplied Pilot ID through the release engine returned KLAX/25L → KLGB/30, LAXX1 SLI KAYOH8, cruise and provisional initial altitude 4,000 feet. The complete planning draft is returned; visual simulator acceptance remains a user check.

# Airborne instructions and phrase alternatives — 12 October 2026

Core, engine, UI and settings tests pass, including descent-distance limits, published crossing windows, phonetic short replies, current/predicted sector ownership, pending-readback reminders, and separate approach/Tower check-in/landing permissions. The Linux release engine/plugin build and real-engine radio, crew and speech checks pass. Repeated start-up now rejects after approval; its integration expectation was updated accordingly. Existing vendored SDK documentation warnings remain.

Rendered the developer Map with installed coverage and the slow terrain overlay removed. The separate Arrival terrain profile remains. The new airborne catalogue contains 87 phrase alternatives across 21 events, with corresponding runtime templates and additional amendment/refusal/readback responses. TOML contracts and placeholder validation pass.

No complete simulator flight has been flown for this revision. Sector coverage quality, actual controller audio timing, descent performance, arrival surface monitoring and third-party controls still need live acceptance. Curved/unnamed/vector procedure legs, local MVA and complete missed-approach routing remain implementation limits. The nominal descent check must not be described as an aircraft-performance guarantee.

Debug surface jumps and addressed controller calls: target tests verify taxi permission and ground-state gating, heading and safe hold position. Radio integration checks no handoff while stopped with takeoff clearance, addressed controller messages and no backwards Delivery redirect for repeated clearance requests. SDK repositioning in ToLiss has not been exercised live.

Airport-filter and circle-rendering revision: default workspace tests, the UI pavement-containment regression, release build and radio integration pass. The regression checks KLAX surface geometry overrides 57CA reference-point selection and does not override it away from that pavement. Circle geometry is now two triangles with one shared alpha texture. Simulator appearance still requires a live check.

Holding-marker approach-side revision: tests cover both approach directions and an oblique approach, requiring the disc edge to remain at least four metres before the line. Default tests, release build and radio integration pass. The marker is now 30 metres across and uses the shared computed position on map and ground. Live scenery alignment remains user acceptance.

Successful crossing-readback regression: accepted readback produces no ATC transmission, enables the crossing, and far-side completion issues onward taxi. Default tests, release build, speech validation and radio integration pass.

# Holding-point marker — 9 October 2026

Default workspace tests, settings/plugin tests and release radio integration pass. The marker-placement test checks that it uses the route limit and approach direction; the setting defaults off. Clippy checks pass with existing advisory warnings. Ground terrain placement retains slope/footprint checks. Your local setting is enabled. The marker has not yet been visually exercised in X-Plane; live appearance and aircraft alignment remain simulator acceptance checks.

# Vacated reports and final holding-point handoff — 9 October 2026

Core/engine/UI tests and release radio integration pass. The radio sequence verifies “VACTED” produces the onward taxi instruction without “readback correct”, then its readback activates monitoring and stopping at the departure hold produces the Tower frequency. Core tests cover holding lines offset from graph endpoints and the surface lifecycle with readback preceding monitoring. Installed KLAX scenery regression also passes. Live simulator map colours, holding-line detection and physical taxi remain user acceptance checks.

# Controller voices and crossing completion — 9 October 2026

Core, engine and UI tests pass. Radio integration verifies distinct English Delivery/Ground voices, reuse of the saved Ground voice after a full engine restart, crossing completion while moving and onward taxi requiring readback. The installed KLAX regression checks the reported position clear of 25R and routes onward to the 25L hold. Physical simulator playback and taxi behaviour remain user acceptance checks.

# New flight and runway crossings — 8 October 2026

The release engine and plugin build passed. Rust core, engine and UI tests passed, including a delayed HTTP snapshot check that prevents an old flight from returning after reset. Clippy completed with the existing vendor and pedantic warnings; it was not warning-free.

The isolated radio integration verifies proactive Ground crossing clearance, holds when traffic data is missing or a runway is occupied, crossing readback, no crossing completion on the runway, and onward taxi to the original departure runway. The installed KLAX regression verifies that a route toward 25L first holds at 25R, crosses only 25R and continues toward the 25L hold. The airport identifiers belong to regression tests, not production routing branches.

Reset checks verify an empty plan and conversation, cleared permissions and crew actions, preserved live telemetry and monotonic transmission IDs, rejection of stale crew acknowledgments, and rejection of a delayed LLM response after reset. Crew integration also passed. The installed plugin and engine were updated after X-Plane closed.

The New flight icon and re-enable lifecycle change still require live simulator acceptance. The crash log identified OpenATC during re-enable; inspection found that the previous enable path created a second active ImGui context while retaining the first. Enable now reuses that renderer and avoids duplicate command registration. Yellow scenery guard lights remain unchanged; individual native light control has not been established.

# Crew control validation — 8 October 2026

The real engine crew integration passes role-specific bounded controls, heading 360 normalization, flight-level units, cabin slides/cross-check, no-action queries/negations, ground gear-up rejection, both checklist modes, failed-item stopping, idempotent acknowledgements, button-dispatch wording and aircraft replacement. This uses simulated plugin observations and acknowledgements. It does not prove live ToLiss writability or actual cockpit operation. The user must check ATTN/MECH selection, controls, cabin lighting and slides in the simulator. ToLiss pushback distance/angle are not implemented.

# Taxi fallback and Auto Reply — 8 October 2026

Core/engine/UI tests cover complete pilot readbacks, background traffic exclusion, centerline versus edge marking types, pavement gaps and holding-line barriers, assigned runway backtracking and rejection of other-runway crossings. The release engine integration verifies Ground-to-Tower handoff for runway taxi, mandatory backtrack readback, endpoint gating before departure and rejection of reused start/pushback acknowledgments. Installed VLVT scenery is separately exercised for runway 31 taxi and a holding point plus runway 13 backtrack. ATC was rendered at 960 and 520 pixels with Auto Reply beside Replay ATC. Live simulator message selection/system clipboard, physical taxi tracking and ground arrows remain user acceptance checks.

Run the installed-scene regression with `OPENATC_SIM_ROOT="/path/to/X-Plane 12" cargo test -p openatc-core --test airport_scenery installed_centerlines -- --ignored --nocapture`. Optionally set `OPENATC_ROUTE_REVIEW_PATH` to export the airport and calculated paths for geometry review. Production routing has no VLVT-specific branch; only this regression names the reported airport.

# Debug export and Planning activation — 8 October 2026

Removed sample departure/destination/runway defaults from live plans. YMLT/YMML remain in explicit historical test fixtures only. A clearance request submits the Planning draft, waits for successful engine acceptance, then transmits; plan validation failures stop the request. Conversation history is preserved through plan submission. Developer mode offers Copy ATC conversation, including active and draft flights and radio/ground telemetry; export formatting has a regression test. Clipboard helper failures produce a text-file fallback. Core/engine/UI tests and release radio workflow checks passed; live clipboard copying and simulator import-to-clearance are user acceptance checks.

# Clearance feedback and list controls — 8 October 2026

Core/engine/UI tests and strict Clippy passed. Real-engine checks confirmed a mismatched flight departure returns the same specific message in the transcript and result, and VLVT Tower still accepts a valid IFR clearance. The full radio workflow regression and dependency audit passed. Channels was rendered at 960 and 520 px with the nearest-airport filter applied; the narrow layout keeps Show all visible. Native simulator header clicks and COM1 tuning remain user acceptance checks. Sort support was added to every data-list table (stations, frequencies, navaids, stands and both procedure lists); form layout tables are unchanged.

# Consolidated controller duties — 8 October 2026

The real release engine, using installed VLVT scenery/navdata and isolated settings, accepted an IFR clearance on Tower 118.100 for a VLVT–VTUD plan. Regression tests cover Tower-only clearance/ground duties, Ground clearance where Delivery is absent, separate Delivery priority, and ATIS exclusion. Core/engine tests and strict Clippy passed, as did the existing full radio workflow integration test. Simulator UI acceptance still requires reloading the plugin.

# Channels controller-data correction — 7 October 2026

Loaded the installed scenery and active custom ATC data at VTUD and VLVT. VTUD now includes Ground 121.900, Tower 119.450 and 122.500, and Approach 126.200 (plus its shared 119.450 channel). At VLVT the loaded data provides Tower 118.100 and Vientiane Center 124.100/128.300. The initial list incorrectly filtered Approach by its assigned altitude and omitted adjacent regional Center coverage. A follow-up real-engine check at VLVT now verifies the screenshot’s Tower 118.100, Approach 119.700 and Hanoi Center 123.300 using installed data. Other published regional channels can also appear; the simulator airport pop-out is not a complete nearby receiver list. Tests cover Tower override, Ground preservation, FREQ/CHAN units, regional polygon reception, altitude limits, and longitude wrapping without opposite-hemisphere false matches. Core/engine/UI tests, strict Clippy, release radio integration and binary dependency audit passed. Live simulator Channels and COM1 selection still require a restart and user acceptance.

# Engine startup correction — 7 October 2026

The installed engine log reported missing `libssl.so.3` under Steam. Rebuilt the Rust engine using reqwest Rust TLS with bundled roots, and strengthened the dependency audit to reject OpenSSL in either runtime binary. Four engine unit tests and the release radio integration workflow passed. The release engine’s offline speech validation and the installed engine’s complete isolated radio HTTP workflow also passed through the installed Steam runtime launcher. Its remaining dynamic dependencies are libc, libm and libgcc_s. A live simulator restart remains the final plugin-launch acceptance check.

# Validation record

## Radio/departure sprint validation — 7 October 2026

The final Rust core, engine, audio, HTTP and UI run passed **74 tests**, with **two environment-dependent checks ignored** (live devices and installed scenery). Strict Clippy passed those five packages with the existing `too_many_lines` allowance. Release engine/plugin builds passed. Plugin Clippy completes with existing pedantic warnings in the older arrival/render/window code; the vendored SDK retains its documentation/import warnings.

The real release engine HTTP integration test uses isolated settings/scenery and a local fake model: station discovery and named services; silence on unknown channels, radio power loss and range loss; Delivery/Ground role restrictions; validated plan/runway, clearance and readback; separate start/pushback permissions, combined request and a generated taxi route; airport-specific simulator weather, ATIS information-letter changes, destination weather isolation, unavailable weather and rejected aircraft-altitude samples; and fallback when an LLM changes the clearance's operational values. No remote AI service is used by this check. Separate stalled-provider/client tests prove cancellation without waiting for a 30-second request timeout and worker joins before audio context destruction or client unload.

The real `/suggest` test passed three regional/rules selections and rejected three mismatches. Offline validation loaded 476 situations and 1,420 phrases. The installed Heathrow-area scenery produced 24 receivable stations; release indexing took approximately 2 seconds. Desktop Channels was rendered and visually inspected at 960 and 520 px, with complete narrow station cards. These previews do not prove simulator COM1 tuning or SDK weather values.

Simulator acceptance still needs the user to check: actual ToLiss power/COM1; microphone/STT and voice timbre; changing simulator weather while listening to local and destination ATIS; repeat broadcasts and stop-on-retune/power/range changes; SimBrief import or a manual plan followed by Delivery clearance/readback, Ground start/pushback/taxi; approved map and terrain-following night arrows; and pause/quit engine lifecycle. No claim is made that a live simulator flight or remote server was exercised in this sprint.

Reproduce the focused checks:

```sh
cargo test -p openatc-http -p openatc-core -p openatc-engine -p openatc-audio -p openatc-ui
cargo clippy --manifest-path Cargo.toml -p openatc-http -p openatc-core -p openatc-engine -p openatc-audio -p openatc-ui --all-targets -- -D warnings -A clippy::too_many_lines
cargo xtask test-radio target/release/open-atc-engine speech
cargo xtask test-speech target/release/open-atc-engine speech
target/release/open-atc-engine --check-speech speech
```

The older validation records below are historical checkpoints.


## Current Rust work — 2026-10-07

The records below the current section describe earlier C++ authoring and are historical, not the current deployment status. The Rust release engine/plugin have been built and installed on the local X-Plane target. Native selected-device capture/output, engine lifecycle ownership, ToLiss dataref type/tuning checks, configuration loading, terrain/descent geometry and desktop rendering have focused checks. Desktop map/Arrival images are previews; the Arrival terrain fixture is labelled PREVIEW and is not evidence of real simulator terrain.

The core/engine run passed 52 tests, with one installed-scenery test explicitly ignored. Focused speech/configuration tests and Clippy were rerun after the final regional filtering changes. Release engine/plugin builds passed; the vendored SDK retains existing warning messages. A local fake model exercised the real `/suggest` HTTP path: three regional/rules selections passed and three invalid/mismatched selections were rejected before model dispatch. This proves retrieval and endpoint integration, not a real model’s procedure accuracy.

The editable library has dedicated tests for every phrase rendering, historical IDs, regional longest-prefix boundaries, FAA/ICAO isolation, IFR/VFR separation, global crew selection, taxi-in phase normalization, missing context, malformed user edits and duplicate-ID diagnostics. `shipped_config` loads the real distributed files with application loaders. The offline engine `--check-speech` command also loads and renders the full library without calling AI or starting a service.

Source review uses FAA, EUROCONTROL, UK CAA, NAV CANADA and Airservices Australia references. The speech guide records the scope and retrieval limitations. Regional folder coverage does not mean every national procedure has been reviewed. Baseline-only packs say so.

Still awaiting simulator acceptance: powered ToLiss radio behaviour, selected-microphone transcription, terrain-following/night arrows, pause/shutdown behaviour during a real sim session, and flight descent/clearance handling. Windows/macOS builds, full traffic separation, comprehensive published-procedure execution and worldwide national procedure validation are not claimed.

Authoring date: 2026-10-05. Source revision: 0.2.0.

## Executed

GCC 13.3 compiled the dependency-free controller and airport-data sources in C++17 mode with warnings treated as errors. All **94 checks passed**.

```bash
mkdir -p build/core-only
g++ -std=c++17 -O2 -Wall -Wextra -Wpedantic -Werror -Iinclude \
  src/core.cpp src/flight.cpp src/airport_data.cpp tests/core_tests.cpp \
  -o build/core-only/openatc-tests
./build/core-only/openatc-tests
```

Coverage includes phase-gated buttons and backend requests; clearance/readback sequencing; altitude and direct-to validation; ground/airborne transitions, pause and bounce hysteresis; go-around and landing; taxi graph paths, one-way restrictions, aircraft size and active-runway zones; parking completion; airport, parking, frequency, navaid and procedure parsing; localizer/glideslope encoding; descent geometry; dateline distance; flight-plan validation; METAR fields and transcript retention.

Python scripts passed syntax compilation. Shell scripts passed `bash -n`. Source delimiters and archive integrity were checked. These checks do not substitute for compiling the engine and UI.

## Not executed

- Full CMake engine, desktop or X-Plane plugin builds. CMake and development dependencies were unavailable in the authoring environment, and dependency/SDK downloads returned HTTP 403.
- The JSON/SimBrief parser executable and engine HTTP integration tests supplied in the former CMake-era test harnesses (now replaced by Rust tests).
- Actual `ldd`/`nm` auditing of a newly built plugin, installation into X-Plane or simulator startup.
- Rendered UI inspection, audio-device capture/playback, STT/TTS round trips, real SimBrief OFP downloads or weather API calls.
- Windows/macOS compilation or CI execution.

No working binary or simulator screenshot is claimed. The Fedora build script and CI configuration are the next integration gates; integration fixes may still be required after dependencies become available.

## Steam Runtime / TLS correction

Muse reported that the previous plugin linked `libssl.so.3` and `libcrypto.so.3`, which X-Plane could not resolve inside Steam Runtime. The supplied `src.zip` contains the same seven source files as the supplied full-project archive; it does not contain Muse's reported CMake or dependency-audit changes. This revision implements those build changes in the complete project.

`openatc_transport_plain` includes httplib headers without linking its TLS-enabled CMake target. `openatc_ui_plugin` and the plugin use that plain transport. A compile-time guard rejects accidental `CPPHTTPLIB_OPENSSL_SUPPORT` in the UI. The engine retains HTTPS and proxies SimBrief, weather, AI, STT and TTS. This also permits HTTPS speech providers without linking TLS into the simulator plugin. The plugin uses an `$ORIGIN` RPATH.

The Linux audit rejects missing dependencies, X11/XCB/GLX and legacy GL dependencies, and additionally rejects SSL/crypto/curl dependencies in `.xpl` files. It checks all five required XPlugin exports. The script was syntax-checked here, but only a build on the target host can establish its actual linked dependency list.

## Runtime checks after building

1. Run the Fedora plugin build and `cargo xtask test-radio target/release/open-atc-engine speech` as shown in the README.
 2. Install the staged plugin with X-Plane closed, then launch X-Plane (the plugin starts its bundled engine itself; `~/.config/openatc/engine.log` shows its output). Check `Log.txt` for OpenATC startup and loader errors.
3. At a loaded airport, confirm that parked controls exclude altitude/direct-to, aircraft position matches the surface map, and taxi is unavailable before clearance readback.
4. Import the latest SimBrief OFP and compare its route/procedures, fuel and payload with the SimBrief output before accepting it.
5. Check font clarity, centered menu labels, resizing, request dialogs and layer filters on the target display.
6. Select microphone/output devices; use the local input monitor before enabling transcription, controller speech or copilot speech. Check separate volume controls and COM1 tuning on an explicit airport frequency assignment.
7. Fly a departure, descent, go-around and landing. Confirm stage transitions and that taxi-in finishes only when stopped at the assigned stand.

Weather awareness is controller-spoken METAR hazard advisories, not precipitation radar. Procedure lists do not implement full procedure-leg navigation. Taxi approval covers the loaded graph; dashed ramp connectors are unverified and runway-crossing clearances are not implemented. These limits are also stated in the UI and README.

## 0.2.1 branding follow-up

The vector logo asset was rendered and inspected, branding references were checked, and sidebar wordmark dimensions were checked over the supported UI scale range. The complete plugin build and X-Plane rendering were not executed in this workspace. The 94 core checks above describe revision 0.2.0; this branding follow-up does not change controller behavior or font rendering.

IFR clearance messages explicitly request readback. Spoken readback matching checks NATO callsign/destination, route, altitude, squawk and runway; unclear values receive say-again rather than a phase error. Taxi instructions remain pending until readback is accepted. Ground guidance ends when stopped within 25 metres of the route endpoint. Protected runway edges are excluded; a reachable identified crossing hold point may end a partial route, without crossing permission. The UI registers a native system clipboard backend for external text-field copy/paste, with persistent ownership. Live simulator voice and clipboard interaction remain user acceptance checks.

Live controller responses, readback prompts/corrections, taxi instructions, ATIS/weather wording, crew fallback replies, copilot callouts and background exchanges now come from `speech/runtime/responses.toml` (191 templates, 381 phrases). The regional/crew example library retains 476 situations and 1,420 phrases. Runtime alternatives preserve named fact placeholders; startup rejects missing responses or changed placeholder contracts. Phrase edits do not change Rust authorization logic. Taxi readback uses structured taxiway/hold-short facts, so editable wording is not parsed as route data. An isolated engine regression edits the TOML acknowledgment and verifies that exact edited text is returned; invalid placeholder alternatives are rejected.

Pending readback evidence now takes priority over generic altitude/taxi intent classification. Regression coverage uses the reported VLVT→VTBS typed readback and garbled transcript; an ordinary taxi request is not treated as a readback. The plugin provides a visible bottom-right resize grip that respects minimum size and pop-out pixel scaling.

The exact reported typed readback passes the parser-priority regression and a numeric typed readback passes the isolated release-engine workflow. Synthetic UI mouse-down/drag events verify that the visible resize grip emits resize actions. The preview rendering was inspected; X11 desktop automation did not deliver its mouse press, so native simulator dragging remains a user acceptance check.

2026-10-08 taxi follow-up: core/engine/UI test suites and all 12 plugin unit tests passed. Isolated engine integration exercised the server-side Auto Reply endpoint for ordinary taxi and runway backtrack, verified taxi approval and no altitude acknowledgment, and rejected repeated automatic replies. Ground-arrow sampling and terrain-placement unit tests passed. Ground guidance was enabled in the workstation settings; visible rendering in X-Plane remains a simulator acceptance check.

Holding-point follow-up: core tests cover an empty valid scan, missing traffic data, runway occupancy, inbound final and outbound non-conflict. Isolated engine tests cover stopped endpoint detection, Ground handoff with Tower frequency, pause protection, Tower holding for final traffic without repeated messages, and automatic clearance/removal of taxi approval when traffic clears. Plugin tests cover ground-arrow sampling and terrain placement; duplicate holding-light markers have since been removed. Native holding-light control has not been established.

Legacy-source cleanup: the Rust plugin/engine release build and core/engine/UI/plugin test suites pass after removal of top-level src/include and the old CMake targets. The isolated radio engine integration also passes. The Fedora helper passes shell syntax validation. The Cargo-based Linux package was built locally and checked for binaries, speech, source/license notices and SHA-256 integrity; the updated GitHub workflow has not yet run on GitHub.

Root-workspace migration: the Linux plugin/engine release build, core/engine/UI/plugin tests, settings/platform/AI-core/audio tests, desktop compilation check, formatting, scoped Clippy, isolated radio integration and dependency audit pass from the repository root. The Linux archive was rebuilt with the new paths and verified for binary/docs/branding/lockfile presence and checksum. Python and Fedora script syntax checks pass. packages/ and target/ are listed in .gitignore, and vendor/xplane is explicitly allowed. No Git commands were used for the migration or readiness checks; index/staging and commit operations remain with the maintainer. GitHub CI and new live simulator acceptance were not run during this filesystem-only migration.

## Named positions and TTS recovery — 8 October 2026

Offline GeoNames tests cover the reported VLVT location, airborne/ground wording near Hobart, invalid coordinates and an ocean position with no nearby settlement. Core/engine suites passed after replacing the coordinate-response fixture. Restored eSpeak data alongside the existing fedora-server binary and restarted its AI service; live provider requests synthesized both the exact reported coordinate phrase and a named-place phrase into valid 24 kHz WAV audio. This does not establish every prior failure cause, but confirms coordinates themselves can synthesize. Failed-text diagnostics were added to engine and server source; simulator acceptance remains pending.

Phrase variation checks cover independent runtime-template rotation, saved-history reload, different IFR clearance wording across flight resets, strict mode avoiding wording-model calls, and role/task/previous-wording context in variety prompts. Unsafe model clearances still fall back to checked TOML text.

Simulator follow-up checks produce ten distinct IFR clearance transmissions for identical plans across resets, map the chalks alias to the chocks action, reject compound partial actions, and exercise Ground-to-Tower/traffic-hold/clear-runway flow with ownship 35 m from the taxi endpoint. Native ToLiss control behavior and the revised panel edge still require simulator acceptance. The earlier missed KLAX trigger could not be replayed exactly because no live taxi-state snapshot remained after simulator exit.

Holding-marker size revision: plugin tests and release build pass. The translucent disc is 24 metres across, with terrain checks expanded accordingly. Live cockpit visibility remains to be checked; installed after X-Plane closed; cockpit appearance remains unverified.

Copilot radio replies now originate in the simulator after its speech queue drains, rather than being applied inside the engine request handler. The same endpoint handles onward taxi instructions and takeoff acknowledgements. Automatic tuning and cockpit actions wait for playback and pending copilot replies. Final holding-point telemetry does not invent a pilot ready report; holding recognition no longer requires zero taxi speed. Channels also loads local airport geometry (page 6) to identify the occupied airport. Regression checks cover deferred readback, takeoff acknowledgement, moving holding detection and installed KLAX positions; live ToLiss audio timing still needs simulator testing.

Radio turn-taking uses a two-step copilot exchange: record and play the readback, then apply it after playback finishes. Telemetry marks the radio busy during playback, recording or pending crew replies, delaying automatic surface and airborne calls. A previously reported ready aircraft can receive takeoff clearance after blocking traffic clears. Crossing eligibility follows holding-point detection without a conflicting stopped-aircraft gate. Unknown runway status produces an explained standby instruction and retries when confirmation becomes available. The debug header uses a vector fast-forward icon with the same dimensions as other controls. Unit and engine regressions cover prepared readbacks, occupied-radio suppression, moving hold recognition and traffic-clearance retry; embedded simulator layout and live audio need user verification.

Typed ATC requests add missing controller and callsign addressing. Spoken transmissions retain the words recognized by speech-to-text. AI intent recovery tolerates clear request-word typos and recognition errors; operational values and readback checks remain authoritative. Crew requests can contain up to eight explicit actions, such as “remove chocks and external power.” Clauses inherit the action verb where appropriate. All targets are validated against the aircraft profile before the batch becomes visible to the simulator; each action needs confirmation, and failure cancels the remaining actions. Duplicate controls and unsupported clauses are rejected. The LLM can return a bounded actions array using mapped control IDs, never raw simulator refs. Settings → Voices → Pilot (own transmissions) → Speak my transmitted requests enables synthesized playback of the pilot requests. It is off by default. Engine regression tests use mocked model responses and simulator acknowledgements; live control effects remain a simulator check.

## Platform build validation

On 2026-10-09, Linux cross-target checks passed for `openatc-core`, `openatc-settings`, `openatc-platform` and `openatc-http` on `x86_64-pc-windows-msvc`, `x86_64-apple-darwin` and `aarch64-apple-darwin`. An unused Windows import was removed. These checks compile the Rust code; they do not run Windows or macOS tests.

Full plugin/engine and AI cross-target attempts encounter missing native tools on this Linux machine. Windows requires MSVC tools such as `lib.exe`; Apple targets require a compatible compiler and macOS SDK. The native GitHub matrix is the next validation gate and has not been executed from this checkout. Run it manually after uploading the workflow, then download the artifacts and test inside X-Plane. Native build success does not validate microphone devices, rendering, aircraft controls, signing or model execution.

Windows native CI runs Cargo commands in PowerShell and selects the Visual Studio linker explicitly. Git Bash also supplies a `link.exe` utility, which must not be used to link MSVC binaries. The initial Windows CI failure occurred before application compilation; the corrected workflow still needs a native rerun.

The Intel macOS AI-server job builds ONNX Runtime 1.28.0 from source because `ort-sys` 2.0.0-rc.13 does not supply Intel Mac prebuilt libraries. Cargo links that shared library, includes it in the build artifact, and uses `@loader_path` so the server can find it beside the executable. Other platform jobs retain their existing dependency setup. The source build disables ONNX Runtime unit-test compilation with `onnxruntime_BUILD_UNIT_TESTS=OFF`; `--skip_tests` alone only skips running those tests. Runtime model support is unchanged. This configuration still needs a native Intel Mac CI run.

Workflow changes must pass `actionlint -shellcheck= -pyflakes= .github/workflows/build.yml .github/workflows/platform-builds.yml` before upload, plus `bash -n` for Bash steps. YAML parsing alone does not check GitHub expression contexts. On 2026-10-09, both workflows passed actionlint 1.7.12; the validator also rejected a temporary copy containing the original invalid `runner.os` shell expression. Windows and Unix Cargo steps now use explicit shells selected by their step conditions. Native compilation and runtime checks still require the hosted runners.

## Rust tooling migration

`cargo xtask test-radio`, `test-crew` and `test-speech` replace the maintained Python integration suites and exercise the actual engine with isolated fixtures and a fake model. `cargo xtask audit-linux` checks Linux dependencies and plugin exports; `cargo xtask package linux-x64` creates the installer-backed archive. `cargo test --locked -p xtask` checks atomic file replacement, permissions and nested resource copies. The older CMake/Python validation sections above are historical records, not current build instructions.

Migration checks on 2026-10-09 passed: all three Rust HTTP integration suites against the release engine; core/engine/UI/plugin regression tests; xtask unit tests and Clippy; Linux dependency/export audits; plugin ZIP creation, integrity, executable permissions, license notices and SHA-256; staged plugin installation with backup, station-file preservation and legacy speech migration; staged AI installation and artifact collection with eSpeak data. Both workflow files passed actionlint 1.7.12. The real simulator installation and running AI service were not changed. Native Windows/macOS execution of the new tools still needs CI.

## Windows SDK regression

The five reported CRT-symbol errors (`memcmp`, `memcpy`, `memmove`, `memset`, `strlen`) were reproduced on Linux with Windows-target bindgen and minimal test headers declaring those CRT functions. After restricting generated functions to XPLM/XP, the Windows-target `xplane` wrapper check passed. A separate Windows-target metadata check compiled the production supervisor and OpenGL resolver modules against the SDK and HTTP crates. These checks use synthetic Windows headers and do not replace a native Windows build or simulator test. Linux plugin/engine release builds and plugin/UI/engine regression tests were also run.

Windows build review: the plugin launcher, OpenGL loader and SDK bindings pass the Windows-target metadata check with temporary minimal headers. Plugin tests (14) and artifact-tool tests (2) pass on Linux. Full Windows compilation remains unverified locally because MSVC native build tools are unavailable; the Actions job performs the native build. Its PowerShell engine checks now stop on each failed command rather than allowing a later success to mask it.

## Controller check-ins and copilot handoffs — 10 October 2026

Core, engine, UI and plugin tests passed after the check-in and handoff changes. The real-engine radio exercise checks that a prepared copilot check-in appears before the controller responds, then receives an ATC acknowledgement when committed. The crew exercise passed. Linux plugin and engine release builds passed. Natural check-in variants and cabin arming/cross-check forms have regression coverage.

Actual third-party radio tuning and ToLiss slide confirmation still require simulator testing. The observed slide failure was a readback failure after execution; a longer settling window and raw failure readings improve diagnosis but do not prove the aircraft's slides will arm. Departure-sector selection also needs comparison with the simulator's installed station and sector data; these changes do not assert that a particular SOCAL frequency is the correct sector.

Radio pronunciation checks cover C-GTLT, runway suffixes, squawk and frequency digits, while leaving altitude values available to the speech provider as ordinary quantities. The real-engine check-in test requires exactly one additional ATC entry after the prepared copilot transmission, preventing duplicate pilot entries. Final pronunciation quality still needs listening with the configured TTS voice in the simulator.


### Voice API and Map validation — 10 October 2026

Core, settings, engine and UI checks cover provider voice metadata, English filtering, radio abbreviation expansion, airport-name speech, Planning callsigns and repeated sector polygons. Real-engine radio checks confirm that international voices remain unchanged with the filter off, that the filter rejects a non-English voice when on, and that a provider-described custom English voice is accepted.

A live query to the configured TTS service returned 54 voices, 28 labelled English. A French Kokoro voice accepted English radio text and returned a 24 kHz mono PCM WAV. This proves request/synthesis support, not accent quality; listening remains a user check. Real scenery around KLAX returned airport markers and controller coverage; both voice and Map pages were rendered and visually inspected outside the simulator. Map labels use installed data, and no new boundaries were fabricated. Live aircraft movement, hover interaction and end-to-end handoff acceptance remain simulator checks.

## Radio exchanges and map controls — 10 October 2026

Core, engine and UI tests pass. The real-engine radio exercise checks named destination clearances and readbacks, operational airborne check-in replies, and a copilot runway-vacated report that leaves the crossing route active until the prepared reply is committed. Radio-busy telemetry cannot issue the next taxi clearance during that report. The onward route then requires its own readback. Speech-library checks and the Linux release build pass.

The debug map was rendered with installed airport and controller data. Airport visibility controls both drawing and hover details. Terrain sampling uses X-Plane probes on the simulator thread, with 24 probes per flight tick and a local cached grid; it is not a worldwide terrain dataset. Native terrain appearance and the full audio exchange still need simulator acceptance.

Phraseology references: [FAA AIM radio communications](https://www.faa.gov/air_traffic/publications/aim_html/chap4_section_2.html), [FAA departure procedures](https://www.faa.gov/air_traffic/publications/aim_html/chap5_section_2.html), and the [FAA runway-clear definition](https://www.faa.gov/air_traffic/publications/atpubs/pcg_html/glossary-c.html). Terrain API: [XPLMProbeTerrainXYZ](https://developer.x-plane.com/sdk/XPLMProbeTerrainXYZ/).

The updated Linux engine, plugin and speech files were installed after X-Plane closed. Installed engine and plugin hashes match the release build. Git was not changed.

## New flight confirmation — 12 October 2026

The confirmation was rendered inside a 760 × 760 plugin window within a larger viewport. An ImGui input regression uses an offset 560 × 650 host window in a 2560 × 1440 viewport, verifies both button bounds stay inside the host, and clicks Cancel, Start new flight and Retry. The start click queues a reset and advances the flight-local reset generation; a failed connection restores the confirmation with a visible error. UI and engine tests, the Linux release build and the real-engine radio/reset exercise pass. Native simulator input remains a user acceptance check.

## Controller, provider and GPU changes — 13 October 2026

The shared Rust suite passes for core, engine, UI, plugin, TTS, STT, AI server and xtask. Installed-scenery tests remain opt-in; the KLAX containment regression was also run explicitly against the local X-Plane installation and passed at the three reported aircraft positions. It confirms KLAX ownership and usable published Delivery/Ground/Tower entries without airport-specific production logic.

New regressions cover strict model-request JSON, independent/global CPU flags, the full eight-voice Kitten roster with no reuse until exhausted and saved controller identities, touchdown followed by runway exit and separate/combined Ground duties, and parking completion only at an approved final stand. The radio, crew and speech integration suites pass against the release companion engine. Linux plugin/engine dependency audits pass.

The AI/STT/TTS targets pass strict Clippy and formatting checks. The broader engine Clippy pass still reports existing core pedantic warnings (including public API documentation and geometry conversions); the complete workspace is not claimed Clippy-clean. Actionlint accepts the native build workflows. Linux GPU binaries were compiled and exercised on Fedora with actual models; the changed Windows/macOS GPU jobs have not been run on hosted runners.

On the remote RTX 2080 Ti server, Qwen JSON completion was exercised repeatedly, speech was generated on CUDA and CPU, and Whisper transcribed a generated radio sample through the HTTP API. With cuDNN unavailable, the speech warm-up selected CPU and the same synthesis request returned valid audio. Measurements and their scope are in the README and [AI server guide](AI_SERVER.md).

Whisper Tiny misrecognised the heading in one synthetic phrase ("two seven zero" became "to 770"). This is not evidence that operational speech recognition is reliable for every phrase; requests must retain numeric validation, and live microphone/radio testing remains necessary. All eight Kitten presets generated valid finite audio in the Rust CPU audition run. Automated validity checks do not establish a listening-quality preference.

Remaining live acceptance includes the full departure-to-parking flight, embedded UI/audio turn-taking, third-party aircraft controls, and provider voices on native Windows/macOS. No complete live flight or worldwide procedure certification is claimed.

## KittenTTS 2 GPU evaluation — 13 October 2026

An isolated Rust/PyO3 benchmark exercised the upstream CUDA runtime on the RTX 2080 Ti using the compact `emb4` checkpoint. All 47 voice names were discovered; five voices were sampled across eight requests per decoder. Both the default and student decoders returned valid audio. Repeated heading/climb instructions took 5.62–5.69 seconds; the test added 5,088–5,268 MiB above the resident OpenATC service's GPU allocation. A trial FP16 conversion failed on invalid probabilities and was not adopted. Full measurements and limitations are recorded in [AI server evaluation](AI_SERVER.md#kittentts-2-evaluation).

The production service stayed on Kokoro throughout. Its health endpoint and synthesis were checked after the benchmark, and the test GPU processes exited. Version 2 was not added to the Rust application, and no project dependency or simulator file changed during this evaluation.

## Bluebell installation and controller buttons — 13 October 2026

The engine downloaded all 12 publisher-hosted DDS archives, extracted 4,216,196,964 bytes, installed the pack and recovered its installed status after restart. Archive-path rejection and download cancellation/retry checks pass. The pack and download cache were subsequently removed from the plugin installation at the user's request so they can test a fresh download from Traffic.

A UI-client test against the release engine and installed KLAX scenery switches Ground → Delivery, checks that IFR clearance becomes available, submits the request immediately, reads it back, and switches back to Ground. It checks taxi/start-up availability and hides start-up when engines are running. The cold-start run exposed a station-discovery timeout from repeated whole-catalogue capability scans; grouping by airport and deduplicating scenery paths fixes that path. Concurrent index requests now share one scan.

Station-list tests cover airport ownership over a nearer heliport, automatic ground/airborne/arrival lists and the absence of a persistent ICAO filter. Core, engine, plugin and UI regression suites pass, as does the real-engine radio replay. Linux release builds pass. Clippy completes with advisory warnings. These checks do not establish live X-Plane UI, aircraft-specific radio readback, or Windows/macOS runtime acceptance.
# Active source checkpoint — 14 October 2026

The current generated-traffic source adds worker simulation, asynchronous SDK instances, bounded terrain placement and TCAS ownership/cooperation. The installed Nice/Bluebell ground replay passes actual taxi movement, crossing readback before movement, vacated reports and onward taxi to the final departure hold. It selects airliners and ATR turboprops; piston GA needs a separate suitable airport fixture. These changes are not installed/native-accepted and do not complete airborne generated flights, separation, recovery or the broader traffic scope.

The strengthened real HTTP-engine IFR replay imports an internal STAR ceiling and requires an automatic descent clearance/readback before lowering telemetry. It exposed and repaired initial-fix capture and passed-leg cross-track projection. Separate, combined and Tower-only controller arrangements pass through approach/Tower/landing, runway exit and taxi-in crossing to the assigned gate. Actual LFMN–LIMC/ToLiss runtime acceptance remains open. The replay report is `packages/flight-validation/report.json`.
