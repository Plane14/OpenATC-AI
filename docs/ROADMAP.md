# Roadmap

## Current delivery order

Generated traffic integration comes first, followed by traffic performance validation, remaining conversation quality, traffic coordination and optional pilot exchanges, the outstanding takeoff-callout timing investigation, and the wider plugin backlog. Debug checkpoint acceptance follows those items; release/build/publication leftovers remain the final catch-all. Coordination and movement-authority requirements remain prerequisites of safe generated-flight integration.

A completed Linux test flight confirms the reported window, speed-instruction, controller-message, crew, taxiway-speech and descent-readback repairs worked. Spoken V2 has been removed. Airspeed alive, 80 knots, V1 and Rotate precede Positive rate after confirmed airborne climb; native playback timing confirmation remains pending. Installer/platform acceptance is delegated to community testers, including Intel and Apple Silicon Macs, and is not claimed complete.

## Crew conversation pipeline — 16 October 2026

Dedicated bounded role dialogue, ordered chat history, exact captain-quote continuity, current-request control isolation, flight/aircraft reset guards and identical token-count/generation framing are implemented. Current situation and total planned minutes now distinguish predeparture, airborne and postlanding conversation. Focused tests and HTTP-engine crew/radio/flight replays pass; these pipeline changes are installed for Linux testing, with conversation quality and native acceptance still open.

The remaining acceptance work is real-model conversation completeness and consistency: nested food/banter requests, personal-fact ownership, duplicate control claims, joke variety and copilot latency. Passing mapped-action or required-word checks alone does not close this work. See `VALIDATION.md` for the tested boundary. Generated traffic integration and native simulator acceptance remain separate milestones.

## Flight repairs — 16 October 2026

The latest Linux build includes the following repairs:

- Restore saved floating-window bounds after applying UI-scale limits; preserve the compact/full anchor and splitter state.
- Combine descent clearances with due published speed restrictions, including the crossing fix and exact or maximum speed qualifier.
- Validate complete level/speed readbacks against the current clearance sequence, including spoken numbers and climb/descent direction.
- Avoid reporting a climb back to the ILS intercept altitude during final Tower check-in.
- Retain editable crew pacing guidance, taxiway pronunciation, ATIS reception updates and independent takeoff-callout playback.

The locked Linux release build, focused readback/descent tests and four HTTP flight-replay arrangements pass. The replays cover a published STAR speed restriction, descent, approach clearance, Tower transfer, landing, runway exit, crossing and parking. The completed test flight confirms these repairs, with the corrected takeoff callout sequence awaiting a native recheck. A ToLiss Nice–Milan flight through parking was completed successfully. Generated traffic and the wider AI pipeline remain in development; generated traffic is disabled by default.

Operational wording is in `speech/runtime/responses.toml`. Full-file overrides must retain the required situation IDs and slot contracts; compare customised files with the shipped defaults when upgrading.

# Remaining work

Current priorities include IFR reliability and generated traffic, with bounded multicore execution on Linux, Windows and macOS. The items below distinguish completed foundations from planned work.

Current priorities are complete generated-flight integration and traffic performance validation, followed by remaining crew conversation completeness and latency. IFR descent, approach clearance and Tower transfer have been exercised in replays and a native Nice–Milan flight. The developer jump menu includes surface holds/runway start and source-tested SID/cruise/STAR/pre-approach checkpoints using native flight initialization and confirmed-position route synchronisation. Native ToLiss/default-aircraft checkpoint acceptance remains open; final/landing are flown normally so the tool does not manufacture those permissions. Native aircraft acceptance remains separate from replay results.

The 0.2 interface, online OFP import, local navdata reader, stage controller, taxi graph, simulator airport-weather/ATIS reporting, audio settings and HTTP/TLS separation are implemented in source. Check `VALIDATION.md` before assuming runtime coverage.

Current Rust work includes native audio selection, plugin-owned engine lifecycle, improved maps/Arrival profiles, local-frequency tuning, ToLiss A20N datarefs and the recursive editable speech library. See the READMEs and current validation section.

Still needed for a complete ATC system:

- Native acceptance of preloaded copilot takeoff callouts: verify actual ToLiss and default-aircraft speed sources, short-gap timing, rejection/reset, pause and output devices. Source trigger and crew replay tests do not close this acceptance item.

- Simulator runtime validation across X-Plane UI scales, multi-monitor and pop-out configurations; native SDK 4.4 graphics integration, non-ASCII keyboard entry.
- Complete traffic separation, wake sequencing and plausible random traffic generation without real-world timetables. Native AI amount control, predictive taxi holds, whole-aircraft runway clearance, concurrent traffic advisories, limited jet approach spacing and simulated pilot weather/landing exchanges are implemented. FAA pair-specific departure/approach wake checks cover observed same/close-parallel runway traffic; intersecting paths and preceding low/missed approaches remain incomplete. These paths need live traffic-provider and performance acceptance.
- Complete the generated airborne lifecycle on the bounded CPU pool. Scenery/navigation loading, CSL indexing, generated ground-route preparation and movement already use workers. Queue/reset regression tests and immutable snapshot publication pass; distance-based work budgets and measured simulator frame time remain acceptance requirements on Linux, Windows and macOS.
- Complete lightweight CSL traffic integration. Settings → Traffic installs Bluebell with download/extraction progress, cancellation and retry. Bounded asynchronous instance loading, interpolated positions, model animation, ground probing and TCAS cooperation are implemented in source. Generated ground movement includes radio-gated pushback, taxi, crossings, vacated reports and Tower handoff, with an installed Nice scenery replay. Full generated airborne flights, mixed-flight sequencing and native visibility/audio/FPS acceptance remain unfinished; generated traffic stays disabled by default. The follow-me car is one scenery instance, not an aircraft traffic controller. X-Plane's documented initialization API does not expose our own per-aircraft route/clearance schedule. CSL packs are visual assets rather than native flight models.
- KittenTTS 2's 47-voice production integration. The GPU/VRAM benchmark is recorded, but the selectable server provider currently remains the older eight-voice Mini runtime. A Rust native prototype produces correct CPU audio; its Vulkan path returns invalid logits and is not enabled in the server. Kokoro stays the default. Full-roster validation, GPU compatibility and cross-platform packaging checks are still needed before advertising Kitten 2 as supported.
- Broader enroute/arrival acceptance against installed controller coverage. Current and predicted sector handoffs, copilot tuning and check-ins are implemented; coverage gaps, complex overlapping sectors and worldwide routing still need simulator testing.
- Aircraft-size/obstacle validation beyond the current painted-centerline fallback, taxi-route amendments and richer holding-point metadata. Separate runway crossing clearances, crossing readbacks and onward taxi are implemented and checked against installed KLAX scenery; broader simulator acceptance remains. Conservative centerline fallback and separate Tower-authorized runway backtracking are implemented.
- Navigraph developer registration, entitled-user authorization and licensed chart rendering. No working login or chart support is claimed yet.
- Precipitation radar tiles, map terrain/coastline base layers, forecast fields and simulator-weather grids. Current radio weather comes from airport-location simulator surface samples, not radar.
- Aircraft-specific performance, mass/fuel loading, route generation and restrictions. Current dispatch data is imported from SimBrief or entered by the user.
- Complete diversion routing and scenario-driven runway closures/vector offers. Optional direct-to offers are implemented with terrain, restriction and descent checks. The Missions selector/editor supports eight A320-family stories, phase/probability triggers, branches, dialogue and a journal. Mission media fields are editable metadata; playback and automatic assistance dispatch remain unfinished.
- Gate occupancy beyond TCAS, including static scenery aircraft and coordinated relocation of OpenATC-owned traffic once that exists. Current taxi-in rechecks known occupancy and can hold/reassign a suitable stand; it does not remove another provider's aircraft.
- Aircraft-specific radio overrides where a third-party aircraft does not honor X-Plane's standard COM1 dataref; hardware push-to-talk binding and broader speech-cancellation controls.

Editable speech follow-through:

- Add explicit IFR/VFR state to the flight plan and operational controller flow; `/suggest` already accepts it, but its default remains IFR.
- Extend checked live-slot speech dispatch beyond the implemented departure clearance/start/pushback workflow; expand spoken readback recognition beyond the current NATO/English matching, VFR clearance flow and service-specific airborne requests.
- Add country/FIR/airport procedure packs with authority, section and effective-date review, including metric flight levels, national RNAV terminology and local language support. The existing broad regional folders are a baseline, not completed worldwide procedure coverage.
- Model eligibility for conditional runway operations, visual/SVFR services, CPDLC, intersection performance and LAHSO before wiring those example situations into automatic clearance generation.
- Add speech hot reload and a user-facing editor/validation report. Current changes load on engine restart; the offline checker and personal-library override are available now.

The current sprint implements nearby station discovery, unconditional reception gating, service-aware departure permissions, simulated ATIS and optional conservative AI wording. Full sector routing, terrain radio shielding, remote radio sites, worldwide procedural certification and end-to-end simulator acceptance remain separate work.

## Crew follow-up

- Validate the new ToLiss panel routing and each custom control in the running neo. Button-only mappings expose a press, not a verified engaged state.
- Identify a supported ToLiss pushback distance/angle interface. Starting its already-configured sequence is supported; spoken distance/angle changes are not applied.
- Add reviewed, complete aircraft/operator checklists; current examples are abbreviated.
- Add held-control semantics where the SDK requires more than a tap.

New flight clears active flight state without unloading the plugin. Live header reset and disable/re-enable checks remain part of simulator acceptance; automated checks cover reset, pending actions and delayed responses. Native per-holding-point light control is not implemented.

Generated traffic now retains bounded runway-compatible terminal candidates on the preparation worker, including connected STAR/approach entries, published recovery legs and known MORA enroute floors. Repeated join-fix constraints are intersected. These inputs do not complete the airborne flight factory or enable traffic; see `TRAFFIC_PLAN.md` for the remaining trajectory/controller/native acceptance work.

The player route now uses ordered installed SID/STAR parts with selected runway transitions, preserves altitude/flight-level and minimum/maximum speed restrictions, and exposes the assumed route in the debug map. Later straight STAR-leg telemetry can recover a missed sequencing update under tight position/course checks. Anonymous vector/altitude legs and curved geometry still need complete native/FMS-route acceptance; dashed map connectors are diagnostic, not direct clearances. Cabin landing preparation/passenger-watch services and unfamiliar LLM paraphrases have focused coverage; native ToLiss panel deselection and taxi-aid timing remain user checks.

Crew narration now uses original compound requests, recent role-specific conversation and verified outcomes with bounded TOML examples, rather than returning a canned recognised-request response whenever AI is enabled. Current seating and editable fictional cabin mood support focused follow-up answers. Control terms/numbers and unconfirmed results remain guarded, with fallbacks. The debug checkbox is **Flight path**; filed arrivals into a STAR initial fix are continuous, and containing controller areas receive a dark translucent tint independent of tuning. Complete anonymous/vector/arc geometry and native actor/map acceptance remain separate work.

## Generated full-flight continuation — 14 October 2026, joint scheduling

Fixed a production registration gap: generated/flight::install now atomically retains the Prepared clearance/recovery plan when replacing the movement world. The complete-flight protocol test no longer inserts Runtime.flights itself; failed installation asserts no active-flight/plan mutation, and clearance-to-parking still passes.
New `crates/engine/src/generated/airborne.rs` composes ordered Climb/Cruise/Descent/Approach crossings into one trajectory solve so later STAR ceilings constrain preceding enroute levels. Every sample still requires caller-provided protection; incomplete/unprotected inputs reject. This module is NOT YET CALLED by production scene preparation. Generated traffic remains disabled until the factory is integrated and validated. Actual retained terminal decoding/environment/protection, speed feasibility, recoveries, real rollout/taxi-in/stands and typed clearances still need wiring into complete Prepared flights.
The complete-route regression exposed inconsistent time units: trajectory used rounded 0.514444 metres/sec per knot while movement uses exact nautical-mile/hour. Corrected trajectory conversion to 1852/3600; movement validation allows only relative 1e-9 floating point roundoff, not operational altitude tolerance. Joint future ceiling, protected-path rejection and appended rollout/taxi-in route compilation pass. Nine generated source tests pass, five licensed-data tests remain ignored; ten motion tests and the trajectory test pass. `cargo check --locked -p openatc-engine -p openatc-plugin` passes. These are historical source results, not native traffic acceptance.
The remaining production work uses retained routing::Terminal inputs plus fresh origin/destination atmosphere/wind to resolve all sections and joint speeds, verified terminal/enroute terrain protection and published recoveries, then attaches landing/gate and controller plans before populating production flights. Native visibility/audio/FPS/platform acceptance remains open.
