# Remaining work

Current priorities include IFR reliability and generated traffic, with bounded multicore execution on Linux, Windows and macOS. The items below distinguish completed foundations from planned work.

The 0.2 interface, online OFP import, local navdata reader, stage controller, taxi graph, simulator airport-weather/ATIS reporting, audio settings and HTTP/TLS separation are implemented in source. Check `VALIDATION.md` before assuming runtime coverage.

Current Rust work includes native audio selection, plugin-owned engine lifecycle, improved maps/Arrival profiles, local-frequency tuning, ToLiss A20N datarefs and the recursive editable speech library. See the READMEs and current validation section.

Still needed for a complete ATC system:

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
