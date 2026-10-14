<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/branding/logo-dark.png">
  <img src="assets/branding/logo-light.png" alt="OpenATC AI" width="600">
</picture>

# OpenATC AI

Free, open-source ATC and cockpit crew conversations for **X-Plane 12**. OpenATC combines a Rust controller engine, editable TOML phrasebooks, voice input/output, simulator telemetry and aircraft control profiles. You can speak, type or use the available request buttons.

The **AI server can run on another computer**. Keep its language model, speech recognition and speech synthesis away from the simulator's CPU and GPU, or run everything locally if you prefer. The included models run locally without subscriptions, API token payments or a cloud account. Internet access is needed for the first model download and optional online services such as SimBrief.

OpenATC's application code is MIT-licensed. Models, libraries, fonts and simulator data have their own terms; see [third-party notices](docs/THIRD_PARTY.md). This is an early-development flight-simulator project. A phrase in the library does not mean that every associated procedure, aircraft or regional rule has been implemented or validated.

## Contents

- [What is included](#what-is-included)
- [Install and connect](#install-and-connect)
- [AI server, voices and GPU selection](#ai-server-voices-and-gpu-selection)
- [Fly an IFR flight](#fly-an-ifr-flight)
- [Copilot, cabin and ground crew](#copilot-cabin-and-ground-crew)
- [Missions and flight stories](#missions-and-flight-stories)
- [Emergencies and assistance](#emergencies-and-assistance)
- [Traffic and performance](#traffic-and-performance)
- [Maps, stations and simulator guidance](#maps-stations-and-simulator-guidance)
- [Settings and realism](#settings-and-realism)
- [Customise speech and aircraft](#customise-speech-and-aircraft)
- [Builds and packages](#builds-and-packages)
- [Development and validation](#development-and-validation)
- [Troubleshooting](#troubleshooting)

## What is included

OpenATC brings the controller and crew into the same flight. Request your IFR clearance, taxi through runway crossings, check in with the next controller, then work through approach, landing and taxi to parking. Speak naturally, type a request, or use buttons that follow the current phase and controller's duties.

- **A helpful copilot:** spoken readbacks, handoff tuning and check-ins, aircraft controls and read-or-perform checklists. Replies wait for controller speech and are tied to the instruction being answered.
- **Cabin conversations:** ask for coffee, check on passengers, prepare the cabin for turbulence or request mapped door/slide actions. Editable personalities and wording provide variety, with restrained humour when the situation permits it.
- **Ground services:** ask to remove chocks and disconnect external power in one message. Aircraft controls are confirmed before the crew reports success.
- **Traffic you can hear and talk to:** choose native AI compatibility or suppress it, listen to background exchanges, and ask observed traffic about weather, landing conditions or crosswind. ATC can point out traffic, hold for taxi conflicts and protect occupied runways.
- **Stories you can create:** select an A320-family mission or edit your own triggers, dialogue, choices and outcomes with the mission form and advanced TOML editor.
- **An open local setup:** run the AI server on a second machine, choose voices, edit phrasebooks and aircraft profiles, and keep model inference local without paid API tokens.

| Area | Current behaviour |
| --- | --- |
| IFR departure | Planning/SimBrief import, clearance, readback checks, start-up/pushback permissions, taxi, runway crossings, Tower handoff and takeoff permission. |
| Airborne controller | Check-ins, sector handoffs, altitude requests, direct-to requests, heading/speed assignments, lateral deviations and selected procedure/runway amendments. Eligibility depends on phase, controller role and available navigation data. |
| Arrival | Separate approach and landing permissions, Tower check-in, touchdown/runway-exit monitoring, Ground handoff and taxi to a selected stand. |
| Copilot | Optional spoken readbacks, frequency changes and check-ins, mapped aircraft controls, and two checklist modes. |
| Ground and cabin crew | Compound control/service requests, one confirmation after simulator acknowledgements, request history, refreshment requests and a simulated passenger cabin. |
| Missions | Eight A320-family flight stories, mission selection, a form/TOML editor, phase and probability triggers, choices and a flight journal. |
| Traffic | Simulator traffic inspection, optional native AI count control, taxi conflict holds, runway protection, limited approach spacing and observed-weather pilot conversations. |
| Speech | Local Whisper recognition, Kokoro synthesis by default, optional Kitten Mini 0.8, editable phrasebooks and optional LLM wording variety. |
| Simulator data | Published airport frequencies, combined controller duties, taxi networks, runways, stands, installed navigation data and controller coverage. |
| Interface | Resizable in-simulator window, Light/Dark themes, transparency; Planning, ATC, Radios, Taxi, Arrival, Airports, Traffic, Missions and Settings pages; developer Map and testing tools. |

These are implemented paths, not a claim of complete worldwide ATC coverage. Automated checks exercise important transitions and failure cases. Full aircraft-specific flight testing, separation, procedure interpretation and native platform acceptance remain ongoing; see [Validation](docs/VALIDATION.md) and [Remaining work](docs/ROADMAP.md).

Further work includes random mixed traffic, lightweight Bluebell rendering, wider wake sequencing and arrival holding. The [roadmap](docs/ROADMAP.md) distinguishes these plans from implemented features.

## Install and connect

### The three running components

1. **X-Plane plugin:** displays the interface, captures audio, plays speech, reads simulator state and applies supported aircraft controls on the simulator thread.
2. **Companion engine:** runs beside the simulator and owns the flight plan, controller state, readbacks, requests, monitoring and conversation. The plugin starts it automatically.
3. **Optional AI server:** runs the LLM, STT and TTS locally or on another machine. It exposes one HTTP service for all three.

The AI server still launches a bundled `openatc-stt` helper. The language and recognition libraries currently contain incompatible copies of a native dependency, so recognition remains in a child process. Kokoro and Kitten run inside the AI-server process. Install the supplied bundle rather than copying only the main executable.

### Install the plugin

Close X-Plane before replacing plugin files. A packaged plugin installer asks for the **X-Plane root folder**, not the aircraft folder. A complete plugin payload can also be copied into `X-Plane 12/Resources/plugins/OpenATC`.

For a source build:

```sh
cargo build --locked --release -p openatc-plugin -p openatc-engine
cargo xtask install-plugin "/path/to/X-Plane 12"
```

The installer checks speech files, makes a recoverable backup outside the plugin directory, and installs the plugin, engine and editable resources. It preserves personal settings and existing station-role overrides. Do not rename the engine executable into an `.xpl`: the simulator module and companion engine are different binaries.

The simulator module is `64/lin.xpl` on Linux, `64/win.xpl` on Windows and `64/mac.xpl` on macOS. Keep the rest of the OpenATC folder with it. Restart X-Plane and open **Plugins → OpenATC**.

### Start the AI server

From an extracted AI-server package:

```sh
# This computer only.
./openatc-launcher

# Accessible from other computers on your LAN.
./openatc-launcher --host 0.0.0.0 --port 8099
```

On Windows use `openatc-launcher.exe`; in PowerShell prefix it with `.\`. The launcher passes its options to the AI server. First startup downloads and verifies the selected models. Subsequent starts reuse the model cache. Keep `openatc-stt`, the supplied shared libraries and `espeak-ng-data` with the server.

In **Settings → General**, enter your **AI server address** and apply it to all three services. This preserves model and voice choices. Separate providers can still be configured under **AI** and **STT / TTS**. Example addresses:

```text
http://127.0.0.1:8099        same computer as X-Plane
http://SERVER-LAN-IP:8099   separate AI computer
```

Use an origin without `/v1`. If the server is remote, `127.0.0.1` refers to the simulator computer and will not reach it. Allow TCP 8099 on the server's local firewall. The bundled service has no authentication or TLS termination; bind it only where your intended clients can reach it. Local inference does not require provider API keys.

Check readiness from the simulator computer:

```sh
curl http://SERVER-LAN-IP:8099/health
curl http://SERVER-LAN-IP:8099/v1/voices
```

A running process is not enough: wait for a healthy response after model loading. The health response includes the selected TTS provider and each component's compute status. See [the AI server guide](docs/AI_SERVER.md) for build commands, GPU requirements, API examples and measurements.

### Select microphone, speakers and voices

In **Settings → Audio & microphone**, refresh devices and select the microphone and output. Test the input before flying. Linux supports PulseAudio/PipeWire, with ALSA fallback; recording lists exclude playback-monitor devices. System default follows the operating system's current routing. After changing hardware, refresh the list and reselect missing devices.

In **Settings → Voices & personalities**, refresh the provider's voice list. Test voices with the editable preview sentence before adding them to the controller pool. Role volume and voice effects are separate from the operating-system output volume.

**Speak my transmitted requests** controls synthesized playback of your own pilot transmissions and is off by default. Copilot radio replies have their own setting. Seeing your text in the log does not imply that pilot playback is enabled.

### Data locations

| Data | Default or override |
| --- | --- |
| Engine settings and logs, Linux | `~/.config/openatc/settings.json`, `~/.config/openatc/engine.log` |
| Engine settings and logs, Windows | `%LOCALAPPDATA%/openatc` |
| Engine directory override | `OPENATC_CONFIG_DIR` |
| AI models, Linux | `~/.local/share/openatc-ai/models` |
| AI models on any OS | Platform application-data directory; override with `OPENATC_AI_MODELS` or `--models-dir` |
| Model integrity record | `models.lock.toml` in the model directory |
| Persistent controller identities | `controllers.json` in the engine configuration directory |
| Speech override | `OPENATC_SPEECH_DIR`, or a complete personal speech library |

Settings, downloaded models and controller identities are separate from packaged binaries. Back them up before moving machines. Optional third-party credentials belong in `OPENATC_AI_KEY`, `OPENATC_STT_KEY` and `OPENATC_TTS_KEY`, not in speech TOMLs. `OPENATC_AI_URL` and `OPENATC_AI_MODEL` override the engine's initial AI settings.

## AI server, voices and GPU selection

### Included model choices

| Component | Default | Purpose |
| --- | --- | --- |
| Language model | Qwen2.5 7B Instruct, Q4_K_M | Interpret requests and produce optional natural wording. |
| Speech recognition | Whisper Tiny | Turn microphone audio into English text. |
| Speech synthesis | Kokoro v1, 82M parameters | 54 voice presets, including 28 English presets. |
| Optional synthesis | Kitten Mini 0.8, 80M parameters | Eight English presets in a smaller model download. |

Kokoro stays the default. Kitten Mini is selected explicitly; it is not the larger KittenTTS2 model. Only one TTS model is loaded by a server instance. Switching providers requires a server restart and a voice refresh in the plugin. The model files are downloaded separately from the application.

```sh
openatc-ai --tts-provider kokoro
openatc-ai --tts-provider kitten
```

Kitten exposes Bella, Bruno, Hugo, Jasper, Kiki, Leo, Luna and Rosie through `/v1/voices`, using IDs such as `kitten-bella`. Kokoro's roster comes from its loaded voice table. The engine selects from the actual provider roster rather than assuming a fixed set of six names.

**English-language voices only** is enabled by default. It filters language, not accent. Disable it to audition international presets with English text. A model's number of supported languages is not its number of different voices. Kokoro's current OpenATC text preparation is English; non-English presets speaking English require listening tests and do not turn the controller into a multilingual service.

Controller voices are saved by controller identity and reused on later visits. New controllers use unused voices before reusing an exhausted pool. With eight Kitten presets, more than eight controllers necessarily share voices. A saved voice absent from the newly selected provider is replaced with an available one. The selected provider therefore affects both variety and pronunciation.

### GPU preferred, CPU available

An accelerated build tries its GPU backend first and falls back to CPU if initialization or the TTS warm-up fails. There are four independent command-line overrides:

| Flag | Effect |
| --- | --- |
| `--force-cpu` | Force LLM, recognition and synthesis to CPU. |
| `--force-llm-cpu` | Force only the language model to CPU. |
| `--force-stt-cpu` | Force only Whisper to CPU. |
| `--force-tts-cpu` | Force only Kokoro or Kitten to CPU. |

For example, leave the larger LLM on the GPU while running speech synthesis on CPU:

```sh
openatc-ai --tts-provider kokoro --force-tts-cpu
openatc-ai --tts-provider kitten --force-tts-cpu
```

Or run without GPU inference:

```sh
openatc-ai --force-cpu
```

The global flag takes precedence over individual choices. CPU inference uses system RAM; it does not require dedicated VRAM. It can be slower, particularly for the LLM. Running the server on another computer is usually preferable to taking resources from a busy simulator.

GPU support depends on the compiled features and installed drivers. Linux/Windows native AI release jobs enable Vulkan for the LLM and Whisper, and CUDA for ONNX speech. macOS jobs enable Metal/Apple support and CoreML speech. CUDA TTS needs the matching NVIDIA CUDA/cuDNN runtime libraries; an NVIDIA driver alone is insufficient. AMD/Intel Vulkan can accelerate LLM/STT while TTS falls back to CPU. Plain source builds without these optional features remain usable on CPU.

The server warms up GPU speech before reporting ready because some CUDA libraries load only when inference first runs. `/health` reports a fallback reason if it cannot use the accelerator. Native startup logs identify the actual GPU: a `gpu-requested` status alone is not a VRAM measurement. Hardware requirements, measured usage and source-build flags are in [AI server setup and measurements](docs/AI_SERVER.md).

### Measured VRAM and speech speed

Measured **13 October 2026** on Fedora 44, an **RTX 2080 Ti (11,264 MiB / 11 GiB)**, NVIDIA driver **615.71.09**, Core i9-9900K and 32 GB RAM. Every run loaded Qwen2.5 7B Q4_K_M with a **4,096-token context**, Whisper Tiny and the selected TTS model. The GPU runs used Vulkan for Qwen/Whisper and CUDA for speech, with CUDA 13 and cuDNN 9.27. These are exact observations from this test, not fixed requirements for every installation.

| Configuration | AI/STT process VRAM after startup | Whole-device sampled peak | Speech generation, three requests |
| --- | ---: | ---: | ---: |
| Kokoro, all three on GPU | 5,520 MiB | **6,059 MiB** (5.92 GiB) | **0.15–0.20 s** |
| Kitten Mini, all three on GPU | 5,042 MiB | **5,499 MiB** (5.37 GiB) | **4.51–4.73 s** |
| Kokoro on CPU; LLM/STT on GPU | 4,794 MiB | 5,025 MiB | 1.90–1.95 s |
| Kitten on CPU; LLM/STT on GPU | 4,794 MiB | 4,926 MiB | 5.22–5.40 s |
| Kokoro, LLM and STT all forced to CPU | 8 MiB driver context; no inference tensors on GPU | 20 MiB | 1.78–1.97 s |

The sentence was “Charlie Golf Tango Lima Tango, turn left heading two seven zero. Climb and maintain five thousand feet.” Kokoro's `af_bella` produced 7.21 seconds of audio; Kitten's `kitten-bella` produced about 11.08 seconds. Both were requested at speed 1.0. Different delivery lengths mean this is not a listening-quality ranking. Kokoro was substantially faster here, so it remains the default; Kitten is available for its alternative voices and smaller download.

Startup figures sum the two GPU processes after model loading and the GPU speech warm-up. Peaks were sampled every 100 ms through three speech requests, a JSON completion and recognition of the same sample, with all models resident. Requests ran sequentially. The idle machine baseline was 1 MiB; NVIDIA also reported 429 MiB reserved separately. Sampling can miss shorter peaks, and concurrent inference or longer prompts/utterances can require more memory.

After these requests, the GPU processes retained 6,040 MiB for Kokoro and 5,274 MiB for Kitten, versus 4,802 MiB with TTS on CPU. The extra retained allocation was therefore approximately **1,238 MiB for Kokoro TTS** and **472 MiB for Kitten TTS** in this particular run. Those deltas include allocator/workspace effects and are not standalone model-weight requirements.

All tested configurations fit this 11 GiB card on a separate AI machine. Leave additional headroom if X-Plane or other applications share the GPU. The global CPU flag keeps inference on CPU even though a GPU-enabled binary can create a small driver context during backend discovery. A CPU-only build does not require an NVIDIA card. See [measurement reproduction](docs/AI_SERVER.md#measurements-and-reproduction) for the commands and limits.

### KittenTTS 2 evaluation

KittenTTS 2 has **47 voices (38 English and nine non-English)**. It is a different model from the eight-voice Mini 0.8 option above. It was tested separately on the same RTX 2080 Ti; it is not integrated into the OpenATC server.

The compact `emb4` language-model file is 492,114,467 bytes (469.3 MiB). That is not the complete runtime: the upstream GPU implementation also downloads a 1,064,875,036-byte audio decoder, voice assets and, when selected, a 38,817,934-byte student decoder. The language-model weights are expanded for inference.

With the model already loaded, the heading/climb sentence above took **5.62–5.69 seconds** to generate, compared with **0.17–0.29 seconds** in a fresh Kokoro check. The other tested Kitten instructions took **4.26–9.89 seconds**, depending on voice and text. Kitten added about **4.97–5.14 GiB** to GPU use. The whole device peaked at **10.40–10.58 GiB** while the existing LLM, Whisper and Kokoro also remained loaded. These totals include both TTS models and are not the memory requirement for replacing Kokoro with Kitten.

Both the default and smaller student decoders produced valid audio. Converting the language model to FP16 failed during generation, so that trial supplies no usable faster result. Kokoro remains the recommended built-in model for prompt radio replies. See [the evaluation details](docs/AI_SERVER.md#kittentts-2-evaluation) and [recorded measurements](docs/benchmarks/kitten-tts-2-2026-10-13.json).

A separate Rust native prototype also generated intelligible speech without Python. Its standard Q4 model is 1,447,458,656 bytes, plus a 312,339,253-byte student decoder and voice data. On the same i9-9900K, with four CPU threads, a takeoff instruction took **16.03–17.46 seconds** for 5.08–5.24 seconds of audio. The Vulkan GPU trials returned non-finite logits, so that path is not ready to ship. This prototype is not the selectable Mini provider and has not changed the default server or release dependencies. See the [native experiment](docs/AI_SERVER.md#native-kitten-experiment).

### How AI is allowed to act

The model does not issue simulator commands hidden in its prose. ATC interpretation returns bounded JSON, for example:

```json
{"intent":"altitude","altitudeFeet":7000,"waypoint":""}
```

The engine rejects unknown intents, fields and unsupported values. It keeps the original pilot text for readback validation. Model output cannot claim that a clearance was acknowledged, mark itself as controller-initiated, supply a clearance sequence, or write arbitrary datarefs.

Rust then checks the controller's duties, radio state, flight phase, pending exchange, route and constraints. A recognised request may be refused or clarified. Crew interpretation uses a separate bounded list of aircraft-profile control IDs; actual simulator confirmation is still required. Structured JSON improves interpretation without giving the language model control of flight-state transitions.

The bundled chat API supports grammar-constrained `response_format: {"type":"json_object"}`. That guarantees the output format, not the truth or operational correctness of its contents; validation remains in the engine. Unsupported `json_schema` requests return an error.

## Fly an IFR flight

### 1. Plan and request clearance

Create a flight in **Planning** or import a SimBrief OFP. Check the departure, destination, callsign, runway, route and cruise level. A clearance request uses this flight automatically. Aircraft identification and flight callsign can differ, so verify the callsign shown in the plan.

Power the radio and tune a published station in **Channels**. Clearance duties are inferred from installed services: Delivery first, otherwise Ground, otherwise Tower. At an airport with only Tower, that controller can handle clearance and ground duties. Airports and frequencies are not hardcoded for test routes.

Older scenery frequencies restore the omitted final digit: `FREQ 11802` is **118.025 MHz**, so handoffs, copilot tuning, controller replies and request buttons agree with the aircraft radio. Modern 8.33 kHz `CHAN` records retain their exact channel numbers. See [X-Plane's frequency-format explanation](https://developer.x-plane.com/article/8-33-khz-radios-for-users-and-authors/).

Routine pressure calls use **QNH 1022**, without a unit suffix. Settings below 1000 hPa retain “hectopascals” to avoid ambiguity.

The clearance needs a correct readback. **ATC Auto Reply** submits the current pending reply; optional copilot replies can do it for you. A request button is a request, not permission to move. Unclear or incorrect readbacks leave the instruction pending. Routine clearances do not need to announce a list of fields to read back, but those fields remain checked.

### 2. Start-up and pushback

After the IFR readback, Delivery directs you to Ground where one is published. Start-up and pushback are separate permissions; the quick requests reflect existing approvals and observed engine state. A start-up request should not remain available after the engines are running.

ATC approval does not operate the tug or engines. Address **Ground crew** for external power, chocks or the aircraft's pushback control. With ToLiss, set pushback distance and angle in its own panel first; OpenATC can start the configured sequence, but currently cannot apply a spoken distance or angle.

### 3. Taxi and cross intervening runways

Request taxi from Ground or the combined controller. OpenATC builds a route from installed scenery and names published taxiways where available. Read back the route and hold-short limit. The map displays pending routes in amber and approved routes in green; optional simulator arrows appear only for approved portions.

Approved taxiway arrows remain visible from stands within 1,200 metres of the route, even when the stand is outside the published taxi network. A dashed amber ramp connector on the map remains unverified and receives no simulator arrows. Guidance stays bounded to 64 nearby arrow instances.

Each intervening runway is a separate clearance limit. The controller can issue a crossing instruction when you approach the hold and traffic information permits it. A crossing clearance names the runway being crossed, which may differ from the departure runway. Read back the crossing before entering. Unknown traffic information does not count as a clear runway.

After the crossing, the copilot can report vacated at the observed completion point. The next taxi instruction replaces the route and requires its own readback. In manual mode physical completion can advance the crossing workflow; a vacated report is not treated as an altitude readback.

Published taxi networks are preferred. Where they are missing, painted scenery centerlines can provide a conservative fallback. Backtracking is used only when the authorized route runs along the runway. The engine does not invent taxiway names for unnamed scenery paths or treat a drawn path as runway-entry permission.

### 4. Holding point and takeoff

At the final departure hold, Ground sends you to the published Tower frequency. If already on Tower, stay with that controller. Reaching the hold does not invent a ready-for-departure call: report ready yourself or use an available supported copilot exchange.

Holding-point detection considers approach geometry and the aircraft's reference position; it does not require the aircraft to be perfectly stopped to recognise arrival. Tower checks available runway and final-approach traffic and either clears takeoff or explains the hold. A previously reported ready aircraft can receive clearance when blocking traffic clears.

A takeoff instruction may include known wind, pressure and departure guidance. The cleared altitude and departure constraints remain authoritative. The departure handoff is tied to the airborne workflow; accepting a takeoff readback alone should not send the aircraft to another controller while still parked at the hold.

### 5. Departure, enroute requests and constraints

Altitude compliance allows a 300-foot band and a 30-second sustained-deviation delay; an aircraft climbing or descending toward its assigned level is not treated as non-compliant. The plugin supplies geometric, indicated and standard-pressure altitude separately. The engine compares flight-level clearances with standard-pressure altitude using the configured regional transition altitude, and lower assignments with the pilot's barometric altitude. Older clients fall back to geometric altitude when instrument readings are unavailable. Pilot check-ins use the indicated altitude. Debug conversation exports include all three readings for diagnosis.

Check in after tuning the assigned frequency. The airborne check-in includes current altitude and the assigned climb altitude when applicable. Handoffs use current and predicted sector coverage; receiving a frequency is not the same as owning the aircraft's sector. The debug Map displays the controller geometry used by these decisions.

Supported request families include an altitude change, direct to a known fix, a lateral deviation, resuming the route, a speed request, a different approach, and departure/arrival runway amendments. Availability depends on phase and controller role. Requests that cannot be interpreted or supported should be clarified rather than silently applied.

Installed CIFP data supplies selected waypoint altitude restrictions. SimBrief planned levels and step climbs are planning inputs, not permission to climb. The controller must issue an amended clearance, including continued climb/descent where appropriate. Strict constraints preserve published exact, minimum, maximum and window restrictions. Requested climbs also undergo a conservative descent-feasibility check toward the destination.

This is not a complete FMS procedure interpreter: curved/unnamed/vector legs, aircraft performance, winds and local minimum vectoring altitudes remain limitations. MORA corridor checks are conservative terrain references, not a substitute for every local radar vectoring rule. Review the [airborne architecture](docs/ARCHITECTURE.md#airborne-instructions-and-published-restrictions) before relying on a particular route type.

### 6. Approach, touchdown and taxi to parking

Approach permission and landing permission are separate. Follow the selected arrival/approach and controller instructions. When transferred to Tower, check in; a continue-approach instruction is not a landing clearance. Tower's landing decision also depends on available runway traffic information.

Touchdown is detected from simulator telemetry. The arrival controller requests that you vacate the runway. Ground handoff follows observed runway exit, not the first on-ground sample. A combined Tower/Ground station can retain you on the same frequency.

Select a destination stand using the parking list and its filter, then request taxi. Arrival taxi routes have the same runway-crossing and readback limits as departure routes. The flight reaches its completed state only when the approved parking route ends at the selected stand and the aircraft has stopped; stopping at an intermediate runway hold is not the end of the flight.

Stand selection uses the loaded aircraft's wingspan override or ICAO type and the scenery's published A–F width class. An A320/A20N needs at least class C; its departure parking position is not used to guess its size. Too-small, unclassified and observed occupied stands are excluded. If the assigned stand becomes unavailable during taxi-in, Ground holds the aircraft and can issue a new route once stopped. That route needs a readback and retains every runway crossing limit. Static scenery aircraft are not included in the TCAS feed, so their occupancy cannot be guaranteed. OpenATC does not remove another traffic provider's aircraft.

Clearance squawks are allocated per flight from a simulated discrete-code pool and retained through handoffs. New flights avoid recent allocations. This is not a reproduction of each country's operational code allocation.

### Radio turn-taking

**Show COM2 communications** in Settings → Copilot adds a second ATC icon with a small **2** beside the first. Both pages use the same flight and station roster: a station keeps its controller voice when reached on either radio. COM2 has its own tuned channel and supports the compact communications view. The `openatc/transmit_com1` and `openatc/transmit_com2` X-Plane bindings target their named radios from any page; the existing `openatc/talk` binding retains the selected crew/radio workflow. Dedicated radio bindings bypass ATTN/MECH interphone selection. Push-to-talk captures the recipient and radio when recording starts.

The source's complete HTTP flight replay includes a COM2 flight from clearance through taxi to gate, checks that COM1 remains unchanged, and retains COM2 through readbacks and handoffs. Native aircraft audio-panel reception and third-party COM2 bindings still require simulator acceptance; these replays do not establish native audio or FPS.

Routine automatic ATC transmissions wait while the radio is occupied. Copilot readbacks play before their effects are committed; automatic tuning waits for the exchange, and check-in waits for telemetry to confirm the new frequency. An urgent short-final go-around preempts normal playback. These are separate steps so a fast model response cannot skip a pilot transmission.

**ATC Auto Reply** handles the pending response once. It does not generate permission for unrelated movement or answer background traffic. Quick buttons are filtered by tuned service, flight phase and applicable state; completed start-up/pushback permissions and duplicate readback actions are not intended to clutter the list.

The debug Map offers **Flight path**, showing the engine’s expanded route and installed SID/STAR fixes. Hover waypoints for altitude/flight-level and speed limits. A filed connection into the STAR’s initial fix stays continuous; genuinely unresolved vectors or curves retain dashed connectors. These are schematic rather than direct clearances. Controller volumes containing the aircraft at its current altitude receive a dark translucent tint derived from their border colour, independently of the tuned frequency. The tuned controller keeps its stronger border. Taxi arrows and holding-point markers appear when instructions are issued; readback still controls operational permission.

## Copilot, cabin and ground crew

Crew conversation uses the LLM when enabled, including recognised service requests and replies after confirmed aircraft actions. The original message, recent conversation with that character and current facts are supplied together, so jokes and follow-up questions are not discarded. `speech/crew/actor_conversation.toml` supplies a bounded selection of editable style examples; `crew_response` in `prompts/prompts.toml` guides composition. Control names, targets and unconfirmed outcomes remain protected, with TOML fallbacks for failed or invalid replies. The fictional cabin retains passenger seating and an editable baseline mood (`passenger_mood` in `cabin/behaviour.toml`).

Role prompts in `prompts/prompts.toml` guide natural, distinct characters: friendly ramp and cabin crew, a capable conversational copilot, and concise professional ATC. Editable `crew_social_*` TOML rules handle greetings, thanks and farewells without turning them into preparation checks. With wording variety enabled, the LLM can phrase these social replies; TOML remains the fallback. Social examples are kept separate from operational examples, and a greeting combined with a control request does not discard that request.

**Talk** addresses the copilot. **Transmit** follows the active radio/interphone role. The ToLiss A320neo profile observes ATTN/CAB and MECH/INT selections for cabin and ground routing. Check the displayed conversation role before transmitting.

The ToLiss profile marks latching ATTN/MECH and CAB/INT commands in `toggle_commands`: pressing a selected button again restores ATC. Momentary commands in other profiles can retain their recipient. Selecting a VHF radio also restores ATC. Start-up/pushback permission alone does not prevent ground services from disconnecting external power or removing chocks: those actions require a valid, stationary ground position and simulator confirmation. They are refused while moving.

Crew requests can include up to eight explicit control actions, such as “remove chocks and disconnect external power.” The engine validates all targets against the loaded aircraft profile before sending a batch. Each action waits for simulator confirmation; a failure cancels the remaining actions. Unsupported or ambiguous clauses are clarified rather than partly executed and announced as complete.

Stock aircraft use standard X-Plane control mappings. Third-party TOMLs override controls where the aircraft uses its own datarefs or commands. **Settings → Aircraft controls** shows observed availability. A listed command does not prove that every aircraft exposes a reliable confirmation state: button-only mappings confirm a press, not a resulting switch position.

The copilot can change mapped cockpit settings, including heading, autopilot controls, gear and other supported systems. The available profile determines coverage. There is no unrestricted model-to-dataref access. Third-party aircraft can bypass standard simulator controls, so aircraft-specific live testing matters.

Checklists support two modes: the copilot reads a challenge and waits for your response, or you read an item and the copilot performs supported actions and responds. Checklist definitions belong with the aircraft; spoken pilot/copilot wording belongs in the speech library. The included checklists are examples and need operator/aircraft review before expansion.

Cabin conversation receives the live flight phase and ground state. With CABIN or ground services selected, both Talk and Transmit address that crew member. On the ATC channel, the separate Copilot button explicitly addresses the copilot. Greetings and refreshment requests do not prepare the cabin for landing. Arming and cross-check requests still depend on the aircraft's mapped indications; an unconfirmed slide indication must not be announced as armed. See [Aircraft controls and checklists](docs/AIRCRAFT_CONTROLS.md) for editing examples and limitations.

Landing preparation and passenger-watch requests are cabin services rather than switch operations. They can be combined with light commands, and service-only requests do not wait for an unrelated control acknowledgement. The LLM interprets unfamiliar paraphrases using the same service/action schema; TOML examples guide it without requiring an exact phrase match. Cabin conversation uses a bounded set of style examples instead of sending the entire role phrasebook.

Common spelling corrections and clause recognition are editable in `cabin/language.toml`. “Cross chek and arm dorrs” and “arm doors and cross-check” both request arming followed by verification. “Turn off cabin lights, can I have a coffee” uses the aircraft profile's off value and keeps the coffee request; no percentage is required. Negations and operational numbers are preserved. The LLM handles unfamiliar clear rephrasing through a compact, JSON-only classifier, with role and target validation before execution. Clarifications and combined confirmations use natural alternatives in the speech TOML files.

Requests such as “coffee, arm the doors and cross-check” keep the service request alongside the control actions. You can also combine a passenger update, “turn off cabin lights” and departure preparation in one request. The crew gives one combined result after the controls are acknowledged, including any unconfirmed items. Departure preparation acknowledges checks in progress rather than claiming they are complete. Recent requests, service counts and confirmed outcomes are retained separately: asking for something does not make it true. Repeat-coffee humour is limited to quiet, nonurgent cabin conversation.

The **simulated cabin** uses the imported passenger count when present, flight phase, seated/lavatory activity and turbulence preparation. It can discuss passengers resting or watching movies and follow up when the simulated cabin is ready. Unknown passenger counts remain unknown. This passenger activity is fictional; actual door/slide/seatbelt indications still come from the aircraft. Urgent mission incidents suppress cheerful all-clear reports and jokes.

Edit `cabin/behaviour.toml` to change service-cycle timing, lavatory activity, turbulence preparation time and the brightness threshold for resting reports. A personal copy in the engine configuration folder overrides the shipped file after an engine restart. Invalid policies are rejected with the file path and reason. Passenger activity follows simulator time when available, pauses with the flight and does not catch up after a large clock jump. Starting a new flight clears passenger activity while retaining your policy. Speech remains in the response TOML, including separate single-passenger and multiple-passenger lavatory reports.

## Missions and flight stories

Open **Missions** using the flag icon in the title bar. Choose a compatible flight story before taxi; loading a story leaves your IFR plan intact. The eight examples cover a medical priority landing, childbirth, rough air, parts transport, a nervous passenger, an island reunion, cabin smoke and an assisted arrival. Suggested routes are starting points for your own current flight plan, not prevalidated operational dispatches.

The mission editor has ordinary fields and an advanced TOML view. Events support flight phase, unpaused delay, probability, prior-choice conditions, passenger seat, alternative speech and captain responses. The journal records what occurred and which choices were made. An urgent mission permits an explicit emergency declaration even with ordinary emergency practice disabled; a cabin report does not declare one automatically. Full automatic diversion planning is still pending.

Files under `missions/` ship with the plugin. Personal copies are saved under the engine configuration folder's `missions/` directory and override matching IDs. `OPENATC_MISSIONS_DIR` adds an optional catalogue directory. Audio, video and image references with transcripts can be authored now, but media playback is not implemented and the example missions contain no media assets. See [Mission format and editor](missions/README.md).

## Emergencies and assistance

Declare an emergency or urgency explicitly, including when requesting a diversion, direct routing or descent. The request retains its emergency status instead of being reduced to an ordinary route change. Routine training penalties and discretionary traffic speed requests pause during a declared emergency.

Mission incidents can introduce a medical problem, childbirth or cabin smoke, with crew dialogue and captain choices. A cabin report does not declare an emergency for you or change your flight plan automatically. Urgent incidents suppress casual cabin humour and misleading all-clear reports. Full automatic diversion planning and mission-driven runway closures remain in development.

ATC can ask whether you need assistance after repeated deviations, and Ground can offer revised taxi routing from your stopped position. Existing IFR clearances remain active; accepting revised instructions still requires the appropriate readback.

## Traffic and performance

**Suppress X-Plane AI traffic** is enabled by default in Traffic settings. While connected, OpenATC requests control of native AI and reduces its active count to ownship; another traffic plugin retains priority. Generated traffic uses the supported TCAS override instead. Disable suppression to use the optional native AI amount controls. The plugin also mutes X-Plane's native COM radio audio while the companion engine is connected, preserving each panel's previous volume for disconnect or plugin disable; OpenATC speech uses its own audio output.

X-Plane's **Draw Parked Aircraft** population and aircraft embedded in scenery are separate from flying AI. Turn off Draw Parked Aircraft in X-Plane when replacing that population with a traffic plugin. OpenATC does not remove scenery objects or modify airport files. Generated parked aircraft and their later departures must share stand reservations and verified movement paths; that complete lifecycle is still under development. An amber map connector is an aid across an unmapped apron, not verified pavement or permission to cross a runway.

**Informal pilot conversations** are optional under Settings → Realism. Choose an identified aircraft on Traffic and type a message; weather/landing/crosswind buttons remain available separately. **Allow light humour** enables restrained friendly replies, and **Allow other pilots to call me** permits occasional ride/weather/arrival questions or greetings. These are simulated conversations, not multiplayer transmissions. Clearances, pending readbacks, emergencies, takeoff/landing, runway crossings and other critical situations take priority. Replies do not issue permissions or operate aircraft controls.

Trigger words, response priority, humour flags and fact conditions are editable in the `traffic_chat_*` entries of `speech/runtime/responses.toml`. `traffic/behaviour.toml` controls quiet phases, timing, range, history and pilot personality within validated limits. Personal overrides belong in the engine configuration's `traffic` folder and load on a new flight or engine restart. General free-text answers use the configured language model; without it, editable matching rules provide brief fallback replies.

Open **Traffic** from the visible page tabs to see simulator-reported aircraft. **Settings → Traffic** holds model downloads, aircraft amount and the separate **Hear simulated traffic pilot reports** and **Talk to simulated traffic pilots** switches. Select an identified aircraft to ask about weather, its landing or crosswind. Replies use recent simulator weather and observed landing history; missing information gets an honest unavailable response. These are simulated pilot conversations, not messages sent to multiplayer users.

**Set X-Plane AI traffic amount** adjusts the native active-aircraft count, up to 19 and limited by available simulator aircraft. It is off by default, leaves flight models enabled, yields to other traffic plugins and restores the previous count when disabled. Native AI flies its own simulator routes and consumes flight-model CPU. Start with a small count and compare X-Plane's performance readings. The SDK exposes aircraft ownership/count and AI control, but no documented interface for handing arbitrary per-aircraft IFR plans to native ATC. See [XPLMPlanes](https://developer.x-plane.com/sdk/XPLMPlanes/).

OpenATC accepts up to 63 simulator targets and checks up to 126 combined simulator/generated observations; pilot history updates at most once per second. It can hold taxiing aircraft for an observed conflict and resume when clear, protect individual runway crossings, and issue a short-final go-around for runway conflicts. Crossing completion and arrival Ground handoff include aircraft dimensions and the relevant holding line, so a centre point outside the runway does not immediately mean the tail is clear. Unknown aircraft use conservative dimensions. Limited automatic speed spacing uses the aircraft's editable indicated-speed envelope, including piston GA, turboprops, regional jets and airliners. It operates on established approaches outside eight miles and below10,000 feet, releases adjustments by five miles with the shipped policy, preserves published speed limits and prioritises a slower leader over a faster follower. It does not accelerate an aircraft towards a nearby leader. Pilots can answer **Unable**; it does not repeatedly offer the same rejected speed. This remains a limited speed workflow rather than complete mixed-traffic sequencing or holding/vector integration.

The `[spacing]` table and each aircraft's `spacing_speed` in `traffic/aircraft.toml` configure these thresholds and nominal speed ranges. Omit `spacing_speed` to disable automatic offers for that aircraft. Profile reloads publish a complete validated snapshot; controller checks retain the previous valid policy while a reload runs. Invalid initial overrides disable these offers rather than silently enabling fallback speeds. An approach clearance explicitly releases a prior assigned speed when it is not restated; published restrictions remain applicable.

**Wake protection** now uses FAA consolidated wake categories for the represented US airports and supported aircraft. Observed departures can produce a **Hold for wake turbulence** instruction, with different intervals for full-length, intersection and opposite-direction departures and qualifying close parallel runways. Timing uses simulator flight time, stops while paused and does not expire instantly after a clock jump. Starting the interval at observed liftoff is conservative; no pilot waiver is assumed. Approach checks consider the aircraft pair, closing speed, parallel runway geometry and displaced landing thresholds. Insufficient spacing at the preceding aircraft's threshold passage remains a conflict after it touches down or disappears from the feed, and can require a go-around. These checks follow the [FAA departure rules](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap3_section_9.html) and [approach wake table](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap5_section_5.html). Intersecting flight paths, preceding low/missed approaches and complete worldwide separation are still unfinished.

**Download and install Bluebell** downloads the publisher's 12 DDS archives and installs them for you. **Settings → Traffic** shows the current archive, downloaded bytes and extraction progress, with Cancel and Retry controls. Completed archives are reused after an interrupted attempt; an existing installation is replaced only after all archives have extracted successfully. Allow about 6 GB of free disk space for downloads and installation, or more when replacing an existing pack. Models live in `Resources/plugins/OpenATC/Resources/CSL/Bluebell` under your selected X-Plane folder. The companion engine performs the network and disk work off the simulator thread.

Downloads come directly from the [Bluebell publisher's public distribution](https://github.com/oktal3700/bluebell). Textures retain their original licences and are not bundled in OpenATC releases. A publisher quota or network failure appears in Settings → Traffic and can be retried. **Scan installed model catalogue** indexes available aircraft types, operators and liveries on a background worker. Matching prefers the requested type/operator, then an unmarked model of the same type; it does not silently substitute a competing airline. Incomplete models and unsupported texture overrides are reported in the catalogue diagnostics. Native AI uses X-Plane aircraft, not CSL objects; lightweight CSL rendering and randomly generated flights remain unfinished. Installing Bluebell alone does not create traffic.

**Traffic advisories** use observed aircraft position, heading, speed and altitude to predict encounters up to two minutes ahead. Calls include clock position, range, direction, type when known and indicated altitude. Several encounters are tracked independently, so an earlier call cannot hide a more urgent aircraft. Urgent alerts can interrupt ordinary speech while preserving the outstanding clearance; routine calls wait their turn. The ATC page offers **Traffic in sight** and **Negative contact** for the most recently called traffic on the tuned frequency. Follow-up calls distinguish traffic that is no longer a factor from traffic that is no longer observed. Reporting traffic in sight does not grant visual separation or acknowledge another clearance. These calls remain available with background traffic chatter off. Trigger thresholds are conservative simulator heuristics, not a complete regional separation system or a replacement for TCAS.

Copilot playback and acceptance are separate steps. Repeated preparation requests reuse the pending transmission, and the UI identifies the exact transmission when reporting playback complete. A delayed completion cannot acknowledge a replacement reply or one prepared on another frequency. Live audible timing still needs simulator acceptance.

**Background CPU workers** controls the engine's bounded scenery, navigation and model-indexing pool on Linux, Windows and macOS. Zero selects automatic: half the available logical CPU threads, at least one and at most four. Set one to limit background concurrency, or choose a higher limit up to sixteen, capped by available parallelism. This does not change traffic density. The pool starts on demand, its waiting queue holds at most twice the worker count, and an overloaded request returns a retryable error instead of blocking the simulator. Changes apply when the companion engine restarts. `/health` reports the active `cpuWorkers` value. SDK calls remain on the simulator thread. The experimental generated-traffic simulation also uses this pool, with at most one simulation job outstanding and complete snapshots published to the plugin. The preparation worker retains runway-compatible terminal candidates with published recovery legs and known enroute MORA floors; `terminal_candidate_limit` in `traffic/operations.toml` bounds retention. Full generated flight operations remain under development.

**Airline, hub and aircraft preferences** are editable in `traffic/airlines.toml`, `traffic/hubs.toml` and `traffic/aircraft.toml`. The starter set has 29 operating/private fleet profiles, 50 airport preferences and 47 aircraft envelopes spanning piston aircraft through freighters. Weights are approximate simulation preferences, not market shares or real schedules. The selector checks both endpoints, home-region affinity, runway/parking capacity, range and compatible installed CSL models. Traffic can reload and validate personal overrides, retaining the last valid set after an invalid edit. These are data/selection foundations; the generated moving fleet remains unfinished. See [profile editing and sources](traffic/README.md).

## Maps, stations and simulator guidance

| Page | What it shows |
| --- | --- |
| ATC | Conversation, current clearance, text entry, role controls and eligible quick requests. Click a message for selectable/copyable text. |
| Planning | Editable/imported flight plan, route, procedures and flight identity. |
| Channels | Automatically shows the airport's stations on the ground and nearby receivable stations in flight, with frequencies, duties, distance/coverage and tuned state. Click to tune COM1; there is no airport filter to maintain. |
| Taxi | Airport surface geometry, stands, pending/approved taxi route, aircraft and clearance limit. |
| Arrival | Geometric planned/current descent profiles, installed runway/approach references and sampled terrain where available. |
| Airports | Airport search, runways, services, navaids, procedures, stands and 2D/3D geometry. |
| Traffic | Native AI amount, reported aircraft, optional traffic speech and pilot conversation controls. |
| Missions | Compatible stories, form/TOML editor, captain choices and flight journal. |
| Map, developer mode | Aircraft-following controller boundaries, altitude bands, airports and current assignment for handoff diagnosis. |

Channels identifies a ground airport using enabled scenery boundaries, pavement and runway footprints before falling back to reference-point proximity. This avoids choosing a nearby heliport or small airfield simply because its reference point is closer. It is scenery containment, not a legal airport-property survey. The station list updates as airport and radio data arrive; there is no airport filter to maintain.

Stations come from enabled scenery and installed ATC/navdata in priority order, including 8.33-kHz channels. Published controller definitions can replace airport Tower entries; Ground and Delivery remain airport services. Approach/Center coverage and receivability are separate concepts. Terrain shielding, actual transmitter sites and full remote-radio coverage are not modelled.

Click an underlined frequency in the conversation to tune COM1 through the same aircraft-control path as Channels. The radio must be powered. Table headers support ascending/descending sorting, and name/identifier filters help narrow airport and stand searches.

Taxi map arrows and simulator-ground arrows are separate settings. Only acknowledged routes produce ground arrows, and unverified stand connectors are not drawn on simulator ground. Guidance stops at the current clearance limit. Terrain probe misses, water and unsafe discontinuities are skipped.

The optional **Holding point marker** is off by default. It draws a translucent amber circle on the map and simulator ground, set back on the approach side of available hold-line geometry. It disappears when the relevant crossing or takeoff permission permits entry. It is plugin guidance, not a replacement for painted markings or native airport lights. Existing scenery runway guard lights are preserved; individual native stop-bar control has not been established.

The Arrival profile is a geometric aid, not a complete STAR/VNAV solver. Missing terrain stays unknown. The debug Map and Arrival profile have different purposes; dense airport labels can be hidden. Use the Map to compare the aircraft position and altitude with loaded sector coverage when diagnosing a missing handoff. Taxi and debug Map each have an independent **Show traffic** checkbox, enabled initially. Reported traffic appears green on the ground and orange airborne, with callsign labels and hover details for name, type, altitude, speed and known COM1/COM2 channels. Native TCAS does not supply tuned radio channels, so those fields show Unavailable unless a traffic provider reports them. Hiding markers does not disable traffic or radio. These layers display reported telemetry; they do not create traffic.

ATIS uses simulator weather sampled at the station's airport and repeats until retuning or radio/reception changes. While OpenATC handles the tuned ATIS, it suppresses X-Plane's built-in broadcast to prevent overlapping voices. It restores the previous native ATIS setting when you tune away, disconnect the engine or disable the plugin. With COM2 enabled it can monitor ATIS on the second radio while you browse other pages; controller instructions take priority over ATIS playback. Settings → Voices provides a dedicated ATIS voice, steady reading speed and optional electronic effect. That effect approximates an older automated broadcast, rather than removing the synthesizer's pitch variation. Online METAR lookup is a separate planning feature. Distant unavailable observations are not replaced by invented weather. Current ATIS does not provide complete worldwide regional diction or NOTAM coverage.

## Settings and realism

ATIS broadcasts also appear in the conversation log with an **ATIS** badge. Spoken communications stay in character; internal software notices are not voiced. Runway selection uses the airport's installed runway geometry and observed wind rather than accepting a tailwind runway from the flight plan. A planned runway only breaks ties between equally suitable directions. Runway changes advance the information letter. Scenery loading runs on a bounded worker and runway geometry is cached. ATIS remains informational, not a takeoff or landing clearance.

New messages from every speaker follow the conversation only while you are at the bottom. Scroll up to read history without incoming pilot, controller, crew or ATIS entries moving your position; return to the bottom to resume following.

Page navigation lives in the title bar: ATC, Radios, Flight plan, Taxi, Arrival, Airports, Settings, Traffic and Missions. The Map icon appears in developer mode. Hover an icon for its name; cyan marks the current page. At narrow widths the navigation icons move together onto a second header row, keeping the title and window controls accessible. The right side holds the debug flight jump (developer mode), New flight, Keep expanded, compact ATC view and pop-out controls.

Clicking **Communications only** keeps the ATC window compact until you expand it, choose another page or start push-to-talk. Hover still expands a window that compacted automatically. The transcript keeps following the latest message across size changes; if you scroll up to read earlier traffic, it preserves that reading position instead.

Settings tabs wrap in narrow windows so every section remains visible. **General** shows the engine connection, current AI/speech addresses, SimBrief ID and scenery folder. It can optionally apply one server address to all three services. **Copilot** holds readbacks, automatic tuning/check-in and the copilot talk button. Advanced service addresses, intercom overrides and CPU limits stay with their relevant sections. Changes save automatically; Save now also submits them immediately.

Dispatch fields adapt to window width and text scale. You can edit a flight plan while the engine is disconnected; SimBrief import becomes available when connected. The Airports page offers departure/destination shortcuts before an airport has been loaded. The debug map keeps controller and airport labels apart and groups overlapping controller frequencies in its hover details.

### Appearance

**Settings → Appearance** contains Light/Dark theme, text scale, background transparency and window behaviour. Dark is the default. **Background transparency** ranges from 0% (solid) to 80%; text and controls retain their contrast. Collapsed-window opacity is separate. The background uses ordinary ImGui blending without scene capture or post-processing.

### Flight realism

Choose **Relaxed**, **Standard** or **Strict**, then review individual settings rather than assuming a preset is appropriate for every aircraft or test. Relevant controls include:

- **Readback and constraint checks:** keep clearances pending until acknowledged, and apply selected published restriction checks.
- **Deviation monitoring:** track assigned altitude/heading/speed where implemented. Repeated unresolved corrections can prompt ATC to ask whether you can comply or need assistance. IFR service and existing permissions are not automatically cancelled after a warning count. The old `terminateAfterWarnings` settings key is retained for existing files and now controls that assistance prompt.
- **Transponder monitoring:** compare the simulator's selected code with the acknowledged IFR clearance, with a 30-second adjustment period and at least a minute between reminders. Airborne checks also look for altitude-reporting mode. Missing or invalid observations and emergency codes are excluded from routine code corrections.
- **Runway permission warnings:** identify runway entry, takeoff and landing without the corresponding permission. Crossing permission applies only to the named physical runway. The monitor does not issue a late takeoff-abort instruction during a high-speed roll; an airborne departure without clearance receives an intentions query. Landing reminders wait until rollout slows.
- **Taxi assistance:** monitoring starts after the taxi readback and confirmed movement: at least five metres of displacement and two telemetry samples at one knot or more. Sitting at the stand, pushback before taxi readback and small speed fluctuations do not trigger an off-route warning. On an unmapped apron, the initial connector from the observed starting position to the first route point uses the same 60-metre corridor tolerance; moving toward the network does not trigger assistance. After joining the mapped route, that exemption ends. Moving away from the connector or spending 15 seconds more than 60 metres from the approved route prompts the issuing controller to offer assistance when the radio is free. A new taxi clearance starts fresh movement monitoring. Runway-permission checks remain independent.
- Answer the current taxi-assistance question with **yes**, **affirmative**, **no** or **negative**. This works without AI classification. When stopped, an accepted offer recalculates a route to the same runway or stand, updates the map and requires a new readback, retaining intervening runway holds. Missing safe geometry results in a hold-and-confirm-position response. These are revised route instructions, not continuous step-by-step progressive taxi. A short answer cannot acknowledge an unrelated clearance or a question from another frequency. Invalid AI classification JSON is rejected and ATC asks for clarification rather than executing model-supplied actions.
- **Copilot replies and tuning:** independently control automatic radio assistance. Keep both enabled if you want the full acknowledge/tune/check-in sequence.
- **AI wording variety:** off uses validated TOML alternatives; on allows natural model wording that must preserve operational facts.

Initial IFR clearances retain the full filed route internally but speak abbreviated routing: the assigned departure/transition, then as filed. The copilot repeats that abbreviation, altitude, squawk and runway, using the destination airport name when available. Full-route readbacks remain accepted; amendments still require the changed routing. Clearance and readback wording lives in `speech/runtime/responses.toml`.
- **English-language voices only:** limits controller selection by language while allowing English accents.
- **Pilot playback:** hear synthesized own transmissions if enabled.
- **Guidance visibility:** map arrows, ground arrows and holding-point marker are independent.
- **Developer mode:** exposes the sector Map, detailed errors and controlled surface test jumps. It defaults off for new settings files; existing explicit choices are preserved. Enable it under Appearance → Developer tools.

Optional live-session traffic speech is tied to identified simulator aircraft and observed events. Scripted demo chatter stays in demo sessions. Runway protection can only see traffic provided through the simulator interface; aircraft missing from that interface remain invisible to this check. Complete traffic management, wake sequencing and separation are still in development.

**Allow ATC to offer en-route shortcuts** is an optional Realism setting, off by default, with a minimum interval of 2–60 minutes. Offers require suitable downstream fixes, known terrain clearance, no skipped published restrictions and enough distance left for descent. Use **Accept direct…** or **Keep planned route** in the ATC page. Acceptance requests a new clearance and readback; copilot mode does not accept optional changes for you. Simulated runway closures and broader ATC scenario amendments are still planned. Safety instructions and required handoffs do not depend on this switch.

The three additional monitoring switches default off for existing configurations. Selecting Standard or Strict enables them; Relaxed disables them. They can also be changed individually. Required crossing, takeoff and landing permissions remain in force with reminders off.

Tower keeps checking the runway after issuing landing clearance. A newly observed conflict can trigger a go-around even with a readback outstanding. A missed approach does not immediately become another approach clearance merely because the aircraft is near the destination; request an approach again. Routine exit instructions wait until the aircraft has slowed below 40 knots on rollout. Published missed-approach leg guidance is not yet complete.

**Follow-me car** is an optional aid under Realism → Taxi guidance and on the Taxi page. It uses X-Plane's installed follow-me vehicle, appears only for an acknowledged departure or arrival taxi route, and waits before each clearance limit. It cannot authorize a runway crossing. Pending readback or pause stops its movement; reaching parking, taking off or ending the flight removes it. If the route is too far from the aircraft or the scenery vehicle is unavailable, it is not drawn. It is off by default. Live appearance and placement still need simulator testing.

Simulator work is bounded: telemetry is sampled twice a second and coalesced, so an outstanding request cannot build up a queue of old positions. HTTP and speech run on workers; route planning and compliance run in the engine. The panel shares immutable snapshots instead of copying the entire route and conversation each frame, and response parsing stays outside the UI lock. Static airport bounds, taxi node lookups and ILS beams are cached when the airport loads. Ground guidance displays at most 64 nearby arrows and reuses object instances between updates. The follow-me car uses one scenery instance, no aircraft flight model or AI traffic slot, and updates at up to 20 Hz with one terrain probe per movement update. It caches its route and stops at the acknowledged limit. The arrival terrain sampler is capped at 25 probes per half-second update. These are workload limits, not an FPS guarantee; measure performance with your scenery, aircraft and traffic settings using X-Plane's Plugin Admin performance page.

The header's **New flight** icon opens a confirmation inside the plugin window. Starting a new flight clears the plan, conversation, permissions, guidance and pending crew actions. Settings, controller voices and actual aircraft controls remain. Reset errors expose retry controls. Use this to begin another test instead of unloading a live plugin.

The developer fast-forward icon offers controlled surface jumps: an acknowledged route is required to move to the next hold, and takeoff clearance is required to move to the departure runway start. It does not grant permissions or jump into airborne phases. Third-party aircraft reposition behaviour still needs simulator testing.

## Customise speech and aircraft

| File or directory | Purpose |
| --- | --- |
| `aircraft/*.toml` | Aircraft matching, supported commands/datarefs and checklist definitions. |
| `regions.toml` | Regional unit and procedure references. |
| `radio-stations.toml` | Optional combined-duty overrides for existing published stations. |
| `intents.toml` | Request names, aliases and examples. |
| `traffic/behaviour.toml` | Pilot conversation conditions, timing, probability, range, history and personality. |
| `cabin/behaviour.toml` | Simulated passenger activity, preparation timing and observed-control freshness. |
| `speech/runtime/responses.toml` | Live controller, pilot, ATIS, readback and crew response templates. |
| `speech/common` and `speech/<region>` | IFR, VFR and shared phrase examples. |
| `speech/crew` | Copilot, cabin and ground-service examples. |
| `missions/*.toml` | Editable flight stories, phase/probability triggers, dialogue, choices and optional future media references. |
| `prompts/prompts.toml` | Editable ATC, classifier, crew-action and conversation role instructions. Legacy personal `.txt` overrides remain supported when that directory has no TOML catalogue. |
| `models.toml`, `models-kitten.toml` | Built-in model download manifests, embedded when the server is compiled. |

All spoken phrase alternatives belong in the speech library. Aircraft profiles describe controls and checklist data; they should not contain a separate collection of pilot/copilot response scripts. See [Speech editing](speech/README.md), [Runtime templates](speech/runtime/README.md) and [Situation index](speech/INDEX.md).

Alternatives must retain required placeholders and operational facts. With wording variety enabled, matching phrases become examples for the LLM, together with its role and context. Repetition can still occur for standard phraseology. The engine remembers phrase selections and protects route, altitude, runway, frequency and other cleared values; a rewrite changing protected facts is discarded.

Displayed identifiers remain compact. Speech expands controller abbreviations such as GND to Ground, uses known airport names, spells registration callsigns phonetically, and applies aviation number pronunciation. IFR/VFR/ILS remain spoken letters. Missing airport-name data can still require an identifier fallback.

Adding TOML text does not implement a new operational permission or simulator action. Add the associated request handling, validation and monitoring before connecting a new scenario to live operations. Regional examples are a starting point and require local procedure review.

`intents.toml` now supplies the request-button labels/order, matching aliases and slot-recognition patterns. The engine and plugin load installed catalogues; a personal engine `intents.toml` can override them. Older catalogues retain bundled missing definitions and button order. Invalid grammar is rejected before activation. Mission phase/probability/choice logic and conversation equality/list/numeric-range conditions are declarative TOML; clearance authority and simulator safety remain validated by Rust.

Validate edited speech before flying:

```sh
target/release/open-atc-engine --check-speech speech
cargo test --locked -p openatc-core --test shipped_config --test speech_library
cargo xtask test-speech target/release/open-atc-engine speech
```

Restart the companion engine after changing speech, prompts or duties. Model manifests are compiled into the AI server; editing an installed copy documents the model set but does not change the executable's download plan. Rebuild to change the built-in manifests.

## Builds and packages

The **Native platform builds** workflow runs on version tags beginning with `v`, or manually through GitHub Actions. Pushes and pull requests do not automatically start the expensive platform matrix. A manual run accepts a version such as `0.0.1-alpha.1` and lets you select one platform and one product.

| Platform | Plugin product | AI-server product |
| --- | --- | --- |
| Linux x64 | ZIP/payload installer plus DEB, RPM, AppImage and Flatpak bundles | Separate DEB, RPM, AppImage, Flatpak and archive |
| Windows x64 | Complete plugin payload and installer | AI runtime and installer |
| macOS Intel / Apple Silicon | Platform plugin payload and DMG | Platform AI runtime and DMG |

The Linux plugin packages install an **installer application**: use it to select your X-Plane folder. They do not assume that your simulator lives in a system package directory. Flatpak also needs access to the chosen simulator folder. Choose one installation format; installing every format is unnecessary.

Manual runs publish downloadable **Artifacts** on the workflow run's page. Version-tag runs collect package outputs and attach them to a GitHub **Release**. These are release downloads, not GitHub Packages container images. Pre-release suffixes mark the release as a pre-release. The raw native artifacts are useful for diagnosis; normal users should use the complete packaged runtime.

Windows builds use MSVC and the Visual Studio linker. macOS builds run natively, including an ONNX Runtime source build for Intel Macs. That dependency is cached and its unit-test compilation is disabled, but a clean build can still take a long time. Linux cross-checks cannot replace loading the Windows or macOS plugin in its native simulator.

The workflows describe intended outputs; see the validation record for what has actually been run. New GPU-enabled Windows/macOS packaging needs native CI acceptance after this change. No hosted build is started by editing the workflow locally.

## Development and validation

### Source layout

| Directory | Responsibility |
| --- | --- |
| `crates/core` | Flight state, controller permissions, geometry, navigation and data readers. |
| `crates/engine` | Request orchestration, radio/crew workflows, surface and airborne monitoring, provider calls. |
| `crates/plugin` | X-Plane SDK integration, telemetry, aircraft controls and rendering host. |
| `crates/ui` | Shared interface and client state. |
| `crates/desktop` | Standalone UI preview. |
| `crates/server` | Local AI HTTP service, LLM inference, TTS worker and STT process ownership. |
| `crates/tts`, `crates/stt` | Speech synthesis/recognition and audio formats. |
| `crates/ai-core` | Model manifests, downloads and integrity records. |
| `crates/audio`, `crates/http`, `crates/platform`, `crates/settings` | Shared audio, transport, operating-system support and settings. |
| `xtask` | Rust build, install, package and integration-test commands. |
| `tests/fixtures`, `xtask/fixtures` | Synthetic test data; not production airport overrides. |

First-party implementation is Rust. Native dependencies still include C/C++ libraries such as X-Plane bindings, ImGui, audio and inference runtimes. Python is not needed to run OpenATC. Some upstream native builds, especially Intel Mac ONNX Runtime, use Python during compilation.

Install current stable Rust, a native compiler, Clang/libclang, CMake and pkg-config as appropriate for your operating system. Optional accelerated builds need their SDKs; see [AI server building](docs/AI_SERVER.md). The old top-level C++ implementation and Python scripts have been removed.

```sh
cargo fmt --all -- --check
cargo test --locked -p openatc-core -p openatc-engine -p openatc-ui -p openatc-plugin -p xtask
cargo test --locked -p openatc-ai -p openatc-stt -p openatc-tts
cargo build --locked --release -p openatc-plugin -p openatc-engine
cargo xtask test-radio target/release/open-atc-engine speech
cargo xtask test-crew target/release/open-atc-engine speech
cargo xtask test-speech target/release/open-atc-engine speech
cargo xtask test-flight target/release/open-atc-engine speech
cargo xtask audit-linux target/release/libopenatc_plugin.so target/release/open-atc-engine
```

Default workspace builds cover the plugin stack. Select optional AI/desktop targets explicitly; `--workspace` includes their native dependencies. On Windows, use the `.exe` engine path in integration commands. The Linux dependency audit is platform-specific.

Integration suites start a real companion engine against temporary scenery/settings and a fake provider. They verify workflow contracts without requiring models or mutating your simulator. Model/audio tests and installed-scenery tests provide separate evidence. None is a substitute for flying the complete route with live audio and the chosen aircraft.

Generated packages, model downloads, build directories, logs, personal environment files and local assistant files are ignored. Product prompts, phrasebooks, model manifests, Cargo files and CI workflows belong in the project. Ignore rules cannot remove an already-tracked file; review your own commit contents.

See [Architecture](docs/ARCHITECTURE.md), [Rust workspace](docs/RUST_WORKSPACE.md), [Validation](docs/VALIDATION.md), [Changelog](docs/CHANGELOG.md) and [Developer commands](xtask/README.md). Contributions are especially useful for native Windows/macOS testing, different airport layouts, aircraft profiles, pronunciation and reviewed regional procedures.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| No stations or the wrong airport | Radios shows the occupied airport's services on the ground and nearby stations in the air. Check the X-Plane folder in Settings → General, radio power and the position/loading message. Export the position if an airport footprint is wrong. |
| Simulator telemetry rejected | Copy the field and value from the error or developer log. The engine can remain connected while simulator data is rejected. A valid new sample clears the error; rejected data does not replace the last accepted flight state. |
| Cannot find speech or voice settings | Open Settings → STT / TTS, Voices or Audio. Tabs wrap in narrow windows. General shows the currently saved provider addresses; editing becomes available after saved settings load. |
| Controller rejects a request | Confirm phase, tuned service, airport duties and any pending readback. A recognisable sentence can still be unavailable in the current state. |
| Taxi stops at a runway | Read back the current route, then wait for or request the applicable crossing. A destination runway does not authorize crossing another runway. |
| No crossing/takeoff/landing clearance | Check runway/final traffic and whether the simulator supplied traffic data. Unknown traffic is not treated as an empty runway. |
| Copilot does not tune/check in | Enable the relevant copilot settings, let queued speech finish, and verify COM1 actually changes in telemetry. |
| Own text is silent | Enable Speak my transmitted requests and check pilot volume/output. |
| TTS failed | Inspect `engine.log` and AI-server output for the exact text, voice and error. Refresh voices after changing providers. Keep eSpeak data and shared libraries with the runtime. |
| GPU not used | Check `/health`, startup logs, build features and drivers. CUDA speech additionally needs compatible CUDA/cuDNN libraries. Remove force-CPU flags only if GPU use is intended. |
| LAN connection fails | Use the server's LAN address, bind with `--host 0.0.0.0`, and check the firewall. URLs should not end in `/v1`. |
| Aircraft action was not confirmed | Inspect the mapped control's availability and live indication. A custom aircraft may require a corrected TOML mapping. |
| Copy/paste fails | Check the native clipboard and display session. Debug conversation export also provides a file fallback. |
| Need to restart the flight | Use New flight in the header; inspect any reset error rather than repeatedly unloading the plugin. |

`test-flight` runs four complete IFR flights through the real HTTP engine using simulated aircraft telemetry and synthetic scenery: separate Delivery/Ground/Tower, Ground plus Tower, combined Tower duties, and separate controllers using COM2 while COM1 remains unchanged. It checks sector handoffs, copilot exchanges, proactive descent before a crossing restriction becomes infeasible, arrival and departure runway crossings, traffic holds, landing, runway exit and taxi to the assigned gate. Reports and transcripts are written to `packages/flight-validation/report.json`. This suite also runs in platform build jobs. It does not prove live audio, third-party aircraft controls or simulator rendering.

When reporting a flight issue, include the conversation export, aircraft/profile, installed scenery/navdata, phase, COM frequency, position and the action you expected. For audio issues also include the provider, voice, relevant server error and whether CPU or GPU was selected. Remove credentials and unrelated personal information before sharing logs.

COM1 and COM2 use independent speech playback so a broadcast on one radio does not block the other. Each ATC log shows its own radio traffic; crew interphone messages remain in the primary conversation. ATIS repeats audibly without duplicating the same information bulletin in the log.

Click an underlined squawk in the clearance summary or conversation to select that code. ToLiss AUTO/ON switches to ON while the transponder/TCAS mode selector stays unchanged. Bindings are editable in `aircraft/transponder.toml`. Nice requires an active transponder before pushback or taxi; selecting a code alone does not complete that procedure.

Default aircraft use the standard X-Plane squawk dataref. Aircraft-specific digit bindings apply only when the complete custom control group is available; setting a code preserves the default aircraft’s transponder mode. Custom controls and the standard fallback are editable in `aircraft/transponder.toml`.

The floating plugin window remembers its position, expanded size and ATC transcript divider in settings. Compact mode preserves the expanded layout, and restored windows stay within the current screen bounds.

Crew requests can combine controls with a friendly remark. Editable language rules separate recognised social clauses from actions while preserving the complete request for the actor’s reply; unknown controls and negations still require clarification.

Generated-traffic flight installation now retains its controller plan atomically. Joint airborne scheduling is tested across departure, en-route, arrival and approach restrictions; connection to the production full-flight factory remains unfinished, so this is not yet a complete generated traffic lifecycle.

All voice actors share the controller engine’s LLM transport and reply-decoding pipeline. Editable TOML role instructions, shared framing in `prompts/interaction.toml`, and example phrases teach tone and behaviour; live facts and conversation history guide fresh replies. Aircraft permissions and confirmed action outcomes remain enforced by Rust.
