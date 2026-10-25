# Validation

## Gate sequence, runway lock and traffic lifecycle — 24 October 2026

Generated gate departures now request IFR clearance from Delivery (falling back to
Ground), read it back, then request engine start and pushback on Ground, then request
taxi. The taxi order is attached to the first taxi-out segment, so it cannot be granted
before pushback. The IFR exchange uses the destination, departure runway, a published
departure hint, an initial altitude and a deterministic squawk.

Generated traffic no longer switches to the reciprocal runway when the requested mix is
better there. The ATIS runway owns the scene; same-flow alternates are only tried when
the selected runway releases no flight. Ground reservations ignore only aircraft behind
on the same assigned route and same movement direction, so a follower can be held without
blocking its leader.

Traffic settings gain a collapsible lifecycle section: taxi target 4–6 (rolled once per
session), and circuit retention radii of 30 NM for airliners, 20 NM for turboprops and
10 NM for GA. The opening cast seeds four to six mid-taxi aircraft plus the hold and
queue where departures allow. Landed arrivals taxi in, pause, despawn and recycle the
slot; generated traffic no longer leaves parked rows. Arrival exit selection favours the
forward exit with the shortest total roll plus taxi distance to a compatible stand.

Tests: generated suite 138 passed, 23 ignored, with only the two pre-existing scenery
failures (`autogen_top_covers_an_interior_ridge_above_its_anchor` and
`retained_tile_limit_evicts_old_stacks_but_never_live_borrows`). UI 48 passed/1 ignored,
settings 13 passed. Installed Nice scene and the bidirectional Nice–Milan lifecycle pass.
The gate Delivery sequence has a focused regression. SDK 440 was reviewed: instance
positions still update from a flight loop, TCAS targets still require unique 24-bit Mode S
identities and valid SSR modes, and no new traffic-injection API replaces the current
path.

## Departure queue and clearance scope — 24 October 2026

Completed taxi departures receive their Ground-to-departure-controller handoff before
new apron or taxi requests. The controller comes from the prepared line-up clearance,
including combined services. Readbacks still precede retuning, onward installation and
movement permission. Summary diagnostics identify controller frequencies, ready calls,
onward installation and repeated holding reasons without flooding the log.

Active-flight acknowledgements now grant only their explicit clearance scope; they no
longer approve unrelated later non-crossing segments. A stationary player no longer
bypasses airborne conflict revocation for other aircraft. Clock discontinuities cancel
old airborne grants before another controller exchange can begin.

The flight suite passes 25 tests, including a new holding-point versus gate-call
regression that reaches takeoff through both Tower and a combined departure service.
Existing runway-occupied missed-approach, crossing readback, wake, airborne conflict,
clearance-scope and clock-reset checks pass. The wider generated suite reports
143 passed, 25 ignored and two pre-existing scenery failures: interior-ridge autogen
coverage and retained-tile eviction. These failures remain open.

The installed LFMN–LIMC two-flight lifecycle completes gate-to-gate after the fix.
Its fixture refreshes unchanged synthetic weather observations during installed-scenery
planning, as production receives fresh simulator observations; weather freshness checks
themselves remain enforced. The serial plugin SDK suite passes 48 tests with 2 ignored.

The official 12.4.4 release notes link the SDK 440 beta download and new API reference.
The new instance API still permits position updates from flight-loop callbacks; the
traffic renderer uses that path. See the
[12.4.4 release notes](https://www.x-plane.com/kb/x-plane-12-4-4-release-notes/) and
[instance manipulation reference](https://xpsdk-api.docs.staging.x-plane.com/docs-xplm/XPLMInstance/instance_manipulation.html).
This controller repair does not require adopting a new SDK function.

Traffic counts, worker settings, procedure planning, approach sequencing and recycling
are preserved. Native Nice departure behavior and the reported Taxi-page visibility
problem require simulator verification; no native acceptance is claimed here.


## Selected-runway fleet eligibility — 23 October 2026

Generated fleet selection now uses the runway selected after the weather/flow
check, rather than the longest runway anywhere at each airport. A heavier aircraft
can no longer win the weighted fleet choice solely because an unused parallel
runway is longer. GA departures retain the capacity of their separately prepared
runway candidates. Simulator airport identifiers are mapped to the configured
world identifiers for the same selection; controller and movement airport records
remain the installed ones.

A weighted UPS fleet regression verifies both runway ends, unused-long-runway
exclusion, selection of the suitable smaller type, unknown-runway refusal and
published airport-code mapping. The generated source suite passes 144 tests with
25 installed-data fixtures skipped. The full installed mixed rerun is pending;
complete cargo/GA and native simulator acceptance remain open.


## Cargo route coverage — 22 October 2026

A partial European route sample was excluding FedEx and UPS from every unlisted
city pair, including their configured US hubs. Operator profiles now have an
explicit `route_policy`: the default `listed` retains the existing catalogue
restriction; `hub_spokes` also permits simulated connections through that
operator's positively weighted configured hubs. Only FedEx and UPS opt in.
Unrelated non-hub pairs remain excluded. Geography, fleet/range, installed model,
stand, procedure, weather and full protection checks remain mandatory.

The spoke policy models a network; it does not label an unlisted pair as a
published flight or schedule. Existing listed pairs retain their weighting.
FedEx describes its [Memphis World Hub](https://newsroom.fedex.com/newsroom/global-english/fedex-unveils-new-automated-sorting-facility-at-memphis-world-hub)
as central to its network, and UPS identifies [Louisville Worldport](https://www.jobs-ups.com/us/en/ups-worldport)
as the center of its global air network. These sources support the hub model,
not an assertion about a specific direct city-pair service.

Profile regressions pass 10 tests, including hub direction symmetry, non-hub and
listed-policy refusals, service-area restrictions, range, model availability and
stand-policy rejection. Both shipped-configuration tests pass. Generated source
checks pass 139 tests, with 24 installed-data fixtures skipped. The locked Linux
plugin/engine release build and core library Clippy pass; Clippy retains warnings.
These changes have not been installed.

The full Memphis–Louisville mixed fixture still fails before movement acceptance.
Cargo now receives two slots in each direction and reaches actual departure and
arrival planning; previously all four requested cargo slots were excluded. The
updated run retains seven nominal flights from the two primary origins and zero
production releases. It still fails the third-origin requirement. Cargo landing,
taxi connectivity, vertical/turn geometry and conservative scenery refusals remain
open. This is no claim of complete cargo, mixed traffic or native acceptance.


## Traffic, setup and arrival controls — 22 October 2026

The Linux plugin and companion engine were built and installed with X-Plane closed.
Installed binaries matched the release build, and all 209 installed resources matched
source. Speech validation passed for 614 situations and 2,307 phrases. Simulator
visuals, audio, frame times and the Developer Re-setup button still need native testing.

| Check | Result |
| --- | --- |
| Engine unit tests | 303 passed; 34 installed-data tests skipped |
| Core traffic tests | 102 passed; 5 installed-data tests skipped |
| Scenery tests, included in the engine suite | 36 passed; 4 installed-data tests skipped |
| Settings | 13 passed |
| Telemetry readers | 3 passed |
| UI | 46 passed; 1 native fixture skipped |
| Engine Clippy, all targets | Passed with warnings |
| Locked Linux engine/plugin release build | Passed |

Focused checks cover arrival closure and braking, final-speed transitions, runway
threshold timing, following traffic, published speed restrictions, readback before
speed changes and smooth generated-aircraft deceleration. Holding tests cover the
setting, instruction/readback, onward clearance, route/runway identity, inbound-fix
exit, pause/time changes and missing weather or protection data. Wake checks include
Australian Heavy-to-Heavy departures, for which a timer cannot replace observed
distance. Conversation checks cover eligibility, matching frequency, percentages,
cooldown and remembering unsuccessful rolls. Existing VFR/CTAF tests also pass.

### Production flight replay

The installed-scenery fixture prepared six nominal flights with origins at Nice,
Milan and Toulouse. One passed production scenery checks and completed taxi-out,
takeoff, climb, cruise, descent, approach, landing, taxi-in and parking through the
controller and movement loop. It started on the ground; single-flight approach
seeding was disabled.

With a newly rebuilt version 2 scenery cache, survey took 1.21 seconds, route
preparation 49.04 seconds and total preparation 59.44 seconds. Movement assertions
took 0.56 seconds; the whole test took 65.46 seconds. Five proposed flights still
failed conservative scenery checks. This establishes one complete protected flight,
not mixed cargo/GA traffic or three completed origins. The earlier multi-flight
arrival stall also remains open.

Run the opt-in fixture against an installed simulator and CSL catalogue:

```sh
OPENATC_TEST_SIMULATOR_ROOT="/path/to/X-Plane 12" \
OPENATC_SCENERY_CACHE_DIR="/path/to/test-scenery-cache" \
cargo test --locked --release -p openatc-engine \
  installed_production_network_lifecycles_reach_parking -- --ignored --nocapture
```

The production test keeps the 180-second preparation deadline, scenery checks and
movement assertions. Other nominal geometry tests use explicit accepting callbacks;
those results do not establish production scenery clearance.

### Scenery bounds and cache

Vegetation-area terrain bounds now clip candidate mesh triangles to the complete
bounding rectangle plus known object reach before finding the maximum elevation.
Tests retain an interior ridge and reject flight through trees, while excluding a
ridge outside the area's bounds. Scenery cache version 2 invalidates old geometry.
Nice's `natural.ags` area still refuses the affected routes after this correction;
more precise local placement or a valid alternative path is needed.

Airport discovery reads only the selected airport's apt.dat block. Network workers
share a scenery planner across directions and balance configured operator links
with nearby fields. Publication removes rejected actors and releases their live
stand/resource ownership. Planning exclusions for rejected candidates still need
review.

Earlier cache measurements found 2.10 seconds cold versus 0.12 seconds warm for the
Nice clearance fixture, and 66.24 versus 39.67 seconds for a full network run and
repeat. That first network run already had one tile cached and released only one
of eleven nominal routes. These older measurements used the previous scenery
interpretation and do not predict native frame rates. Peak process RSS reached
about 1.29 GiB; the 512 MiB retention cap applies per planner, not to total memory.

Scenery format references: Laminar's [autogen string format](https://developer.x-plane.com/article/autogen-string-ags-file-format-specification/)
and [autogen placement rules](https://developer.x-plane.com/article/x-plane-10-autogen-scenery-file-format/).

### TOML setup and reset

An isolated companion-engine API test imported 85 TOML files and exercised setup
status and rebuild. Reset dropped and recreated both cache schemas, removed probe
indexes and repopulated TOML. Tests cover content changes with unchanged size/mtime,
corruption fallback, preserved TOML types, invalid configuration, old writers and
last-good traffic profiles. Editable files, settings, plans and logs survived reset.
Malformed TOML produced a named error; correcting it allowed a retry. Developer mode
is required for rebuild. Active policies retain their normal reload boundaries, so
restart the plugin after Re-setup.

A release parsing benchmark measured 20.33 ms for initial SQLite population,
307.32 ms for 30 direct parsing batches and 111.89 ms for 30 warm cache loads.
This measures syntax reuse, not complete startup or traffic preparation.

### Procedure scope and remaining checks

The arrival, holding and wake work was checked against [FAA JO 7110.65](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/)
sections 5-7, 5-5, 4-6, 3-9 and 3-10, and [Australian MATS 77.3](https://www.airservices.gov.au/mats/docs/nos-saf-2000.pdf)
(effective 3 September–25 November 2026), sections 9.5, 9.6, 10.6 and 11.1.
The US terminal model uses FUSION's 3 NM service; generated Center service and
player-hold release use 5 NM. Australia uses a conservative 5 NM surveillance floor
plus its wake table. Unknown Medium MTOW receives conservative Medium-to-Light
protection. These assumptions do not cover every regional procedure.

Remaining work includes the five refused corridors, mixed cargo/GA/VFR lifecycles,
protected vector or extended-route alternatives, broader hold entries and amendments,
player/generated parity, native Steam testing and Windows/macOS acceptance. Generated
traffic remains disabled by default. See the [roadmap](ROADMAP.md).

## Earlier validation

The records below describe earlier builds and tests. Current traffic and setup
results are listed above.

## General-aviation piston parking — 21 October 2026

Piston aircraft can now use general-aviation tie-down and hangar stands whose published equipment lists turboprops but omits props. X-Plane publishes many GA ramps that way, including all 151 stands at Cannes LFMD, so the previous equipment test rejected every piston aircraft there.

The apt parser appends `props` to those stands when the kind is `tie_down` or `hangar` and the operations include `general_aviation`; `supports_aircraft` also accepts a piston aircraft on such a stand when the data has not been normalised. Gates and airline ramps are unchanged, and a stand that lists only `helos` still refuses piston aircraft.

Evidence: new `parking` unit test; `installed_cannes_piston_scene_uses_propeller_stands` passes with the C172 frames on propeller stands and pavement; core `parking` 6 passed; workspace 585 passed and 0 failed.

Not claimed: KSDF GA Apron still has an unverified runway-entry connector and an unverified surface connector at 38.1828784, -85.7244169, so the mixed cargo/GA fixture still cannot depart GA there. Every GA Apron stand is on pavement and the airport loads 43,485 pavement triangles; the connector point lies 3.065 m from the nearest triangle. No tolerance was added, because that would let a route cross a real three-metre gap. Exclusions were ruled out: clearing every exclusion for the tile leaves the triangle count and the gap unchanged, so the pavement is genuinely absent from the source. The fix is to route around it or correct the source pavement.

## Advisory traffic at untowered fields — 21 October 2026

Untwowered fields that publish a CTAF or UNICOM frequency now carry advisory traffic. A field qualifies only when no Tower service is published for it, so advisory traffic cannot appear where a controlled clearance is required; a towered airport is refused even when it also publishes a UNICOM frequency. The frequency must be inside 118.000–136.975 MHz.

The circuit is a right-hand pattern: downwind on the right of the runway flown against the landing direction, base across to the extended centreline, final on the runway heading, then a landing roll on the published runway. Every leg carries one self-announced call, and no wording contains a clearance. Calls follow the standard advisory form — field, traffic, callsign, leg and runway, field — for example, "KCTF traffic, N12345, entering downwind runway 09, KCTF". The advisory procedure is the authority for that traffic: the movement world grants the segments of the admitted route and refuses a taxi, a runway entry or any other surface movement outside the published pattern, so the controlled-clearance path is not reachable from an advisory route. The pattern repeats: the aircraft climbs from the end of the landing roll, rejoins the downwind and flies the circuit again, starting from the exact point the previous circuit ended. A runway held by the player or another aircraft at the final call turns the aircraft away instead, and the go-around call replaces the final call.

Straight-in arrivals and departures cover aircraft that are not flying a circuit. When the shared capacity still allows one, the field publishes an arrival from about five miles final: it announces five miles final, final and runway vacated, uses the same runway resource for its landing roll, and is removed from the advisory map, frames and movement world when the roll completes. A departure starts at the runway threshold, announces the runway and leaving the pattern, holds the runway resource during the climb-out, and is removed after it turns out. Neither repeats, and the runway-occupied go-around still applies to arrivals. No taxi is modelled at an untowered field; the departure starts at the runway and the straight-in at five miles.

The route compiler still requires a protected landing or closed holding continuation for controlled traffic. Advisory departures use `Route::compile_departure`, a separate entry point, and the caller retires the aircraft when the route ends.

Cross-country VFR flights are published when two untowered fields are 10-40 nm apart. The route starts on the origin runway, climbs and cruises, joins the destination five-mile final and lands. Departure and leaving calls use the origin frequency; five miles final, final and runway vacated use the destination frequency. The aircraft is removed after landing. When the destination runway is occupied at the final, the arrival switches to a local go-around on the destination pattern and never lands on the held runway.

Selection is bounded. The untowered fields come from the published station index, are sorted by distance from the player and are limited by the same candidate and radius budget as the rest of the traffic network. At most two advisory aircraft are published, and each needs a light aircraft with an installed model that can use the field's runway. Advisory aircraft share the 63-aircraft scene cap with the controlled fleet: the capacity is the remaining slots, capped at two, so advisory traffic never pushes the scene beyond the cap. The calls are spoken on the published frequency, so they are heard only when that frequency is tuned, and they are recorded as background information rather than as an instruction to the player. The wording is editable in the `ctaf_*` entries of `speech/runtime/responses.toml`, so a customised library must carry those keys with their declared placeholders.

Evidence: core `traffic::ctaf` 8 passed, core `traffic::motion` 26 passed, engine `generated::ctaf` 9 passed, `openatc-engine` 270 passed, workspace 593 passed and 0 failed, and the locked Linux release build passes. The documented `--all-targets -D warnings` clippy command reports 0 errors; one narrow item-level allow remains on `enroute::crossings`, which takes its request by value because the function advances mutable departure borrows. The workspace count is 593 tests after adding pavement-fallback and apt-parser regressions and removing a duplicated `#[test]` that had registered one test twice. No native simulator acceptance is claimed: the installed-scenery fixtures need the user's X-Plane installation, and advisory traffic has not been seen in the simulator.

## Traffic planning cost, CSL catalogue and prompt reload — 20 October 2026

Measurements were taken with the release test binary on the installed X-Plane 12 navigation and scenery data. No simulator process was running, so these are planning and data-loading costs, not native frame time, rendering cost or audio latency.

| Case | Wall time | CPU | Peak RSS | Result |
| --- | --- | --- | --- | --- |
| Mixed cargo/GA network fixture (KMEM–KSDF, 10 surface aircraft) | 102.8 s | 102% | 1.32 GiB | 3 complete nominal flights, 1 released; fixture still fails its origin/category assertion |
| Seven-flight airline/regional network fixture | 179.3 s | 102% | 1.20 GiB | PASS, seven flights reach parking |
| Bluebell CSL catalogue scan | 0.022 s | one core | not separately sampled | 2,027 models, 0 warnings |

The seven-flight preparation is close to its 180 s fixture deadline. The measured cost is scenery, navigation and full corridor protection, not the terminal-candidate or connectivity changes: the GA direct-approach candidates are only built when a GA aircraft is requested, and the fixture requests none.

Cargo ground coverage was measured separately on KMEM. The D-size cargo types (A306, B752, B763) have 53 to 56 connected stands and can reach the departure runway. The E-size types (B744, B788, B777F) have no connected stand because the Cargo South and Cargo North aprons are published as `taxiway_D` while the stands are equipment class E; refusing those types is the honest reading of the field data, not a missing taxi edge. At KSDF, B752 and B763 connect from 55 to 94 stands depending on runway end; the three UPS Ramp 11 stands cannot obtain a crossing approval. The remaining cargo arrival blocker is airborne: KSDF 17L has no forward exit after the B752 stopping distance, and 17R requires a 5,016 m radius at 294 kt inside a 3,716 m leg for the published CRSON7/MBELL5/I17R crossing.

GA terminal selection now joins a published approach directly when no STAR entry is inside the aircraft envelope. The KMEM–KSDF C172 candidate uses I17L from SLGRR instead of the FL180 MBELL5 entry through JIINN. On the mixed fixture the JIINN refusal is gone; the remaining GA refusal is the KSDF GA Apron taxi-in/pavement connection, not the high-altitude STAR.

Editable prompts are reloaded when `prompts.toml` or `interaction.toml` changes. A partial catalogue is rejected and the running copy is kept. The supported test edits a catalogue, observes the reload, and then supplies an incomplete catalogue and checks the refusal. Full end-to-end TTS/STT latency, CSL rendering frame time and native simulator acceptance were not measurable without the simulator and a running voice service, and are not claimed.

## Installed scenery and airborne permission checks — 17 October 2026

The production worker now calls the complete flight factory with installed grid MORA and enabled physical scenery meshes. The DSF reader supports plain and 7z-compressed tiles, scaled coordinate pools, physical triangle primitives and elevation rasters. It retains scenery-stack priority and checks the full line through intersecting triangles, including ridges between endpoints; missing mesh coverage remains unknown. Codec, tile, point, triangle, cache and worker limits are bounded. The decoder uses one thread inside the planning worker, with no simulator SDK calls. Its tests cover malformed records, pooled coordinate encodings, overlay/base priority, disabled packs, negative tile coordinates, tile boundaries, ridges and unknown gaps. The installed Nice base mesh and enabled custom scenery stack decode successfully.

Static obstacle groundwork now retains DSF object definitions, placement, rotation and optional height, then reads the drawn OBJ8 triangle and line bounds. Library lookup retains all indexed regional, seasonal, ratio and fallback variants, including a taller alternative to a local object. Unsupported animation, terrain alignment, missing resources and exhausted resource budgets remain unresolved. Bounds checks include the aircraft's horizontal envelope. Ground-only polygon and line resources are classified from their actual file headers across every indexed library variant. A virtual .pol/.lin suffix alone cannot suppress an obstacle; a redirected object, missing file or unknown header remains unresolved. Local mesh resolution does not yet establish neighboring-tile coverage, polygon/facade/forest or road-network protection, so object-bearing low-level corridors remain withheld. The installed enabled-stack and synthetic placement/library checks pass; this is not production airborne or native simulator acceptance. See Laminar's [OBJ8 specification](https://developer.x-plane.com/article/obj8-file-format-specification/) and [library specification](https://developer.x-plane.com/article/library-library-txt-file-format-specification/).

This is physical-mesh evidence. Low-level object, polygon and network obstacles remain unresolved and prevent production release. The latest installed Nice–Milan check constructs nine complete nominal flights and their controller/recovery joins, but releases zero flights because those obstacle checks remain incomplete. The factory is wired to the existing Tower-ready installation path for independently releasable flights; it does not use an accepting fixture callback. Generated traffic stays disabled by default. Scenery data comes from the user's installation and is not redistributed. See Laminar's [DSF format](https://developer.x-plane.com/article/dsf-file-format-specification/) and [DSF usage](https://developer.x-plane.com/article/dsf-usage-in-x-plane/).

Airborne permissions now check proposed movement against a forecast of actual accepted scopes, speed changes and registered fallback holds. Continuous relative-motion checks test horizontal proximity during the interval with insufficient vertical separation, rather than comparing only snapshot endpoints. The editable conservative defaults are five nautical miles and 2,000 feet; these are simulation defaults, not a claim that all regional separation or wake rules are implemented. The forecast covers the proposed clearance scope and an additional planning interval, bounded at fifteen minutes. Unknown external speed or climb increases the prediction envelope; an unidentified provider target is kept separate from the player.

The controller checks before an airborne or onward grant and again when its readback completes. Accepted but unstarted departures are also rechecked before movement; a new conflict removes takeoff and initial-SID authority, preserves the cancellation while radio is busy, and requires a fresh clearance after traffic clears. The controller regression covers crossing traffic beyond the runway monitor before, during and after takeoff readback, then completes a fresh exchange through parking. Wake expiry alone cannot release an obstructed departure path. Accepted landing clearances are also rechecked against airborne conflicts. The controller/movement regression covers both a runway occupant and nearby airborne traffic after readback, carries revocations through a busy radio and worker publication, and follows the checked missed route without rollout. Once recovery starts, delayed landing cancellations are discarded so they cannot tell an aircraft already going around to continue approach. Further airborne conflict resolution, regional wake/sequence rules and native acceptance remain open.

After a contingent holding report, the controller can prepare the unchanged checked onward route, perform any required handoff, recheck the proposed exit against traffic, and accept a separate onward readback. It leaves at the holding fix, then continues through landing and parking. This automatic path is covered by the actual movement/radio-loop regression. It does not grant an automatic exit from a published missed-approach hold; that still needs a newly planned approach. Successful scene replacement clears old grants, pending radio state and rejoin ownership rather than reusing them for recycled traffic IDs.


Terminal planning prefers less-used candidates within the configured catalogue budget, retaining the original candidate indices for controller and recovery metadata. A preferred candidate must still connect through landing and a free compatible stand; failures retry other candidates. This is procedure selection for the current airport pair, not complete independent destination coverage or GA VFR generation.

## Airborne clearance-limit recoveries — 17 October 2026

Production assembly now constructs recoveries at every airborne movement limit. Final-approach limits use the separately checked published missed variants. Climb, cruise and descent limits use a level racetrack aligned with the actual incoming path. The controller instruction and readback state the contingent geographic limit, magnetic inbound course, right turns, leg time, level and maximum IAS. The initial SID's contingent limit is included in the takeoff and whole-SID clearance. Holding geometry remains subject to independent protection; the movement engine also checks that the preceding segment provides enough distance to slow before entry. A recovery cannot silently change its cleared pattern or inherit a different limit's authority.

The editable operations policy caps these controller-specified limit holds at 200 KIAS; published missed-approach holds retain their own restrictions. IAS is converted through the retained atmosphere and wind at the actual level, so high-altitude ground speed can exceed 300 knots without raising the indicated holding limit. Holds use 60-second legs through 14,000 feet and 90-second legs above, with bank capped at 25 degrees and the configured turn-rate limit. These are nominal movement envelopes, not aircraft-specific certified holding performance. See [FAA AIM 5-3-8](https://www.faa.gov/air_traffic/publications/atpubs/aim_html/chap5_section_3.html).

The installed-data test now joins the actual Nice–Milan jet and turboprop nominal routes, published controller contracts, every clearance-limit hold and every final recovery into validated complete prepared flights. It also checks missing recoveries and mismatched holding instructions. Its accepting protection callback is a geometry fixture, not terrain or native simulator acceptance. Focused movement checks cover upstream braking, continued flight around the hold, excessive entry-speed rejection and altered-pattern rejection; a radio check covers one holding report without stale onward grants or retuning.

A checked clearance-limit hold can now prepare an onward suffix of its original route. Geometry and future recovery paths remain unchanged while controller, holding, landing and taxi scopes and vacated reports are reindexed. The controller-loop regression uses this producer, completes a handoff/readback when the onward controller differs, then accepts a separate onward clearance and leaves the hold only at its fix before continuing to parking. It retains owner-invalidation and clock-reset checks. Published missed holds require a newly planned approach; this suffix does not turn a missed route backward or grant automatic onward movement.

The en-route planner independently checks every solved curved cruise edge against known grid MORA and both endpoint pressure references. A higher local floor propagates back into the adjacent crossing constraints before the coupled speed/turn/level profile is solved again. Unknown cells, incompatible ceilings and nonconvergence reject the draft. This does not replace missing terminal corridor protection. Full production activation, distinct route networks, nearby replenishment, GA VFR and native traffic performance remain open; generated traffic stays disabled by default.

## Controller contract construction and clearance levels — 17 October 2026

Checked flight assembly now has a controller-contract constructor using the selected SID/STAR/approach, retained IAS/weather references, actual taxi instructions and published station coverage. It refuses missing or mismatched weather, unknown runway resources, unavailable controllers and unrepresentable assigned flight levels. It constructs separate lineup, takeoff/initial SID, airborne, approach, landing/rollout and taxi permissions with full readbacks. Identical approach and taxi wording is retained within explicit spans. The initial SID clearance covers its intermediate procedural limits; those limits are not treated as separate altitude assignments. No complete production airborne flight is activated by this constructor alone.

The joint planner can select standard flight levels inside each crossing's reachable physical envelope. Available levels propagate both forward and backward before a preferred level is chosen, so a narrow later band can start descent earlier. IAS, turn geometry and travel time are recomputed together. Below transition altitude the underlying physical constraints are retained; the approach clearance retains its published vertical restrictions. The constructor independently converts the chosen level back through the same pressure reference. Tests cover out-of-band selectors, missing grid levels, future restrictions and nonstandard weather.

Station planning first narrows the global index to the two airports and area-control volumes intersecting the route bounds. Full segment coverage is still checked, including explicit combined duties. This avoids repeatedly scanning the global station index for each taxi segment. The complete assembly regression now uses the real contract constructor and drives those readbacks through the movement/controller loop to parking, including a cancelled-landing branch into the checked missed route. Full obstacle protection, distinct route networks, GA VFR and native acceptance remain open; generated traffic stays disabled by default.

## Landing clearance through rollout — 17 October 2026

Complete flight assembly requires an explicit Tower landing clearance before the flare. Its readback can cover the final landing segment and the complete contiguous rollout on the same protected runway, without another clearance at touchdown. Validation rejects spans that absorb taxi, a controller/frequency change, different wording, another runway permission or an incomplete rollout. Approach clearances cannot absorb this separate landing permission.

The runway check includes the landing/rollout interval and is repeated at readback and before the relevant movement limit. An unexpected conflict withdraws the upcoming landing/rollout grants without stopping the current airborne leg. If no new landing clearance is received, the aircraft follows its checked missed route. Busy-radio cancellations retain their identity, and movement-worker publication carries revocations back even if newer telemetry says the conflict has cleared. The complete assembly/controller regression verifies one landing instruction through parking, invalid span rejection, and cancellation after readback followed by a missed approach, with no rollout after the withdrawal. Native acceptance remains open.

## Generated departure conflict checks — 17 October 2026

Lineup and takeoff planning now check approaching traffic over the planned runway movement time plus the editable clearance buffer. This extends the existing timed crossing check; a takeoff gate no longer consults wake alone. The controller rechecks before accepting the pilot readback. Movement and observation refreshes withdraw an accepted but unstarted takeoff and its initial-climb authority if a conflict appears, retain the cancellation reason while the radio is busy, and require a fresh clearance after the conflict clears. An already rolling aircraft is not stopped by this pre-roll gate. Wake and other traffic have separate editable hold/cancellation wording.

The controller/movement regression places an arrival outside the fixed five-mile final radius but inside the configured movement-time protection. It verifies cancellation both at readback and after the grant, keeps the radio busy while the conflict clears, checks that the old takeoff/initial-climb permissions cannot restart the roll, then completes a fresh exchange and continues through parking. Wake regressions move the observed leader clear of the runway after liftoff, checking residual wake independently of continued runway occupation. These are source checks; production airborne preparation and native acceptance remain incomplete.

## Complete flight assembly and recovery checks — 17 October 2026

Production preparation now retains a complete nominal flight draft joining the actual departure surface, solved SID/en-route/STAR/approach, flare, runway exit and taxi route to a reserved parking stand. Releasing that draft requires an independent protection check for every airborne edge, the missed approach and holding circuit, and the flare. The final assembly also requires controller contracts and a checked recovery at every airborne limit before it can be installed. The production controller and all-boundary recovery providers are connected to the installed terrain checks, but unresolved low-level obstacles still prevent release in the installed fixture. These nominal drafts do not activate airborne traffic.

Checking the complete route exposed a taxi-in braking defect: a short turn required 14.8 metres to slow from 15 to 8 knots but provided only 4.8 metres. Surface planning now works backward from turns, clearance stops and parking to reduce upstream speed targets without moving taxi geometry or increasing a configured limit. A movement regression verifies braking and arrival at the stand. Installed Nice–Milan jet and turboprop geometry checks pass for the complete nominal join; their accepting fixture callback establishes geometry only, not terrain protection.

Final recovery construction retains the remaining lateral final path through the MAP for an early missed approach. A late recovery may join only the first published missed leg ahead, with its climb reprofiled to the original next fix and level; every modified edge needs independent protection. It rejects backward/off-track joins, insufficient climb distance, invalid positions and entry heights above the published missed/hold limits. For higher early entries, the planner now re-solves each final boundary against the original published restrictions and constructs one explicit amended missed level when needed. That level appears in the approach instruction and full readback; an onward rejoin cannot silently drop it. The installed Nice–Milan jet/turboprop geometry check verifies every early, MAP and supported late-flare recovery boundary. Unknown protection or a conflicting published ceiling still rejects the flight. Published missed protection is not assumed to extend past the MAP: see [FAA AIM 5-4-21](https://www.faa.gov/Air_traffic/publications/atpubs/aim_html/chap5_section_4.html).

A complete assembly/controller regression now drives a checked route and its recoveries through actual controller exchanges, readbacks, movement permissions and parking. This is source integration evidence, not a complete production factory or simulator acceptance. Distinct route networks, continuous nearby coverage, GA VFR, complete operational coordination and native performance remain open. Generated traffic remains disabled by default.

## Generated departure wake integration — 17 October 2026

Generated departures now consult the existing observed wake monitor before a takeoff grant and before the roll. In the supported US wake-policy region, a heavier native, generated or player departure can delay a smaller follower; explicit aircraft categories and runway relationships determine the interval. Player liftoffs are observed alongside traffic liftoffs, including at the generated scene's airports. A conflict during readback cancels the grant. A later conflict withdraws an accepted but unstarted roll and its initial climb authority, queues the cancellation until the radio is free, and requires a fresh clearance after the interval expires. Hold and cancellation wording is editable in `speech/runtime/responses.toml`. This is not worldwide wake-policy coverage or native simulator acceptance.

The actual controller/movement regression observes a B77W liftoff and holds a C172 follower for the intersection interval, using both a native target and ownship as the leader. Both variants subsequently depart and complete taxi-in and parking. A second regression introduces wake during takeoff readback and verifies cancellation without a roll. A third introduces wake after accepted takeoff clearance, keeps the radio busy beyond the wake interval, verifies that neither the roll nor its initial SID authority restarts, then completes the cancellation, fresh clearance and parking. Existing wake-monitor tests cover pause/clock behavior and arrival-threshold persistence.

The wake observer now retains leader identity, excludes self-wake, and includes ownship plus a bounded external traffic view. Generated airport runway geometry is used even when a different airport is selected in the browser. Takeoff direction is matched to the closest aligned runway resource, avoiding selection of a crossing runway merely because it appears first. Movement worker publication reapplies current wake restrictions. Already rolling aircraft are not stopped by this pre-roll gate. A direct movement regression verifies the unstarted-roll withdrawal, generation guard and refusal to stop an already moving departure. It caught an internal-speed-only check that was wrong when no acceleration profile was configured; elapsed roll progress is now checked as well.

Research: [FAA JO 7110.65, Chapter 3, Section 9](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap3_section_9.html), departure wake and runway separation. This integrates the existing US CWT policy; it does not extend those rules worldwide. Validation: generated traffic 37 passed, five installed-data cases ignored; wake-monitor three passed; core traffic 59 passed, four installed-data cases ignored; shipped configuration two passed; locked Linux plugin/engine release passed. No installation, native audio/visibility/FPS or Windows/macOS acceptance is claimed. Complete airborne production, protected recovery/early and late go-around planning, distinct route networks, GA VFR and the wider traffic scope remain open. Generated traffic stays disabled by default.

## Generated controller duties and hold-exit ownership — 17 October 2026

Generated flight clearances now accept published Departure controllers for climb and explicit combined station duties for their supported phases. A queued recovery-hold exit is tied to its original flight: removing the active flight, removing its aircraft frame or replacing its plan cancels the pending exit and leaves the current holding circuit intact. Focused controller tests follow both dedicated Departure and combined Tower/Departure cases through parking, and check stale-exit cancellation through another holding circuit.

The flight validator previously allowed only Approach/Center for climb, despite the station index supporting Departure. It now accepts Departure for climb and checks explicit combined duties against each phase's permitted services. Ground-only stations remain invalid for climb. Pending hold exits retain the original Prepared identity; recovery synchronisation cancels both the queued world exit and its pending exchange when the flight/frame owner is gone or replaced. The regression runs three invalidation cases after readback and advances for another circuit, checking that the onward route is never activated.

Validation: generated traffic tests 34 passed with five installed-data tests ignored; core traffic tests 58 passed with four installed-data tests ignored. The locked Linux plugin/engine release build passes.

These are controller-consumer fixes. Production preparation still does not install complete airborne flights: real route/corridor protection, all-boundary early/late recoveries, controller construction, distinct route networks and GA VFR remain open. Generated traffic remains disabled by default. Native visibility, audio, performance and platform acceptance are separate.

## Generated recovery-hold rejoin consumer — 17 October 2026

The controller consumer can now queue a complete onward flight from a recovery hold after its pilot readback. The aircraft finishes its current circuit and leaves at the holding fix; the new plan takes effect there, with separate landing and taxi permissions. Incomplete recovery sets and incompatible joins are refused. A simulation-clock change cancels a queued exit while preserving the hold. The integrated consumer regression reaches parking after rejoin; this does not establish production route generation or native simulator acceptance.

The regression drives the installed full-flight fixture into its descent-boundary fallback, advances into the holding circuit, rejects an incomplete onward plan, runs the actual controller/pilot exchange, verifies no movement or plan replacement at readback, then advances through fix activation, landing, runway vacation and taxi-in to completed parking. A cloned simulation clock reset cancels the pending exit and remains holding through another circuit. The geometry-only fixture uses an accepting protection callback; it is not evidence of terrain or obstacle protection. Focused generated tests: 33 passed, five installed-data tests ignored. Core traffic tests: 58 passed, four installed-data tests ignored. The additional clock-reset variant passes separately. Full production factory, all-boundary/early/late recoveries and native acceptance remain open; generated traffic stays disabled by default.

## Parked radio requests — 17 October 2026

Radio check was already phase-independent. New menu and endpoint regressions verify it before a flight plan exists, with Clearance, Ground and Tower on both COM1 and COM2, without changing the flight phase, plan or clearance. The same endpoint test verifies local weather and altimeter replies after ingesting a real-format simulator sample. These two requests now have catalogue buttons; previously they were parser-only. State polling now requests weather samples at the tuned airports of both radios, deduplicated alongside planned airports, so an unfiled plan does not prevent local weather sampling. No controller requests are offered on ATIS/Unicom or an unknown station. Ground movement and flight clearances retain their existing prerequisites.

Research: [CAA CAP 413 edition 24](https://www.caa.co.uk/media/km2juvmx/cap413-edition-24-radio-telephony.pdf), sections 2.92–2.94 (test transmissions), aerodrome departure information, and [CAA radio guidance on requesting QNH](https://infringements.caa.co.uk/tutorial/radio-telephony/). Radio checks are not conditional on a filed IFR clearance. Native parked-window visibility/audio remains a simulator check; endpoint and menu tests do not establish it. Focused parked menu, endpoint, COM2 routing, catalogue and shipped-configuration checks pass; the locked Linux plugin/engine release build passes.

# Validation

## Takeoff roll and positive-rate callouts — 17 October 2026

The callout sequence is Airspeed alive, 80 knots, V1, Rotate, then Positive rate. V2 is retained as a performance reading only; it is not preloaded, queued or announced. Legacy policy files can retain a `v2_text` entry without restoring V2 speech. Roll/climb wording and thresholds are editable in `aircraft/callouts.toml`.

Positive rate requires an observed ground roll followed by no weight on wheels, at least 5 feet AGL and at least 100 ft/min climb sustained for 0.3 seconds. Ground-roll vertical-speed noise, a brief wheel bounce, descent, missing instruments or attaching while already airborne cannot trigger it. Rejected takeoffs re-arm after stopping. Missing V1/VR values do not prevent the roll or climb calls. Rotate retains priority over unfinished V1 audio; equal-speed crossings retain the combined clip.

Callout sampling now runs in the lightweight per-frame callback, capped at 50 Hz, rather than the 2 Hz controller tick. It reads only IAS, ground speed, weight-on-wheels, AGL, vertical speed, pause and simulator time; traffic and radio readers are excluded. Controller update frequency is unchanged. The plugin suite passes 34 tests with two environment-specific tests skipped. Native simulator audio and performance remain to be checked.

## Flight-plan clearance feedback — 17 October 2026

Empty Planning drafts now receive a controller response addressed to the callsign: “you have not filed a flight plan.” Partial or invalid drafts receive an incomplete/invalid response with the missing fields or validation error. Drafts with valid airport codes and altitudes must also have a departure runway and a route or route fixes before automatic submission. Radio checks do not require a flight plan.

The UI sends rejected drafts for a normal controller reply rather than stopping at a local notice. The engine checks the current Planning draft and the filed plan before issuing a clearance. Missing, partial, invalid and malformed request data, direct engine requests, and an empty draft alongside a previously filed plan are covered by the endpoint regression. Refusals retain the receiving radio, controller voice and callsign, and leave the filed plan and clearance unchanged. Core validation, UI submission and shipped-configuration checks pass. The locked Linux plugin/engine release build passes; native simulator playback still needs confirmation.

## macOS alpha window report and source fixes — 17 October 2026

A contributor reported window rendering, pop-in crashes and collapse/expand drift on a Mac Studio M4 Max running macOS 26.7.1 and X-Plane 12.4.4 beta 4, with ToLiss A319 and FlightFactor 777. They tested their proposed fixes in the simulator. The integrated version still needs a new Mac build and simulator check.

macOS now selects a Rust OpenGL 2.1 compatibility renderer. It preserves host program/buffer bindings, server/client state and matrix stacks, uploads the font atlas with local pixel-store settings, handles Retina clipping and honours per-command vertex offsets above 65,536 vertices. Other platforms retain the existing glow renderer. Renderer initialization is capped at three attempts, with complete log lines. Compatibility bindings are generated during the build.

Draw callbacks only queue window actions. A separate lightweight main-thread flight loop applies changes between frames, independently of the slower controller/traffic updates. Requests are owned by the window state rather than global stale window pointers; coalescing retains one-shot restore/height requests and accumulates movement. The loop is stopped before plugin shutdown. Native SDK height limits stay compact even while the interface enforces its expanded minimum. Monitor recovery uses the same compact limit. macOS popped-out windows use their native frame for movement and resizing.

Validation: locked plugin suite 33 passed, two ignored; Linux release plugin build passed. Explicitly executed the ignored Mesa EGL regression under a real software OpenGL 2.1 context: over 80,000 vertices rendered, the final pixel checked, and viewport, projection, texture units, client texture unit, unpack row length, buffer bindings and depth state restored. On hosts with multiple EGL drivers, select Mesa with `__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json` and set `LIBGL_ALWAYS_SOFTWARE=1 MESA_GL_VERSION_OVERRIDE=2.1` for this test. This is actual GL 2.1 rendering evidence, not macOS simulator or Mac native-build acceptance.

## Alpha feedback fixes and Windows runtime audit — 17 October 2026

The actual published Windows server ZIP was downloaded and its SHA-256 matched the release digest. PE imports include MSVCP140, MSVCP140_1, VCRUNTIME140, VCRUNTIME140_1 and VCOMP140, while the archive lacks those files. This confirms a packaging omission independently of the unnamed tester error. Windows packaging now copies the licensed toolchain’s release CRT and OpenMP libraries beside each launcher, engine/plugin and speech program. The temporary Inno discovery helper receives runtime DLLs before execution. Installer preflight performs the same runtime availability check before long model builds, and both workflow Visual Studio imports preserve VCToolsRedistDir. A focused fixture verifies required-library failure, CRT/OpenMP discovery and exclusion of debug runtimes. No runtime binaries are added to source control.

The earlier V2 playback policy has been replaced by the roll-and-positive-rate sequence described above. V1 and VR keep their actual performance values; V2 is diagnostic only.

Final locked Linux plugin suite: 31 passed, one ignored. Packaging suite: 13 passed, one ignored. Final plugin, engine and packaging-tool release build passed. Earlier callsign/controller, UI input and monitor checks also passed. Clean Windows package startup, simulator input, ultrawide placement and native audio still need confirmation. The save/replay popup trigger and exact DLL name remain unreported.

## Floating-window monitor recovery and V-speed validation — 17 October 2026

Floating windows now fit an individual full-screen X-Plane monitor at creation and saved-bounds restoration, using boxel coordinates. Windowed mode falls back to the X-Plane desktop bounds. Display layouts are refreshed at most once per second by the window flight loop, and recovery waits until dragging ends. Minimum dimensions are capped to the selected display, so a smaller monitor can still contain the window. Popped-out OS windows and VR are not clamped against floating-window coordinates. Geometry checks cover ultrawide edges, negative monitor origins, empty space between diagonal monitors, smaller displays and unchanged valid bounds. The Windows ultrawide report still needs native confirmation.

A V1 bug above Vr is rejected without suppressing Rotate. V2 is diagnostic only and does not trigger speech. Observed and usable values remain visible in diagnostics. The callout suite passes; the preceding full plugin suite passed 29 checks with one ignored, and the final Linux plugin release build passes. Native Windows testing remains pending.

The repeated AI-server DLL report remains unidentified: the supplied text does not name the DLL. The verified Vulkan-loader staging fix is already present; optional CUDA-provider dependency failures can fall back to CPU, but that has not been established as the reported cause. A complete startup error line or screenshot is required before assigning a DLL fix.

## First-alpha callsign, input and callout repairs — 17 October 2026

Pre-clearance radio checks and the ATC callsign label now use the reviewed Planning callsign. Requests carry that identity to the engine, which applies it only while parked on the ground without an issued clearance. Filing a complete route remains separate. Focused tests cover edited/default identities, COM2 controller routing and transcript identity, plus rejection of malformed identities and changes after clearance or airborne starts.

The host now retains keyboard focus for an active widget or keyboard capture, including the frame before ImGui updates its text-input flag. Headless tests cover field click, release and typing, and header dragging in full, compact and narrow/wrapped layouts. A hover hint identifies the drag area. Focus transitions are recorded without logging typed characters. This is protection against shortcut leakage, not proof of the reported X-Plane situation-save/replay-load popup cause; no first-party code directly invokes those save/load APIs. The tester has not yet identified the triggering action, and a native Windows simulator recheck remains necessary.

Actual V1/VR/V2 performance values remain unchanged; only V1 and VR trigger speed callouts. Positive rate uses weight-on-wheels, AGL and climb telemetry. Existing callout tests cover speed identities, close/equal crossings and once-only scheduling. The shipped configuration tests pass. The locked Linux plugin/engine/tool release build passed; no Windows simulator or audio acceptance is inferred.

## First Windows alpha tester reports — 17 October 2026

Two testers report successful plugin/server installation and configuration, remote AI-server connection and button radio checks. One additionally confirms microphone and controller-voice tests, SimBrief import and the interface layout; the other has not checked audio yet. These are initial user reports, not a complete Windows flight or all-format installer acceptance. Callsign stuck at the default VH-BIL and recurring X-Plane situation-save/replay-load errors remain open. The developer also reports V2 audio after Rotate again; timing and aircraft-specific callout policy remain under investigation. No fix for these new reports is claimed.

## Intel Mac builds opt-in — 16 October 2026

Both Intel Mac products are omitted from tag releases and manual all-platform builds; explicit manual macos-intel builds remain supported. Executed the exact matrix script for tagged all-platform and manual all-platform runs (six builds, three preflights), explicit Intel both-products (two builds, one preflight), and each Intel product separately (one build, one preflight). All passed. Publication additionally requires a push event, keeping manual tag-selected builds artifact-only. No hosted workflow was changed or cancelled; this takes effect after the user commits/pushes the workflow.

## Linux Vulkan download replacement — 16 October 2026

The full LunarG SDK download stalled during a hosted run. Replaced the Linux-only SDK download with Ubuntu 24.04 packages `libvulkan-dev`, `glslc` and `spirv-headers`; Windows SDK/runtime handling is unchanged. In clean Ubuntu 24.04 container builds, compiled the actual pinned llama-cpp-sys 0.1.158 Vulkan backend (329 build steps) and whisper-rs-sys 0.15.0 Vulkan backend (316 steps), including shader generation and shared-library linkage. Also executed the exact replacement workflow shell, with container root replacing sudo: package checks, Vulkan 1.3 shader compilation and loader link/run passed. Optional shader extensions unsupported by the distro compiler are disabled by existing backend detection. These checks establish build compatibility, not hardware inference, optimal GPU performance or full hosted package acceptance. Workflow parsing and diff checks pass; no tag or release was created.

## Final release upload audit — 16 October 2026
The follow-up downstream audit exercised the exact publication shell with all eight platform/product outputs: 18 package files and 18 checksum sidecars were selected, replay reports excluded, and a missing sidecar blocked the upload. Intel Mac source inspection confirmed ONNX 1.28 uses an `@rpath` install name and `@loader_path` runtime lookup; pinned ort-sys enables copying dynamic libraries beside release binaries, and distribution copying dereferences them into the package. The real packaged server and recognition executables run `--help` before archives are completed. ZIP/DMG tooling passed the native preflight; the remaining Intel ONNX compilation, Rust linkage and real macOS runtime probe still require a completed hosted build. GitHub showed no workflow runs at the latest browser observation after user cancellation/deletion; the former 34-percent build was not available to inspect. No native Mac completion is claimed.


The flight replay writes `packages/flight-validation`, and the previous upload glob included that directory. The release shell then passed `release/*` to GitHub CLI, including the directory. Restricted package artifact selection to `openatc-*`, explicitly selected supported archive/installer files for publication, and added SHA-256 verification before upload. Executed the exact workflow publication script with a temporary fake GitHub CLI: a valid package/checksum pair passed while the replay-report directory was excluded; corrupt and empty inputs failed before the CLI call. Workflow YAML and diff checks passed. No release was created by this check. The current alpha release API returned 404, so no existing release currently blocks creation. The hosted run still uses its original committed workflow.

## Flatpak PNG export repair — 16 October 2026

The supplied Linux packaging log shows successful AppImage generation followed by Flatpak rejecting the SVG icon as an unrecognized format. Flatpak now exports a supplied 128×128 PNG derived from the existing tower SVG. Early asset validation checks its header/dimensions. A dedicated packaging fixture calls the actual Rust Flatpak function with a minimal payload, exports the repository and produces a `.flatpak` bundle; it passes locally with Freedesktop 25.08. CI runs that fixture after installing the SDK/runtime, before the real server package operation.

This verifies Flatpak icon validation, export and bundle creation on the local host. It is not a full server install, launch or inference acceptance check; the hosted package run still needs the new source and PNG asset. The AppImage AppStream metadata warning in the supplied log was nonfatal and is separate from this Flatpak repair.

## Linux ONNX ABI and Actions runtime updates — 16 October 2026

A subsequent Linux AI-server link reached ONNX Runtime and failed on `__isoc23_strtol`, `__isoc23_strtoll`, `__isoc23_strtoull` and C++ `_M_replace_cold` symbols. These prebuilt dependency requirements exceed the Ubuntu 22.04 runtime. Only the Linux server build moves to Ubuntu 24.04, with a distinct Rust cache key; the plugin stays on Ubuntu 22.04. Server DEB/RPM metadata declares glibc 2.39 and installation guides state the native binary baseline.

Clean Ubuntu 24.04 container checks linked and ran a minimal `OrtGetApiBase` program against both the actual pinned CPU and CUDA-capable ONNX Runtime 1.28 static archives. This establishes the dependency's libc/C++ ABI linkage and API entry-point startup, not model inference or a complete hosted server/package build. The actual workflow preparation shell generated the expected eight product/platform entries and four preflights. Focused packaging tests, formatting and diff checks pass.

All external JavaScript actions referenced by the workflows now declare Node.js 24 in their fetched action metadata: checkout 7.0.1, cache 6.1.0, upload-artifact 7.0.2, download-artifact 8.0.2 and the current Rust cache v2. The Rust toolchain action is composite. The Node 20 Visual Studio action is replaced by the installed Microsoft developer shell; its exported environment was reset/re-imported on win11-dev and compiled and ran a C executable successfully. YAML and input/platform scope checks pass. Hosted integration of these action upgrades still requires the next run; deprecation warnings from earlier runs remain historical.

## Windows AI-server runtime staging — 16 October 2026

The Windows server in [run 38853995407](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38853995407/job/116822121583) compiled successfully but failed the subsequent runtime copy because neither the SDK Bin folder nor System32 contained `vulkan-1.dll`. The workflow now obtains LunarG's pinned runtime components separately and verifies the archive and x64 DLL checksums before the build. It stages the verified DLL and checks both server executables' startup linkage before packaging. No runtime DLL/archive is committed.

The actual workflow download/extraction/checksum block passed natively on win11-dev. The native release STT executable passed `--help` from an isolated folder containing the downloaded x64 loader; the server also passed with its other packaged dependencies present. This verifies startup linkage, not Vulkan hardware inference or full hosted packaging. Workflow YAML, platform scope, pinned checksums and diff checks pass; the hosted job must still rerun with these changes.

## Linux AI-server Vulkan loader link dependency — 16 October 2026

The Linux AI-server job in [run 38853995407](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38853995407/job/116822121397) built its native dependencies but failed linking the recognition sidecar: `unable to find library -lvulkan`. Shader compilation used the downloaded SDK, while Rust's final linker needed the loader development library in its normal search path. The Linux AI-only SDK step now installs `libvulkan-dev` and verifies a small loader-linked executable before building models' native libraries.

A clean Ubuntu 22.04 container reproduced the missing `-lvulkan` error before that package was installed. After installation, the same linker found it, and a program using the Vulkan loader's global entry-point lookup compiled and ran successfully without a GPU. Workflow YAML, shell syntax and diff checks pass. This verifies the missing link dependency, not a complete hosted server/package build or GPU inference; those still need the next native run.

## Windows integration fixture socket mode — 16 October 2026

Run [38853995407](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38853995407) passed the repaired shared HTTP checks and built the Windows plugin/engine, then failed `test-crew`. The mock model logged Winsock error 10035 before the expected cabin response. Its listener deliberately accepts without blocking so it can stop promptly, but the accepted worker connection must explicitly return to blocking mode on Windows. The fixture now sets that mode before applying bounded read/write timeouts. Application/engine code is unchanged.

The crew suite passes on Linux. All four actual Windows engine integration suites (`test-radio`, `test-crew`, `test-speech`, `test-flight`) pass against the native release engine with the repaired mock. The flight suite covers four controller/COM arrangements through parking. These are HTTP-engine checks, not native simulator graphics/audio acceptance. Hosted verification still requires a run containing this fixture correction.

## Windows HTTP fixture and Intel Mac CMake repairs — 16 October 2026

Native build run [38852127966](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38852127966) on `b17acbc` exposed two independent failures. Windows failed `openatc-http::tests::round_trips_json`: the fixture read the socket only once and could close with an unread POST body, resetting the connection. The fixture now reads complete headers and the declared body before replying; its POST regression exceeds the original read buffer. All three HTTP tests pass locally. The exact native shared gate (`openatc-core`, `openatc-settings`, `openatc-platform`, `openatc-http`, `xtask`) passes on the Windows 11 VM, followed by 20 successful repeats of the HTTP regression. Production transport behavior is unchanged.

Intel Mac's server failed while configuring ONNX Runtime's pinned psimd dependency: CMake 4.4.2 rejected its old minimum policy version. The Intel-only ONNX build now supplies `CMAKE_POLICY_VERSION_MINIMUM=3.5` and uses a revised dependency cache key. The exact psimd revision fails configuration under local CMake 4.3 without the flag and configures successfully with it. Workflow YAML, formatting and diff checks pass. This reproduces and checks the dependency-level repair on Linux; the full Intel Mac build still requires a new native workflow run. Apple Silicon server and both Mac plugin packages were already produced by the earlier run and do not establish Intel server acceptance.

## Windows folder-picker path repair — 16 October 2026

Interactive review found that discovered paths used Rust's extended-length Windows prefix, which Inno Setup's folder field rejected. Discovery output now converts drive and UNC paths to their ordinary folder-picker spelling while retaining canonical paths for internal filesystem checks. Eight focused tests pass on Linux and seven on Windows, including drive/network path regression cases. The rebuilt helper returns the actual Steam root without the prefix and the real plugin installer compiles. Continuing through the interactive wizard remains a separate user check.

## Updated Windows server package and fresh Steam discovery — 16 October 2026

The refactored AI server, recognition sidecar and launcher build natively on Windows 11 with the release Vulkan/Vulkan/CUDA-TTS features. Both server executables pass `--help` linkage checks. The actual server Setup.exe and ZIP are generated successfully; Inno Setup 6.7.3 accepts the dynamic light/dark theme and branding. This establishes package construction, not interactive dashboard, GPU, installation or uninstallation acceptance.

A fresh Steam X-Plane 12 installation exposed a discovery gap: no simulator log exists before its first launch, so the installer silently excluded it. Native executable validation now also accepts a matching X-Plane 12 Steam app manifest when the log is absent. The manifest must identify the exact selected installation; unrelated folders, another app identity and an existing X-Plane 11 log remain rejected. All seven focused installer tests pass on Linux and all six applicable tests pass natively on Windows, including the fresh-install regression. The rebuilt native helper discovers the actual newly installed Steam simulator without a log, and the plugin Setup.exe and ZIP build successfully. Interactive installer/install/remove acceptance remains separate. Standalone installations still require their version log.

Dashboard overlay follow-up: load/unload, Save/apply and deletion now show a modal spinner with actual phase/download progress until the acknowledged runtime generation completes. The user reported the change working on Fedora. A browser check additionally submitted an invalid context draft, verified the error overlay and Dismiss action, returned to the editor, and closed it without saving; the running server and saved settings stayed intact. The focused HTTP mutation test also verifies the generation/instance receipt. Reduced-motion styling and connection-error recovery are implemented; these additions have not yet been tested natively on Windows/macOS.

## AI-server dashboard and model controls — 16 October 2026

The persistent dashboard is built and installed on the Fedora AI server, preserving Qwen2.5 Instruct, Kokoro, fixed 4096-token context, the existing Vulkan/CUDA backends and inference port 8099. The user’s requested LAN dashboard is on port 38665; the default application remains loopback-only. Browser review covers the separated artwork header, one Models panel, direct settings tabs and first-run/rerun wizard. HTML/CSS/JS and branding are embedded, with editable TOML schema supplied separately.

Focused server tests: **17 passed**, covering settings/Origin validation, bounded native progress lines, component selection and disabled downloads, cache identity collisions, safe model-file deletion, shared usage labels and Linux worker-thread RSS accounting. Live Fedora checks exercised Save/apply through the actual browser, return to Ready on a new runtime PID, invalid-edit preservation, and same-config dashboard reuse without duplicate runtime/STT processes. A separate local manager-death check confirmed that closing its ownership pipe terminates its managed runtime.

Real-model HTTP checks on Fedora exercised language unloaded with voice/recognition available; voice unloaded with recognition available; all components unloaded with liveness available and model endpoints returning 503; language loaded alone with successful tokenization and short completion; then all original components restored. Speech produced a WAV and recognition processed test audio. These establish model serving and lifecycle behavior, not simulator audio quality or a flight acceptance claim. A separate fresh 77,691,713-byte recognition-model download exposed real dashboard byte counts at approximately 32 MiB, 64 MiB and completion, then checksum verification and actual recognition loading. It used an isolated temporary cache which was removed afterwards, leaving the main service/cache unchanged.

RAM/VRAM figures are process observations, not exact tensor-allocation measurements. Combined language/voice readings are explicitly shared and non-additive; recognition is measured separately. Missing GPU counters remain unavailable. Windows/macOS dashboard, process cleanup, accelerators and installer-theme acceptance still need their native checks. The new Inno light/dark configuration has not yet been rebuilt in the Windows VM; Fedora review comes first.

## Release build gate repairs — 16 October 2026

The shipped-configuration test now validates transponder settings with their own loader and leaves callout policy validation to the plugin tests; other aircraft TOML files still undergo strict aircraft-profile validation. Role instruction catalogues and text copies now agree on trailing newlines. The exact shared CI test command passes locally: 415 tests passed, 25 environment-dependent tests ignored, with no failures. Formatting also passes.

Manual native preflight run [38813386715](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38813386715) on revision `652c680` passed Linux x64, Intel Mac and Apple Silicon packaging checks. Windows failed before compilation because Chocolatey could not find the requested Inno Setup 6.4.3 package. Both Windows jobs now locate the compiler supplied by the Windows 2022 runner, export its absolute path through `ISCC` and let the actual installer preflight validate compatibility, without a Chocolatey download or downgrade. The subsequent Windows-only run [38814250978](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38814250978) on `4c12d69` found the compiler but rejected its zero file-version metadata before compilation. That metadata check is now removed: the actual fixture compilation validates compiler compatibility and reports its engine version. Workflow YAML and identical compiler-selection blocks pass local structural checks; Windows compilation still requires a fresh native preflight on this revision. Windows-only run [38814868497](https://github.com/billpayne4x4/OpenATC-AI/actions/runs/38814868497) on `d0e80cd` passed compiler discovery and installer tests, then failed linking the packaging executable because Git Bash selected Git’s Unix `link.exe`. Preflight now pins the Visual Studio linker by absolute path and runs Windows packaging in PowerShell, matching the full native build’s linker selection. That correction still requires a native rerun. These fixture checks do not establish application installation or runtime acceptance. Independent preflight results are retained when another platform fails. Manual full workflow runs upload artifacts without publishing a tag or release; see `RELEASE_PACKAGING.md`.

## Native Windows VM packaging checks — 16 October 2026

A Windows 11 x64 VM now has Visual Studio Build Tools 2022 (17.14.41), Rust 1.99.1 with the MSVC toolchain and Inno Setup 6.7.3. All five installer discovery/transaction tests pass natively. Initial fixture compilation exposed quotes around individual wizard-image list entries being treated as filename characters; the template now uses the compiler's comma-separated path format. Both plugin and AI-server installer fixtures then compile successfully with the real Inno compiler. Compiler discovery also covers per-user installations, as used on this VM, alongside machine installations.

This establishes native fixture compilation, not installation, upgrades, removal, signing or simulator acceptance. The locked native release build of plugin, engine and packaging tools also passes, followed by all four real HTTP-engine flight replay arrangements (separate controller duties, Ground/Tower, combined Tower, and COM2). A real plugin Setup.exe and ZIP were then generated successfully for interactive installer review. The accelerated Windows AI server and STT sidecar also build successfully with Vulkan/Vulkan/CUDA-TTS enabled. Their packaged `--help` checks pass, and the real AI-server Setup.exe and ZIP compile successfully. An initial Visual Studio-generator build exceeded the Windows path limit inside Whisper’s Vulkan shader generator; Ninja resolves that build failure. Model inference, GPU use and interactive setup/removal acceptance remain separate. Hosted Windows preflight must still run against the latest committed corrections.

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

Source checks: motion 11 passed, full-flight 7 passed, legacy generated 5 passed, 2 licensed-data tests ignored; release engine build passed. Native simulator testing remains pending. The actual production full-flight factory remains unwired and must construct these spans from real ground orders alongside protected airborne/recovery/landing planning. Generated traffic remains disabled by default; mixed-fleet and GA-VFR acceptance remains pending.

## Generated full-flight permissions and runway occupancy — 15 October 2026

A stronger negative movement test reproduced taxi-in starting without a readback after complete-flight installation removed legacy ground orders. The worker now requires explicit full-flight movement permissions and prevents adjacent taxi edges from inheriting a prior segment's approval. Crossings obtain the correct airport from the complete flight, retain duration-based conflict checks, and cancel/reissue when conflicting traffic appears during a readback or before entry. Typed runway reports must match a real protected runway before installation. External native/player runway occupancy now blocks the named runway resource as well as taxi cells, preventing a landing grant through an occupied runway.

The regression reproduces the former permission failure. Six checks pass, including the synthetic arrival crossing, runway report, controller and movement flow. Five legacy checks pass with two licensed-data checks ignored; the release engine builds. These results do not supply the missing production full-flight factory or establish native traffic, audio, FPS or platform acceptance. Generated traffic remains disabled by default.

## Generated arrival reports and runway exit clearance — 15 October 2026

The complete-flight consumer now requires typed runway-exit reports and emits the existing editable pilot report once on the runway controller frequency before Ground handoff or taxi-in permission. Observed aircraft dimensions and published holding lines gate the report; unknown or occupied runway states cannot progress. Landing preparation uses that same clearance geometry for its graph endpoint and rounded exit, so a point just outside pavement cannot be mistaken for a fully vacated aircraft. Editable aircraft dimensions are retained instead of an unknown-ICAO fallback.

Focused controller/movement/transcript checks pass 3; installed Nice A320 and Cannes C172 arrival-to-parking data fixtures pass 2. The separate holding-line check passes 1, covering the tail still inside the clearance boundary. Release engine build passes; diffcheck clean. These are source-level checks; native simulator testing remains pending. The actual production full-flight factory, mixed operations including GA VFR, and native visibility/audio/FPS/platform scope remain unfinished. Generated traffic stays disabled by default. Reference: [FAA airport holding-position markings](https://www.faa.gov/air_traffic/publications/atpubs/aim_html/chap2_section_3.html).

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

Root-workspace migration: the Linux plugin/engine release build, core/engine/UI/plugin tests, settings/platform/AI-core/audio tests, desktop compilation check, formatting, scoped Clippy, isolated radio integration and dependency audit pass from the repository root. The Linux archive was rebuilt with the new paths and verified for binary/docs/branding/lockfile presence and checksum. Python and Fedora script syntax checks pass. packages/ and target/ are listed in .gitignore, and vendor/xplane is explicitly allowed. GitHub CI and live simulator checks were still pending after the workspace migration.

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


## Generated en-route inputs — 17 October 2026

Focused MORA checks cover short grid-corner crossings, missing cells, date-line routes, great-circle latitude peaks, stationary positions and invalid/ambiguous paths. Generated en-route tests cover terrain anticipation at the preceding crossing, retention of the STAR entry's altitude/course/airspeed, bounded spacing/count, changed or missing MORA, ceiling conflicts and unchanged departure inputs after rejection. Policy tests cover the legacy default and invalid spacing values.

The generated module passes 27 tests, with five installed-data tests excluded from that ordinary run. The traffic-core suite passes 52 tests, with four installed-data tests excluded. New coverage checks long straight/turn sampling, draft protection refusal, consistent altitude-terminated leg timing and floating-point altitude-bound roundoff; a genuine 0.001-foot conflict still rejects. The installed Nice–Milan scenery/Bluebell preparation fixture was run separately and passes for mixed aircraft, including its candidate-retry and parking checks. Its weather samples are synthetic fresh airport observations. The fixture now requires production joint nominal drafts for both jets and turboprops and checks that a refused protection check releases no movement. No simulator was launched. These checks establish nominal geometry and level preparation, not protected complete flights, movement permissions, native rendering/audio/FPS or Windows/macOS acceptance.

MORA grid semantics follow the [X-Plane minimum off-route altitude data specification](https://developer.x-plane.com/wp-content/uploads/2020/03/XP-MORA1150-Spec.pdf). A grid floor does not establish the protection of a low-altitude terminal procedure or an arbitrary curved connection.


Nominal joint solving seeds initial airspeeds from the starting/lower preferred height within each published altitude band, then iterates levels, TAS, wind and turn geometry to convergence. Preferred cruise-height TAS no longer determines the first SID turn. Straight portions of Dubins connections use the configured distance spacing; curved portions additionally retain the two-degree angular limit. The 4,096-point complete airborne budget remains unchanged. Altitude-terminated legs use the same endpoint TAS as their nominal constant-speed movement segment. Vertical scheduling snaps only floating-point roundoff, bounded by eight machine epsilons at the level magnitude, within the original published altitude band.

The locked Linux release plugin/engine build and all four HTTP full-flight replay arrangements pass with these solver changes. No binaries were installed during this batch.

## Traffic speed boundaries and published missed approaches — 17 October 2026

The traffic-core suite passes 55 tests, with four installed-data tests excluded, and the generated module passes 28 tests, with five installed-data tests excluded. A reproduced movement defect lost physical elapsed time at a speed boundary. Regression checks now cover exact acceleration/coasting distance, slower approach entry at three update intervals, and refusal of a too-late slower recovery without replacing a valid fallback. Shared trajectory endpoints retain the original crossing coordinates exactly.

The actual candidate planner now prepares a nominal missed approach from its explicit published MAP through supported missed legs to the named published hold. Fix-to-altitude legs require their resolved starting fix. An independent synthetic fixture checks MAP anchoring, holding altitude/speed, unknown fixes, unsupported legs and refused path/hold protection. The installed Nice–Milan mixed jet/turboprop fixture passes separately, including complete missed/hold geometry and rejected protection. Its weather is synthetic; permissive callbacks in geometry tests are not terrain or obstacle evidence.

Early missed approaches must retain the applicable lateral path to the MAP before turning; chart-specific gradients and obstacle protection remain required planning inputs. The present MAP seed does not solve early/late go-arounds, every airborne authority boundary or full controller recovery. References: [FAA path terminators](https://www.faa.gov/documentLibrary/media/Advisory_Circular/AC%2090-96A%20CHG%201.pdf) and [FAA AIM arrival procedures](https://www.faa.gov/Air_traffic/publications/atpubs/aim_html/chap5_section_4.html). Full runtime generated flights, GA VFR, continuous replenishment and native rendering/audio/FPS acceptance remain incomplete. No simulator was launched or binaries installed for this batch.

Nine generated flight/controller tests pass after adding recovery-boundary identity and radio cleanup. A flight entering its registered fallback discards an old instruction, pending readback or completed-but-unapplied handoff, including while paused. Another flight's grants remain intact. A separate missed-path test reports going around once on the tuned frequency without a grant or frequency change. Worker publication also synchronises the recovery state before publishing it. These are engine/source checks; they do not complete controller rejoining, actual protection or native traffic acceptance. The earlier locked Linux release build and four HTTP full-flight replays also pass before these final radio changes.

Additional movement checks reject an infeasible speed drop inside a fallback, reject that same fallback during airborne rerouting without replacing the current plan, prevent an instantaneous slower hold departure and retain active airborne acceleration/braking settings. The complete focused suites now pass 56 traffic-core tests (four installed-data tests excluded) and 30 generated tests (five installed-data tests excluded).

The final locked Linux plugin/engine release build and all four HTTP full-flight replay arrangements pass after the recovery radio and speed-validation changes. The installed-data missed/hold fixture was run separately as described above. No binaries were installed.

## Generated airborne clearance spans — 17 October 2026

An explicit airborne span now connects controller readback acceptance to bounded movement authority. Core tests cross multiple short legs in one update, stop at the stated limit via the registered fallback, reject missing recoveries and invalid phases, withdraw a narrowed authority and invalidate it after a simulator clock jump. A complete engine flight reaches parking with one controller transmission/readback for three covered cruise legs; changed controller or phase limits reject before installation. A separate test runs the actual simulation worker and verifies that clock-change cleanup survives publication rather than restoring old grants from engine state. This does not populate the production flight factory or establish native traffic acceptance.

The span integration also covers the explicit initial-climb limit carried by the takeoff instruction. The same complete-flight test now exercises both a three-leg initial SID and a three-leg cruise scope: the SID needs no repeated climb instruction after its takeoff readback, while cruise receives one instruction/readback. A movement test queues the SID authority before the roll and verifies the aircraft remains stopped until the separate runway permission succeeds. These tests retain all intermediate recoveries and later landing/taxi permissions.

Final focused verification for the airborne/initial-SID scope changes passes 58 traffic-core tests and 32 generated tests; four and five installed-data tests respectively remain excluded from those ordinary runs. The locked Linux plugin/engine release build and all four HTTP-engine IFR replay arrangements pass. These are source and player-flight replay results, not proof that the production generated-flight factory or native traffic acceptance is complete. No binaries were installed.


### Bidirectional route integration — 17 October 2026

Focused checks pass for IAS/discrete-level convergence, bank-limited course intercepts and actual virtual endpoints, bounded prescribed long-way reversals, small direct holding entries, typed named-fix missed-level amendments, same-file lower/upper controller sectors with scenery override priority, horizontal/vertical controller gaps, and queued-flight isolation from surface readbacks. The installed Nice–Milan/return fixture now constructs two complete nominal routes and both controller/recovery contracts. Its movement acceptance remains failing/in progress: after ground progression it exposed initial-SID scopes exceeding the full forecast budget; staged scopes at existing published fixes are under test. An accepting protection callback in that geometry/controller fixture is explicitly separate from real production scenery release. No new native visibility, audio, FPS, Windows or macOS acceptance is claimed.


### Integrated return flights and long-clearance repair — 18 October 2026

The installed-data two-A319 Nice–Milan/return controller-runtime fixture passes through parking in both directions (75.48 seconds). Its explicit accepting geometry callbacks remain separate from the production scenery gate. Focused tests pass for intermediate clearance duration/pressure-level limits without loss of the original crossing, early/late holding exits with bounded braking and an extra circuit, great-circle interior sector gaps, dateline transfer/gap handling, same-frequency receiver identity, readback/check-in order and stale-route refusal.

The four-aircraft airline/regional controller/runtime fixture now passes (231.91 seconds), checking every required phase through parking. Planned stand footprints and the actual grounded-player observation prevent later taxi routes from crossing committed parking. The remaining Malpensa AT72 block was a crossing endpoint beyond the pavement but short of the far holding line plus tail clearance. Generated crossing selection now uses actual aircraft dimensions and holding-line protection; a focused regression checks both directions, one-way restrictions and the specifically cleared runway. The installed Nice parking regression uses a reported aircraft wingspan and verifies avoidance of the committed footprint. Unknown spans retain their conservative footprint. These geometry/controller fixtures use explicit accepting protection callbacks and do not establish production scenery release or native simulator acceptance.

The larger 19-request production preparation initially lost its retained plans after repeated blocked parking searches expired the weather snapshot (60.98 seconds). Prefiltering occupied stand goals and reusing identical failed landing searches reduced the next run to 44.87 seconds and retained five nominal plans; none passed the unresolved scenery gate. The subsequent controller check exposed an overall movement-count limit caused by unmerged taxi-in samples. Taxi-in now uses the same bounded, equal-speed, resource-preserving merging as taxi-out. The larger installed-data test passes (48.04 seconds), retaining five nominal plans and checking their controller contracts while releasing none through the scenery gate. The four-flight controller/runtime fixture also passes again (231.46 seconds). No weather-age, pavement, ownership, runway or production obstacle checks have been relaxed.

Fresh simulator observations are now shared with the preparation worker through a bounded observation map. Planning keeps its original weather signature and accepts a newer observation only when every relevant input is identical; airport identity, coordinate, pressure, temperature, variation and wind changes reject. Missing observations and reports older than 45 seconds reject. Later directions read the current report before starting their own planning. Focused weather-signature and actual weather-ingress checks pass, including rejection without overwriting the last valid sample. The existing planned-airport weather endpoint test passes. The installed-data worker/publisher test passes (45.58 seconds including setup), retaining seven nominal flights from both origins while receiving repeated synthetic observations through the normal weather endpoint. Worker preparation itself takes 22.07 seconds. None passes production protection. This does not establish native simulator acceptance.

The locked Linux plugin/engine release build passes after the weather-observation integration (19.59 seconds); this build precedes the later direction-start observation refresh. No binaries were installed. The latest crossing-tail regression passes with all-runway endpoint protection. Generated traffic remains disabled by default.

The heading-to-intercept regression now distinguishes prescribed heading from course in crosswind, retains the heading across the joint height/speed solve and verifies the actual post-turn intercept, airspeed and independent protection rejection. The local projection is corrected at the solved intercept latitude so ground-track drift does not leave the solve perpetually unsettled. Wind-vector tests cover multiple headings and wind directions, plus invalid and reverse-motion cases. The four-aircraft installed controller/runtime test passes again after the heading-leg change (232.85 seconds).

Retained wind metadata now reaches the generated aircraft frame: body heading accounts for wind while ground track remains available to traffic reporting and TCAS. Onward suffix construction remaps that metadata. A full synthetic controller-to-parking regression passes with crosswind, checks airborne body/ground-track separation and ground orientation, and rejects wind metadata attached to a ground movement before installation. Renderer appearance and aircraft-bank behavior still require native checks. Focused suites and the release build are being refreshed after this integration.

The refreshed traffic suites pass 73 core tests (four installed-data tests excluded) and 64 generated-engine tests (ten excluded). The locked Linux plugin/engine release build passes in 20.01 seconds after wind metadata/body orientation integration. These results precede the following multi-airport lookup refactor: runtime airport registration now reaches surface ownership and weather requests/validation, while actual source route selection still uses the original airport pair. Its focused third-airport surface/weather regression passes (0.04 seconds). The synthetic prepared-scene test also merges a third airport, preserving unique aircraft identities, model selection, initial ownership and the requirement for fresh movement grants (0.14 seconds). These surface fixtures do not establish wider airborne production or GA VFR. No simulator was launched and no binaries were installed.

### Additional-airport source integration — 18 October 2026

The real worker/publisher now has separate discovery and route-preparation stages. Initial expanded acceptance failed (72.82 seconds): nearby LIMZ/LFTH candidates had incompatible terminal or surface paths and only the original pair retained nominal flights. Discovery now screens actual published connections in both directions and searches at most 16 candidates, retaining at most four airports by default. The next installed worker test passes (118.71 seconds including setup; 94.98 seconds from preparation start), retaining five nominal flights from at least three origins while receiving synthetic observations through the normal weather endpoint. None passes production scenery protection.

A stronger rerun now checks all retained nominal flights through protected-geometry/controller/recovery construction using explicit accepting callbacks confined to that test branch. Its production preparation branch still uses the real rejecting scenery gate. That stronger rerun exposed a late missed-approach join failure on a Milan departure. Arrival selection now checks the touchdown coordinate against the first missed leg before retaining that candidate. The following run passes for seven nominal flights from LFML, LFMN and LIMC (203.16 seconds including setup; 179.93 seconds for the timed preparation and contract-check section). None is released through real scenery protection. A separate allocation invariant check passes for the shared 63-slot limit, compatible categories, preserved requested counts when possible and a visible unsupported-cargo warning. New policy bounds reject airport/candidate over-allocation and non-finite discovery radius. Native visibility, audio and FPS remain untested; no simulator was launched or binaries installed.

Late-entry queries now share the same first-leg projection check used by final recovery construction, including the dateline. A focused test rejects backward, beyond-first-leg, lateral, malformed-coordinate and excessive-tolerance joins without exposing unprotected movement samples. It passes. Further preparation changes retain exact committed stand resource cells across directions and pairs, prefilter conflicting departure stands/routes, cache both successful and failed identical landing searches, and construct the late MAP variant before earlier recovery variants. Departure radio now requires a released complete flight for installed-source actors; the ground-exchange fixture verifies that an unready aircraft receives no pushback exchange or movement grant.

The focused generated suite passes 66 tests (ten installed-data tests excluded) after these changes. Updated installed-data worker/controller and four-aircraft controller-to-parking runs, core/configuration suites and the Linux release build are in progress. These remain separate from unresolved production obstacle release and native acceptance.

### Multi-airport parking and timing — 18 October 2026

The full seven-flight movement run exposed two taxi-in deadlocks: later stand assignments obstructed earlier arrival taxi routes. Planning now retains committed arrival corridor cells across flights and airport pairs, excluding those cells from later parking footprints and initial departure stands. Routes may still share taxiways; occupied stands retain their full protection. The optimized rerun passes all seven flights through taxi-out, takeoff, climb, cruise, descent, approach, rollout, taxi-in and parking across LFML, LFMN and LIMC (39.02 seconds including setup; 14.90 seconds in movement). The other three source actors remain stationary because they lack released complete plans. A separate installed Nice arrival/stand regression passes (0.23 seconds), including the parking-only corridor exclusion.

A read-only stack trace identified repeated sampled-leg duration sums inside separation forecasts. Compiled routes now retain immutable per-segment moving durations, including suffix slicing; dwell and nominal-scope timing remain unchanged. All 21 focused motion tests pass, including exact sampled-sum, suffix and dwell checks. The generated source suite passes 66 tests with two ignored and installed tests filtered. Earlier current core traffic, shipped configuration and locked Linux plugin/engine release reruns also pass (74 core tests, four ignored; two configuration tests; 21.56-second release build). Those broader results precede the latest parking change.

All seven movement plans use explicitly accepting geometry/protection callbacks confined to the test branch. The actual production preparation branch still releases zero flights while scenery obstacles remain unresolved. These results do not establish native simulator visibility, audio, frame rate or Windows/macOS acceptance. No simulator was launched and no binaries were installed.

### Unfiled discovery, short connections and ramp preferences — 18 October 2026

The actual worker can now discover nearby installed controlled airports with blank player departure/destination/runway fields. The installed-data run completes three nominal flights from LFML, LFMN and LIMC through the controller/movement loop to parking (26.78 seconds including setup; 3.14 seconds for movement). Seven incomplete actors remain stationary and the player's route remains blank. Earlier failures exposed a departure runway without a connected taxi route and omitted exact initial stand footprints. Runway retries now consider bounded same-flow departure/arrival combinations; cross-flow or wind-incompatible alternatives are excluded. Scene merging carries exact initial and future parked cells, plus committed departure/arrival transit cells. A refreshed filed-route run completes all seven flights to parking (35.16 seconds including setup; 13.72 seconds for movement).

Coincident SID/STAR boundaries can now produce no extra enroute crossing, including when no crossing budget remains. The point still needs known unchanged MORA, both atmosphere/ceiling checks and the same strict course, level, turn and speed/weather merge used by the combined trajectory. Focused regressions cover a route without a cruise leg, incompatible levels/speeds/coordinates, unknown MORA and no partial mutation after failure. The generated suite passes 69 tests with 12 ignored; the subsequent installed seven-flight run passes in 35.04 seconds. A locked Linux plugin/engine release build passes in 12.91 seconds before the ramp metadata changes below.

Ramp parsing normalizes and deduplicates operating airline codes, while older serialized stands retain an empty preference list. Moving-aircraft selection excludes miscellaneous starts, retains `none` operation ramps as eligible, prefers operator/service/size matches and keeps hangars behind ordinary ramps. Two core regressions pass. The installed Nice arrival test passes in 0.25 seconds, including operator preference selection among independently connected stands and existing corridor/occupancy checks. The full seven-flight network run passes again after integrating these preferences (40.30 seconds including setup; 18.32 seconds for movement). The synthetic GA demo stand now uses its appropriate tie-down type.

All full-flight results above use explicit accepting protection callbacks confined to their geometry/controller test branch. The production preparation branch still releases zero aircraft because physical obstacle support remains incomplete. No native simulator, audio, visibility, FPS or Windows/macOS acceptance is claimed. Clippy was attempted: the existing `dialogue.rs` let-and-return error blocks a clean workspace result; it was not changed as part of this traffic work.

### Cargo and GA integration limits — 18 October 2026

Heading-to-altitude procedure legs now retain their prescribed heading with wind drift, rather than treating heading as ground course. A real Memphis-to-Louisville terminal candidate check passes. Manual-terminated procedure legs still require a controller amendment and remain unsupported by the automatic route join.

The broader installed-data cargo/GA lifecycle check currently fails: it does not produce complete flights for every required category. The initial Memphis surface check exposed passenger, cargo and GA stands outside apt.dat pavement. The verified draped apron sources and published-route alternatives described below now address that ground-routing failure. Pavement and aircraft-fit checks remain enforced. Production obstacle release, continuous traffic replenishment and GA VFR remain unfinished; generated traffic stays disabled by default.

The final source refresh passes 68 generated-traffic tests (two ignored; installed-data tests excluded) and five focused resolved-procedure tests, including heading-to-altitude wind drift. The locked Linux plugin and companion-engine release build also passes. These checks do not supersede the failing installed cargo/GA and Memphis ground-route tests above. No new native simulator or platform acceptance is claimed.

Draped scenery polygons retain their actual rings, holes and coordinate layout for physical surface verification. Verified airport-owned asphalt/concrete now reaches the shared departure/arrival airport loader; the broader mixed flight lifecycle still remains incomplete.

The installed draped-surface diagnostic also retains airport-ID ownership. Surface definitions must agree across every library variant; focused checks reject missing, oversized or nonphysical alternatives. Scenery exclusions and ownership still need to be applied before supplementary pavement can grant any route.

Sixteen scenery checks now pass together, including installed Nice terrain/stack checks, Memphis-owned physical polygon resources, bounded physical-surface variants and airport-ID comments. A separate airport selection regression confirms that the selected apt.dat source path is retained for ownership checks. These remain source checks; supplementary routing pavement is not yet integrated.

### Physical apron integration checkpoint — 18 October 2026

The generated airport loader now retains verified, airport-owned draped pavement as well as apt.dat pavement. It preserves holes and curve controls and conservatively rejects surfaces affected by higher-priority polygon exclusions. The Memphis installed-data coverage check improves from zero to 271 covered stand centers out of 279. At the initial apron checkpoint, connector/turn checks still refused complete passenger, cargo and GA routes; the later published-route alternative check below resolves that demonstrated blockage. Scenery coverage now follows runway, taxi-node and parking extents across up to four tiles with an aircraft/connector margin. Ownership, exclusion and bounded-input regressions pass. Generated traffic remains disabled by default.

After shared-loader integration, 73 focused generated-traffic tests and 13 airport scenery tests pass; four airport tests and two generated tests remain ignored in those focused runs. The locked Linux plugin/engine release build passes. The installed mixed cargo/GA network test still fails with zero complete nominal flights. Remaining constraints include individual stand connectors, runway entry/crossing, turn and stopping envelopes, late missed-approach joins and manual-terminated arrivals.

The seven-flight LFML/LFMN/LIMC controller/runtime fixture passes again after physical apron loading (37.09 seconds total; 14.84 seconds movement). Production protection still releases zero; incomplete actors remain stationary. The separate cargo/GA fixture remains failing.

Memphis departure routing now passes the installed passenger/cargo/GA holding-point check: 19 B738, 53 B752 and 10 C172 stands have verified routes to runway 36L. When a sampled connector leaves physical pavement, the planner can try up to four alternatives through existing published non-runway edges. Ambiguous matches and runway links are never removed, and every alternative retains the original geometry and clearance checks. Individual unsuitable stands still reject. This resolves the demonstrated ground-route blockage; full mixed flight lifecycle and production/native acceptance remain incomplete.

After published-route retries, 74 focused generated tests pass and the locked Linux plugin/engine release build passes. The mixed installed-data fixture advances to sixteen terminal candidates. The MORKK altitude descriptor is now supported; complete flights still refuse turn, stopping and late-recovery constraints alongside individual stand/runway restrictions. It does not yet pass the mixed lifecycle acceptance check.

FACF altitude code J now preserves its published minimum independently of its glide-slope intercept reference. Different values no longer cause an unsupported-descriptor refusal or become an invented altitude window. Pressure references remain mandatory, and final approach geometry and crossing restrictions still apply.

For a missed approach beginning with a course/heading-to-altitude leg, a late runway recovery now gets a separate complete draft from its actual position and height. It preserves the published termination altitude and hold, retains the original MAP identity and still requires independent protection. Fixed legs retain their existing forward-join guard. The seven-flight source fixture continues to pass; the broader mixed test now retains two nominal flights but still fails origin/category coverage and production scenery release.

Same-flow runway selection now considers coverage of the requested traffic categories before complete nominal count, while retaining its existing bounded search and all weather/geometry checks. It no longer stops with only airline/regional drafts when cargo or GA was also requested. Unknown or ambiguous operator categories do not count toward coverage. The mixed installed-data test still fails reverse/third-origin and category acceptance.

Latest source refresh passes 76 focused generated-traffic tests and the locked Linux plugin/engine release build. The seven-flight installed-data controller/runtime fixture passes again across LFML/LFMN/LIMC (38.81 seconds total; 14.79 seconds movement). The full mixed cargo/GA fixture still fails origin/category acceptance with only two nominal KMEM flights and zero production releases. None of these results establishes native simulator acceptance.

Directional traffic discovery now checks published routes and fleet eligibility independently for each direction. Two-way coverage has priority in both airport selection and category allocation; eligible one-way alternatives serve categories without that coverage. Regressions cover odd-count splitting, no invented return, weather-driven eligibility fallback and the global aircraft limit. The refreshed generated suite passes78 tests, and the locked Linux plugin/engine build passes. The seven-flight installed-data controller/runtime fixture passes across LFML/LFMN/LIMC again (39.99seconds total;14.83seconds movement). Earlier attempts exposed and corrected one-way candidates displacing working coverage. Production protection still releases zero flights; mixed cargo/GA and native acceptance remain incomplete.

The final directional refresh passes78 generated tests and the locked Linux plugin/engine release build (13.45seconds). The mixed cargo/GA fixture still fails (29.11seconds), retaining two nominal KMEM flights and zero production releases. Bounded alternate-runway diagnostics now retain later stand failures too: the cargo B752 on the longer Louisville17R runway fails airborne turn geometry, while the C172 still rejects the high-altitude STAR. These remain open planning failures; no stopping, turn, terrain or clearance guard was relaxed.

Manual-termination arrivals now plan as explicit amendments. Core decoding keeps the computable legs, refuses a terminator that is not last, and reports the transition as provenance; the strict decoder still refuses these routes, so nothing authorises them silently. Routing prefers fully published arrivals and offers amended ones only where no published route reaches the approach entry. The controller issues and reads back the amendment, and the prepared flight is refused unless its approach clearance carries the directive and the flown path reaches the cleared target.

Evidence: core resolved suite passes 7 focused tests including the new manual-termination coverage; the generated suite passes 79 with 3 ignored; core library passes 189 with 5 ignored. Installed-data checks against the user's navigation data: the cargo terminal candidate check passes, and a new check confirms reverse KSDF–KMEM arrivals into 36L now produce manual-terminated candidates where none existed before. Memphis publishes 61 STARs, 25 of them ending in a manual transition. Runways 34L/34R/35L remain refused because they have no supported approach or recovery, which is a separate gap. One Nice–Milan installed-data check fails identically before and after this change.

Coincident departure/arrival joins now compare intended indicated speed rather than requiring identical airspeed objects, which embedded each airport's own weather sample. Two airports sharing a pressure setting was effectively required before, so short routes could never connect. A real speed change at the same point still refuses without an upstream deceleration plan.

Draped-polygon ground detection now also requires that a polygon carries no vertical geometry. A facade or wall shares the DRAPED_POLYGON header with an apron, so header-only detection treated walls as flat ground an aircraft could fly over; they now refuse as obstacles. Evidence: library suite 4 passed including a new regression covering v_line, v_facade and v_triangle while LINE_PAINT remains ground; generated suite 81 passed/3 ignored; all four installed scenery checks pass; core library 189 passed/5 ignored; locked plugin+engine release build passes.

Found during this work: the uncommitted arrival-descent preference change in the current working tree makes the installed Nice departure-route fixture fail its amended-missed-level assertion, and it did not resolve the cargo 17R turn-radius failure it was investigating. It passes again when that single call is removed. This is unresolved and needs a decision before further integration work.

The arrival-descent preference was measured rather than assumed. With it, the mixed cargo/GA installed fixture produces two complete nominal flights; without it, none. It is retained for that capability. The cost is that installed_nice_scene_has_verified_departure_routes fails its amended-missed-level coverage assertion, most likely because better-planned arrival levels mean that airport no longer needs a missed-level amendment. Both anchoring variants behave identically, so this is inherent to the change rather than an anchor defect. The assertion has not been weakened; the amended-missed path remains covered by the assembly unit test. This needs a review decision rather than an edit to the test.

The full workspace test suite passes: cargo test --locked --release reports 514 passing tests across 12 binaries with no failures, and cargo build --locked --release succeeds for every target. Two pre-existing clippy errors were fixed in passing (a complex map type in the audio crate and a let-and-return in dialogue). Clippy remains not clean: --all-targets still surfaces roughly 48 further pre-existing errors and a large pedantic warning backlog across scenery DSF parsing, crew, LLM, routes, radio and test code. None of those are in code added here, and clearing them is left as separate cleanup rather than folded into this work.

Simulator bindings are now defined in a single table and dispatched through a total action mapping, so a binding cannot be registered without a handler and then silently do nothing when pressed. Two regressions cover that, plus the fact that the two radio transmit bindings share one action while staying distinct from the pilot talk binding. Plugin suite passes 36 with 2 ignored; the workspace suite passes 516.

The engine supervisor's timing policy is now testable and tested. Grace period after a disconnect, growing backoff between launches, and the timer resetting on a reconnect are covered by three new regressions, which required splitting the decision from engine-binary resolution. Plugin suite passes 39 with 2 ignored; workspace suite passes 519.

Obstacle resolution was measured rather than assumed, and three real defects were found and fixed. Forest definitions in X-Plane 12 are mesh-built trees with no tree-height command, exceed the previous 4 MB read limit, and separate fields with tabs; heights now come from the vertical span of the tree geometry. That reduced unresolved forest placements in the Memphis cargo fixture from 18,738 to zero, with real definitions resolving to plausible heights. The OBJ8 loader no longer refuses objects for material and export directives that cannot move geometry (about nine thousand placements), and the per-tile distinct-resource cap rose from 256 to 4096 while the 32 MB geometry budget still bounds memory. Road elevation was settled against real scenery: across the Andes at 5,285 m terrain the published value stays near zero, so it is height above the draped surface, confirmed by the pool scale of 65535 with -32768 offset. Workspace suite 530 passed, 0 failed; all installed scenery and landing checks pass.

Production release remains zero. The remaining cause is a deliberate policy that marks any tile carrying objects as unresolved, asserted by an installed test, which no airport tile can satisfy. Changing it requires replacing that assertion with a stricter boundary rather than removing it, and that work is in progress.

Production release now succeeds. The real obstacle gate releases flights rather than only the test fixtures doing so: the mixed cargo/GA scene releases 1 of its 2 complete nominal flights, and the Nice/Milan seven-flight scene releases 1 of 7. Every remaining refusal comes from a checked intersection or an unresolved scenery class, not from a blanket rule.

Four corrections achieved that. The blanket rule marking any tile with object placements as unresolved was replaced by the precise contract that a tile resolves when all its placements resolve and stays unresolved when any fails; the installed assertion was strengthened rather than removed, and unknown retention remains covered by a separate unit test. Network chains are now compared against the base mesh rather than an overlay's empty one. Facade and forest heights, which are published above ground, were being added to the terrain twice in the vertical anchor. Forests are tested against their true polygon, with holes clear and uncertain geometry still refusing, instead of their bounding box.

Evidence: workspace 536 passed, 0 failed; installed scenery 4/4 and landing 3/3 pass; seven-flight PASS with 1 released; bidirectional and unfiled-player PASS; mixed cargo/GA releases 1 with its remaining failure being the pre-existing origin/category assertion. Two items flagged for review: the tile-cache guard rose from 256 MiB to 512 MiB (eighteen legitimate corridor stacks exceeded the old bound; still fail-closed), and full corridor protection now runs to completion so seven-flight preparation takes about 160 s against a 180 s fixture deadline.

## Local-server defaults and setup documentation — 18 October 2026

Fresh plugin settings use http://127.0.0.1:8099 for language, recognition and synthesis, enable AI understanding with the bundled Qwen2.5 model, enable controller/copilot/pilot speech and copilot radio readbacks/tuning, and enable heard reports from observed traffic. Existing explicit settings survive loading and serialization; autopilot target control, unsolicited/social chat and experimental generated traffic remain opt-in. Settings regressions pass10 tests, UI tests pass45 with1 ignored, and the non-installed release engine suite passes216 with8 ignored/19 filtered. The no-chatter advisory test explicitly disables chatter instead of relying on the retired default. The locked Linux plugin/engine release build passes. Native fresh-install audio, NVIDIA DLL discovery and Windows/macOS runtime checks remain separate.

README validation confirms all7 registered simulator commands documented and all prior detailed paragraphs retained after whitespace/bullet normalization, apart from the intentionally corrected own-pilot speech default. Setup and NVIDIA runtime download guidance now precede the feature catalogue; long implementation history remains expandable. Markdown fences/disclosures and local documentation link targets were checked.

## GA departure runway split — 21 October 2026

The generated scene now builds departure plans for both same-flow runways. A general-aviation flight uses the runway nearest its own stand, while the other categories keep the scored primary runway. This removes the KSDF GA Apron conflict where the 17R entry sat inside 11/29's protected strip: the GA C172/CRJ7 now depart 17L and complete.

The mixed cargo/GA fixture prepares 13 aircraft and 126 terminal candidates (was 12 and 60) with 3 complete nominal flights and 2 released (was 1). It still fails the first origins check: every complete flight departs KMEM, so KSDF is missing as an origin. A full-length departure retry removes the KSDF departure refusals (B2/GA/UPS); the reverse flights now fail at the KMEM arrival instead: B2 terminal level or missed-approach hold, UPS Ramp 11 repeated runway crossing, UPS Ramp 7 36C vacate. The small-field SID gap is a later blocker and is not reached.

Empty-root unit scenes now receive synthetic runway threshold elevations, so the demo model/stand test still prepares flights without simulator data. Workspace 593 passed, 0 failed; clippy reports 0 errors under the documented command; installed routing 4/4, scenery 4/4, landing 3/3, and the seven-flight, bidirectional and unfiled lifecycles pass. No native simulator acceptance is claimed.

### Complete runway-exit and parking selection — 22 October 2026

A generated arrival can retain a later verified runway exit when the first exit cannot reach free parking. The bounded search checks up to eight complete exit-to-stand candidates, retains all stopping/pavement/aircraft-clearance/resource checks, and compiles flare, rollout and taxi-in together before retaining a candidate. The new two-exit regression first proves that the initial exit is a safe dead end, then exercises actual arrival preparation through the later connected exit. Reserving the sole stand still rejects the arrival.

The focused generated suite passes127 tests (three ignored); the locked Linux plugin/engine release build passes. The installed mixed cargo/GA fixture still fails after64.02seconds: four nominal flights, all fromKMEM, and zero production releases. Reverse missed-approach/hold metadata, clearance-duration feasibility and connected GA parking remain blockers. Physical-obstacle and unresolved-facade protection still refuse nominal routes. Other traffic source changes were present during this run, so this is a current snapshot rather than a controlled before/after comparison. Native visibility, audio, FPS and Windows/macOS acceptance remain separate.

### Explicit level on the repeated holding fix — 22 October 2026

The recovery regression now also tests a blank-altitude HM record immediately following an explicit exact/minimum altitude at the same identifier, region, section and subsection. Actual recovery preparation retains that level and named fix; denying protection still refuses the draft, and source records remain unchanged. Different fix/region, a maximum-only condition and missing altitude or pressure reference reject. All127 focused generated tests and the locked Linux plugin/engine release build pass after the change.

The installed Memphis I36L input uses this repeated-fix pattern. Its explicit5000-foot level is corroborated by the [FAA ILS or LOC36L chart for01–29October2026](https://aeronav.faa.gov/d-tpp/2610/00253IL36L.PDF). This is evidence for retaining the explicit same-fix restriction, not a rule for guessing altitude from unrelated earlier legs. Full mixed-route and native acceptance remain separate.

With the repeated-fix level support, the mixed installed-data rerun retains seven complete nominal drafts from both KMEM and KSDF (76.41seconds). It passes the two-primary-origin check, then fails the required third-origin check before category or controller/movement acceptance. Production protection releases zero flights because of physical-obstacle intersections and unresolved facades. This is progress in candidate planning, not seven completed simulator flights. Discovery coverage, valid cargo/operator pairs, some connected stands and clearance/holding cases remain open.

### Opening cast, approach placement and taxi releases — 24 October 2026

The opening cast now fills the configured moving target before the first publication. Local departures supply the hold queue and mid-taxi aircraft, and taxi-in arrivals fill the remainder from their threshold-to-parking route. The installed LFMN probe publishes 12 flights with 3 mid-taxi, 1 taxi-in, 2 at the hold queue and 3 arrivals on final; the first three finals are placed at 8.0, 13.0 and 18.0 NM from the player.

Approach seeding measures distance along the published approach path and searches from the runway backwards, so a procedure turn yields the inbound crossing rather than the outbound leg. Previously only the first arrival was close; later slots used whatever long STAR segment was selected and appeared at 17–35 NM.

Generated IFR requests select the same clearances service as the pilot request. X-Plane publishes Delivery as `Clearance`; a service test limited to the literal `Delivery` sent generated aircraft to Ground. The installed Nice–Milan fixture logs `requests IFR clearance from NICE Delivery on 121780 kHz` and `CLNC DEL on 120900 kHz`.

Ground conflicts now distinguish following traffic from a genuine block. Aircraft sharing a taxiway are cleared up to the cell before the leader instead of holding at the gate; a held aircraft is released with “continue” as soon as the protected path is free, without a second pilot request. A unit regression holds a follower behind a leader, releases it with `continue`, and confirms the shared cell remains owned by the leader. Runway crossings still require their own clearance.

Evidence: generated suite 139 passed, 2 failed (the two pre-existing scenery fixtures), 23 ignored; core library 228 passed, 6 ignored; settings 13 passed; UI 48 passed, 1 ignored; installed Nice departure-route fixture passed; installed mixed bidirectional lifecycle passed. Installed engine `e24a58cc…`, plugin `3142e7a2…`; backup `OpenATC-9rn84q`. Native simulator visibility and audio acceptance remain separate.

### Opening-cast merge, TCAS slot churn and log volume — 25 October 2026

The 24 October runtime log showed a 12-flight opening scene published after preparation, followed by `3 additional flights admitted; 5 aircraft retained`. The prepared cast was correct; the merge path discarded most of it. It applied per-category limits and the 20 NM refill distance to the opening cast, and private registrations that did not match an airline profile were treated as having no category at all. The cast also inherited two plan-less local-pattern frames that consumed slots before the first planned flight was admitted.

The first complete scene now replaces the plan-less seed outright instead of merging into it. Incremental admission only accepts aircraft at segment zero, so a cast containing mid-taxi, holding and taxi-in aircraft was silently reduced to the parked ones even after the category filters were bypassed. Unmatched generated callsigns count as general aviation instead of being dropped. A regression builds a four-aircraft cast, a live scene with two local-pattern frames and zero category caps, and asserts that all four planned aircraft survive publication. The installed population fixture pre-seeds two local frames, merges the opening cast, and now retains all 12 planned aircraft.

The same log showed the plugin acquiring and releasing the simulator's traffic slots every 0.5 s for more than a minute: `override_TCAS` 1, then 0, then `releasing control of AI aircraft`. Local and advisory identities used `0x0E00_0000` and `0x0E10_0000`, above the 24-bit Mode S range, so a scene containing only those frames could never fill a TCAS slot. Their ranges moved to `0x00D0_0000` and `0x00D1_0000`; the plugin also refuses to acquire slots when no frame has an addressable identity. Plugin regression covers the no-addressable-frames case.

Ground-speed instrumentation was writing a `MOTION` line for every speed change in release builds, producing a 6.6 MB log in a short session. It is now opt-in through `OPENATC_MOTION_DEBUG`.

Evidence: generated suite 140 passed, 2 failed (the two pre-existing scenery fixtures), 23 ignored; core library 228 passed, 6 ignored; plugin 49 passed, 2 ignored; installed Nice departure-route fixture, installed mixed bidirectional lifecycle and installed population fixture passed. Installed engine `300a0110…`, plugin `bf222173…`; backup `OpenATC-vj6Sf7`.

### Generated traffic uses the pilot's station resolver — 25 October 2026

Generated exchanges now select controllers through `stations::station_for_intent`, the same capability mapping the pilot uses. It prefers the aircraft's tuned frequency when that station serves the intent, otherwise the nearest published station, and falls back to the airport's published frequencies through `serves`. Combined duties such as Ground on a Tower station are resolved by `has_role`; the previous literal `service == "Ground"` lookup could miss them.

Clearance now treats `Delivery` as a clearance service. The index's inferred Ground or Tower clearance fallback is used only when no Delivery is published, so a real Delivery always wins. Contacting Ground while a Delivery existed was the defect that started this thread. Tower and Ground handoffs keep using the same station object for the aircraft's COM1, the player's receiving radio, the controller voice key and the transcript. Every exchange logs `callsign on station (role) kHz`, so the next log states whether an instruction came from Ground, Tower, Delivery or Approach.

Evidence: core library 232 passed including four station-selection regressions; generated suite 144 passed, 2 failed (the two pre-existing scenery fixtures), 23 ignored; plugin 49 passed, 2 ignored; installed Nice departure-route fixture and installed mixed bidirectional lifecycle passed. Installed engine `02f4a63d…`, plugin `d9dbbf24…`; backup `OpenATC-fpq9Yq`.

### Tower departure priority over the Ground taxi queue — 25 October 2026

The 01:23 runtime log showed `EZY8431` correctly handed from `NICE Ground` to `NICE Tower` on 118.700, followed by Tower's ready acknowledgement, then a stream of Ground taxi clearances to other aircraft. The Tower departure waited at the hold while the Ground queue kept winning the controller loop. Active aircraft already talking to Tower are now checked before ordinary Ground taxi readbacks, so a line-up or takeoff clearance cannot be starved by a busy surface queue. The initial “hold short, stand by” remains the ready acknowledgement; the movement clearance follows on the next controller cycle.

Installed engine `d4ba9615…`, plugin `d9dbbf24…`; backup `OpenATC-DwwfYM`.

### Takeoff waits are no longer silent — 25 October 2026

The AFR3501 sequence showed Tower's ready acknowledgement, then installation of the onward flight, then no further Tower exchange. The takeoff path had a silent `continue` when `world.authorise` refused the line-up segment, so the aircraft sat at the hold without a reason in the transcript or log. A refused line-up or takeoff now returns a Tower hold exchange and logs the blocking resource owners and wake state:

`AFR3501 takeoff held at segment 0 (LineUp): owners [...], wake false`

The next run will identify whether the blocker is a landing aircraft on the same runway, an aircraft still owning a runway resource from the other runway, or wake separation. Active Tower traffic also remains ahead of the ordinary Ground taxi queue.

Installed engine `88d6e0cb…`, plugin `d9dbbf24…`; backup `OpenATC-FxfzcU`.

### Takeoff denial was a surface-cell conflict, not the approach — 25 October 2026

The AFR6424 log resolved the denial: `takeoff held at segment 0 (LineUp): owners [("GROUND/LFMN/-1/1", 15728643), ("GROUND/LFMN/-2/1", 15728643)]`. The owners were the departure's own surface cells, not the arriving aircraft. The approach was not conflicting with the departure runway. The remaining block came from the external cell set, which includes the stationary player and observed native traffic.

A stationary player more than 0.05 NM from the holding point no longer blocks a line-up through the coarse surface grid; moving players and observed traffic still block. The hold log now also records the blocked resource names, observed ground targets within 0.2 NM, the player's distance to the hold and the wake state. A line-up allowed past the stationary player is logged explicitly.

Installed engine `27c4e0d7…`, plugin `d9dbbf24…`; backup `OpenATC-hZkpBu`.

### Exact runway width and wing footprint — 25 October 2026

The take-off denial now uses the published runway dimensions instead of the 50 m ground grid. `traffic::runway_clearance` combines the apt.dat runway width and threshold-to-threshold length with `traffic::footprint`, which applies the aircraft length, reported wingspan and heading. A wingtip is over the runway when the centreline offset is less than `width/2 + lateral half-extent`.

The player's size comes from the ICAO profile and the simulator's reported wingspan, with a 36 m fallback instead of the previous 80 m default. The coarse player cells are removed from a line-up only when the aircraft is clear of the runway strip and is not on a taxiway or stand. A wing over the runway, a taxiway conflict or a moving observer still holds the departure. Logs record runway, offset, lateral extent, half-width and span.

Regression: a 45 m runway with a 36 m span at 30 m offset is over the runway; at 55 m offset it is clear. Evidence: generated suite 144 passed, 2 failed (the two pre-existing scenery fixtures), 23 ignored; core traffic suite 108 passed, 5 ignored; installed Nice and mixed bidirectional lifecycles passed. Installed engine `3cf722b0…`, plugin `f942b9a0…`; backup `OpenATC-8gun6h`.

### Tower answers the ready call immediately — 25 October 2026

The latest EZY341 log showed the aircraft correctly on `NICE Tower 118.700`, but the first Tower exchange was still the generic “hold short, stand by”. The flight was installed on the following cycle, so the actual line-up or take-off clearance was delayed behind the acknowledgment. That handshake is now removed when the clearance can be issued: Tower answers “ready for departure” with the line-up/take-off instruction, or a reason-specific hold, in the same exchange. The generic standby remains only as a fallback when installation cannot complete.

The exact runway-width and wing-footprint check remains in the line-up path; the player is allowed past the coarse cells only when the wing footprint is clear of the published runway strip and the aircraft is not on a taxiway or stand. Installed engine `0e8b3c92…`, plugin `f942b9a0…`; backup `OpenATC-4uPndj`.

### Clear runway means takeoff, not standby — 25 October 2026

Tower no longer answers a ready call with a generic hold when the runway is available. `takeoff_exchange_for` probes both the line-up and take-off movement, then issues the planned take-off contract directly from the hold. If the runway is occupied or wake-protected, the normal line-up/hold path remains. The direct path logs `cleared for takeoff directly from the hold on runway ...`.

Wake is leader-based. The engine previously used a 240-second fallback for an unknown or unsupported pair, which could hold a heavier follower behind a lighter leader. Unknown pairs now add no interval, and a missing runway position adds no interval. Regression: `B738` followed by `B77W` needs zero, while `B77W` followed by `B738` keeps the 120-second full-length interval.

Phrase audit: the only hardcoded generated-traffic speech left was the held-taxi “continue” pair. It moved to `generated_taxi_continue` and `generated_taxi_continue_readback` in `speech/runtime/responses.toml`; take-off, line-up, hold and wake wording continue to use the existing TOML entries.

Installed engine `5b8a4e2b…`, plugin `097c7e5b…`; backup `OpenATC-Vg6D9w`.

### Editable regional wake policy and independent runways — 25 October 2026

`traffic/wake.toml` is now the source of wake and runway-relationship data. It contains FAA CWT, ICAO Doc 4444, EUROCONTROL RECAT-EU and Australian MATS tables, leader/follower category matrices, departure intervals, arrival spacing, LUAW limits and per-region parallel-runway thresholds. The **Wake separation** setting selects `auto`, `icao`, `recat_eu`, `faa_cwt` or `mats`; `auto` resolves by airport prefix with ICAO as the fallback. The plugin reports the simulator's wake category and wingspan; the policy uses that first, then the aircraft profile's `cwt` or `wake`.

A landing on one runway no longer blocks a departure on the other. Runway occupancy and wake checks use the exact runway resource, and airborne separation ignores targets flying parallel to a different runway more than 250 m from the departure centreline. Same-runway traffic and projected crossing paths still block. LFMN 04L/04R are independent for surface operations.

A wake interval now produces **line up and wait**, not hold short. Take-off clearance is issued directly from the hold when the runway is clear and no wake interval applies. A heavy behind a heavy gets LUAW followed by take-off; a heavier follower behind a lighter leader adds no interval. Runway clearances name the flight's own SID and initial level, with no latitude, longitude or full hold procedure in the take-off clearance. The hold remains in the clearance data for the controller and is issued separately only if needed.

Evidence: core 234 passed; engine 311 passed, 2 failed (the two pre-existing scenery fixtures), 33 ignored; settings 13 passed; UI 48 passed, 1 ignored; plugin 49 passed, 2 ignored; installed Nice and mixed bidirectional lifecycles passed. Installed engine `be537cfd…`, plugin `ba7659c9…`; backup `OpenATC-eS93EF`.

### Runway queue and initial-climb authority, not runway width — 25 October 2026

The EZY1021 and EZY824 transcripts ended in `cancel takeoff clearance` immediately after a direct takeoff clearance from the hold. In both logs the player was about 0.13 NM from the holding point and clear of the runway; LFMN 04R is 45.11 m wide and the player was roughly 150 m from its centreline. Runway width was not the cause.

Two separate faults produced the same symptom.

The first was the initial-climb authority. A take-off clearance issued from the line-up carries an airborne scope beginning at route segment 2, with the take-off roll at segment 1. `World::authorise_airborne_until` only accepted a scope beginning at the current segment or the next one, so the readback or the next conflict check could not record the climb. The take-off grant was inserted and then withdrawn, which the controller announced as a cancellation. The motion world now accepts the intervening take-off segment as part of the same clearance, grants the line-up exit and take-off roll with it, and a regression test confirms the approved take-off order survives the readback and the next conflict check.

The second was queue ownership. Ordering complete taxi-out aircraft by distance to the threshold let a later arrival on a nearer parallel taxiway jump ahead of an aircraft already waiting. The queue now keeps aircraft that reached the holding point first, gives a lined-up or rolling aircraft priority, and uses distance only as a tie-break. A `lineup_ahead` gate applies before installation, before a line-up clearance and before a direct take-off clearance. Take-off cancellations now log `{callsign} takeoff cancelled: crossing …, airborne …` so the cause is attributable.

A third fault held 04R departures for traffic established on the 04L final. The final-approach corridor widens with distance, so an arrival on the parallel runway eventually fell inside the 04R corridor. Line-up and take-off protection now ignores airborne targets aligned with a different runway when they are more than 250 m from the departure centreline. A generated arrival is also matched to its landing runway, so a 04L arrival is ignored even while turning onto final. Initial-climb clearances inherit the take-off runway, because their own segment carries no runway resource. Surface occupancy still counts, and traffic on the same runway's final still blocks. Runway cells built from external targets and taxi crossing holds use the same rule. With the Nice runway centrelines about 309 m apart, the 04L final no longer blocks a 04R departure or crossing.

Hold logs now name the crossing and airborne results and report a changed hold reason. Landing clearances withdrawn by a conflict are logged separately. This makes a stuck queue attributable to a runway, an airborne forecast or the movement world rather than a generic traffic message.

### Line-up and departure pacing — 25 October 2026

A distant final-approach aircraft no longer prevents a departure from lining up. The line-up check now uses the time needed to enter the runway; a short final still holds the departure at the line, but a distant one allows `line up and wait`. The final-approach conflict is then applied to the take-off clearance after the aircraft is lined up. A lined-up aircraft that is still waiting does not receive repeated `hold position` calls. It hears from Tower when it is cleared or when an issued take-off clearance is actually cancelled.

A low aircraft on short final to either parallel runway now blocks the take-off before the clearance is issued. The EZY3931 cancellation occurred when EZY7000 went around from short final at 1.7 NM and 690 ft, after the clearance had been issued. The runway-relationship filter no longer ignores a target below 2,000 ft AGL within 3 NM of a runway, so the departure waits instead of being cleared and then cancelled.

A stopped taxi follower now resumes when the aircraft ahead releases the shared cell. It still needs current or next-segment approval from its own taxi clearance; the automatic resume cannot bypass a readback or a hold. This keeps the queue rolling instead of waiting for the leader to stop and for another Ground call.

The Ground-to-Tower handoff now starts while the aircraft is still taxiing, within 550 m of the hold. The hold itself is then a ready/line-up exchange rather than a handoff, ready call and clearance serialized after the aircraft stops.

A stopped active flight whose current clearance is a runway crossing now has that current-segment clearance reissued when the runway is clear. Previously the controller only looked at the next segment, so an aircraft that lost or consumed its crossing grant could sit at the line indefinitely. The reissue path is restricted to the current crossing and still checks the runway before transmitting.

The blunt 60-second removal watchdog was removed. It removed a legitimate flight in the installed mixed-lifecycle fixture. A phase-aware session health and rogue-aircraft escalation is still required before alpha3.

A generated pose with no frame, flight plan, apron assignment or active-flight assignment is now removed on the next movement tick and its resources are released. This prevents an orphan world aircraft from retaining a runway or taxi cell after its controller lifecycle has gone.

Take-off and runway-crossing checks now use the time the movement needs to clear the runway instead of the fixed 5 NM final radius. A distant arrival that cannot reach the threshold before the departure is airborne does not block the take-off. The fixed envelope remains in use for landing clearances.

The take-off roll no longer targets the recovery speed while the initial climb clearance is pending. This removes the visible accelerate-brake-accelerate cycle while Tower waits for final traffic. Visual interpolation now uses the source simulator clock and a monotonic playhead instead of HTTP arrival time, so a late or early snapshot cannot make the aircraft surge and slow down. Wall-clock interpolation remains the fallback for older servers.

The player's microphone now blocks only a controller on the frequency in use. A busy Tower frequency no longer stops a Ground or Delivery exchange, and a pending player readback no longer holds every generated controller. Generated exchanges continue back-to-back instead of waiting for the player's radio, the copilot queue or a movement simulation job to finish.

Tests: core 238 passed; engine 317 passed, 2 failed (the two pre-existing scenery fixtures); release generated suite 152 passed, same 2 failures; settings 13 passed; UI 48 passed, 1 ignored; plugin 50 passed, 2 ignored; installed Nice and mixed bidirectional lifecycles passed.

Installed engine `88db5cec…`, plugin `a93ebafc…`; backup `OpenATC-JPB8Nz`.
