# Phraseology references and implementation limits

Use the speech TOMLs for wording and the controller/aircraft state for facts and permissions. Examples never authorize movement or establish that an aircraft action completed.

Reviewed primary references:

- [FAA AIM departure procedures](https://www.faa.gov/air_traffic/publications/aim_html/chap5_section_2.html) and [FAA clearance delivery](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap4_section_2.html): published SID restrictions matter to a climb-via instruction. A procedure name alone does not establish those restrictions.
- [UK CAA CAP 413](https://www.caa.co.uk/data-and-publications/publications/documents/content/cap-413/) and [UK SID/STAR phraseology](https://www.caa.co.uk/Commercial-industry/Airspace/Communication-navigation-and-surveillance/SID-and-STAR-phraseology/): regional procedures and wording differ. Do not apply an FAA example indiscriminately worldwide.
- [Airbus slide-deployment guidance](https://flightsafety.airbus.com/2023/03/16/preventing-inadvertent-slide-deployments/): slide procedures separate the command, the action and cross-check confirmation. The crew examples retain that distinction.
- The installed ToLiss A320neo Simulation Manual, sections 3.6 and 4: integrated pushback uses distance and angle; the interactive communication panel separates ground services and cabin services.

Checklist wording is concise challenge-and-response. Exact items, sequence and expected values depend on the aircraft and operator. The shipped aircraft lists are abbreviated examples. A button press is described as a press when no system-state readback exists.

Repeated route or altitude deviations retain the IFR clearance. The optional assistance prompt asks whether the pilot needs help; it does not cancel IFR service after an arbitrary number of warnings. Published SID/STAR altitude and speed leg restrictions are not yet fully modelled.

Ground-crew pushback reference: [ICAO A-CDM implementation attachment 4.5](https://www.icao.int/ru/filebrowser/download/27408?fid=27408), including readiness, brake confirmation, commencement and completion exchanges. OpenATC currently acknowledges the dispatched tug request; it does not claim pushback completed or brakes released without simulator confirmation.

Departure holding-point reference: [FAA AIM 4-3-14](https://www.faa.gov/Air_traffic/publications/atpubs/aim_html/chap4_section_3.html). Ground/local-control transition is part of the departure flow; turbine-powered aircraft may be assumed ready at the runway unless advised otherwise. Every runway crossing requires explicit authorization. [CAA phraseology](https://regulatorylibrary.caa.co.uk/923-2012/Content/SERA%20AMC%20GM/Appendix%201%20to%20AMC1%20SERA%2014001.htm) also includes report-ready and explicit crossing instructions. Arrival at a holding point alone does not authorize entry.

Routine IFR clearance delivery omits the teaching prompt listing readback items. Readback validation remains mandatory in the plugin. Once accepted, a separate published Ground station may be assigned for start-up/pushback when ready; this is a frequency handoff, not movement permission. See [FAA clearance delivery](https://www.faa.gov/Air_traffic/publications/atpubs/atc_html/chap4_section_2.html) and [SERA readback requirements](https://regulatorylibrary.caa.co.uk/923-2012/Content/Regs/01900_SERA8015_Air_traffic_control_clearances.htm). Local responsibility for start-up, pushback and taxi varies.

Runway crossings: installed scenery determines the runway at the actual route limit, separately from the departure runway. Ground and Tower can issue an explicit crossing clearance, including proactively when the aircraft is stopped at an intermediate hold. A correct crossing readback enables only that crossing. Onward taxi remains a separate instruction after the aircraft clears the runway. Yellow runway guard lights continue flashing; red stop bars are the clearance-controlled lights. See [FAA airport lighting guidance](https://www.faa.gov/Air_traffic/publications/atpubs/aim_html/chap2_section_1.html). Native scenery guard lights remain unchanged.

## Wake turbulence

Wake holds use **Hold for wake turbulence** with a runway hold-short instruction. Wording alternatives in `speech/runtime/responses.toml` preserve those facts. Insufficient approach spacing can produce a wake caution while continuing, or a go-around instruction when required. Aircraft-pair minima and runway geometry are decided by Rust; adding a phrase does not implement a separation rule. References: [FAA departure/wake procedures](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap3_section_9.html) and [approach separation](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap5_section_5.html).

## Crossing readback acknowledgement

FAA JO 7110.65 sections 2-4-3 and 2-4-5 require checking pilot acknowledgements/readbacks and limiting transmissions to necessary messages; they do not prescribe a routine “readback correct” reply. The plugin validates a crossing readback and activates its permission silently. Incorrect readbacks still receive correction. After verified crossing completion the controller issues the next taxi clearance, with its own hold-short limit and readback cycle. This is an implementation choice consistent with that guidance, not a claim that controllers must always remain silent.

References: [pilot acknowledgement/readback](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap2_section_4.html), [taxi and ground movement](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap3_section_7.html).

## Traffic, urgency and cabin coordination

Taxi give-way instructions identify observed conflicting traffic and retain the existing runway limit. A traffic-clear message cannot authorize a crossing. Approach speed changes use indicated airspeed and published limits, and stop near final; an imminent runway conflict triggers a go-around instead of a demand to accelerate onto an occupied runway. See [FAA speed adjustments](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap5_section_7.html) and [landing operations](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap3_section_10.html). The limited spacing monitor does not implement complete separation or aircraft performance.

The first response to an emergency/urgency call asks for the nature of the problem and pilot intentions. Fuel, people on board and assistance may be requested later as needed; the opening reply does not routinely demand a new transponder code and every detail at once. See [FAA emergency assistance](https://www.faa.gov/air_traffic/publications/atpubs/atc_html/chap10_section_2.html). Mission choices report fictional cabin circumstances; only an explicit pilot declaration invokes the ATC emergency state.

Cabin reports distinguish the captain's instruction, preparation in progress and confirmed completion. Turbulence preparation uses the available seatbelt observation; an unknown switch does not become an on indication. Friendly service alternatives and repeat-request humour come from TOML and are suppressed during urgent incidents. [FAA cabin coordination guidance](https://www.faa.gov/documentLibrary/media/Advisory_Circular/AC_120-48A.pdf) informs the timing and coordination; simulated passenger stories are not medical or operator procedures.
