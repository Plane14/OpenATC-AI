use super::*;
struct Crew<'a> {
    engine: &'a Engine,
    profile: Value,
    values: Value,
}
impl Crew<'_> {
    fn observe(&self, aircraft: &str) -> Result<Value> {
        self.engine.post("crew/observe",json!({"aircraft":aircraft,"profile":self.profile,"values":self.values,"available":self.profile["controls"].as_object().unwrap().keys().collect::<Vec<_>>()}))
    }
    fn request(&self, role: &str, text: &str) -> Result<Value> {
        self.observe("ToLiss neo")?;
        self.engine.post(
            "request",
            json!({"role":role,"intent":"conversation","text":text}),
        )
    }
    fn ack(&self, task: &Value, success: bool) -> Result<Value> {
        self.engine.post("crew/ack",json!({"sequence":task["sequence"],"aircraft":task["aircraft"],"success":success,"detail":"test simulator confirmation"}))
    }
    fn action(&mut self, role: &str, text: &str, control: &str, value: f64) -> Result<Value> {
        let v = self.request(role, text)?;
        assert_eq!(v["result"]["pending"], true, "{v}");
        let tasks = v["state"]["crewActions"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["control"], control);
        assert_eq!(tasks[0]["value"].as_f64().unwrap(), value);
        let done = self.ack(&tasks[0], true)?;
        assert_eq!(done["accepted"], true);
        let state = self.engine.state()?;
        let last = state["transcript"].as_array().unwrap().last().unwrap();
        assert!(["COPILOT", "ATTENDANT", "GROUND"].contains(&last["speaker"].as_str().unwrap()));
        assert_eq!(self.ack(&tasks[0], true)?["accepted"], false);
        assert_eq!(count(&self.engine.state()?), count(&state));
        self.values[control] = json!(value);
        Ok(done)
    }
    fn denied(&self, role: &str, text: &str) -> Result {
        let v = self.request(role, text)?;
        rejected(&v);
        assert!(
            v["state"]["crewActions"].as_array().unwrap().is_empty(),
            "{v}"
        );
        Ok(())
    }
}
pub(super) fn crew(e: &mut Engine, m: &Model) -> Result {
    let profile: toml::Value = toml::from_str(&fs::read_to_string(
        repo().join("aircraft/toliss_a320.toml"),
    )?)?;
    let profile = serde_json::to_value(&profile["aircraft"]["crew"])?;
    e.tune(
        118100,
        json!({"latitude":17.974,"longitude":102.57,"groundSpeedKnots":0}),
    )?;
    let mut c = Crew {
        engine: e,
        profile,
        values: json!({"heading":0,"altitude":5000,"gear":1,"beacon":0,"seatbelts":0,"slides":0,"cabin_brightness":100,"chocks":0}),
    };
    for (role, text, control, value) in [
        ("copilot", "set heading 270", "heading", 270.),
        ("copilot", "set heading 360", "heading", 0.),
        (
            "copilot",
            "set altitude flight level 240",
            "altitude",
            24000.,
        ),
        ("ground", "add chocks", "chocks", 1.),
        ("ground", "remove chocks", "chocks", 0.),
        ("ground", "Remove chalks.", "chocks", 0.),
    ] {
        c.action(role, text, control, value)?;
    }
    let v = c.request("ground", "remove ext pwr and chocks")?;
    let tasks = v["state"]["crewActions"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|t| t["value"].as_f64() == Some(0.)));
    assert!(
        tasks.iter().any(|t| t["control"] == "chocks")
            && tasks.iter().any(|t| t["control"] == "external_power")
    );
    for (i, task) in tasks.iter().enumerate() {
        assert_eq!(
            c.ack(task, true)?["state"]["crewActions"]
                .as_array()
                .unwrap()
                .len(),
            1 - i
        );
        if i == 0 {
            assert_eq!(
                count(&c.engine.state()?),
                count(&v["state"]),
                "Ground spoke before the whole request completed"
            );
        }
    }
    let done = c.engine.state()?;
    assert_eq!(count(&done), count(&v["state"]) + 1);
    let text = latest(&done).to_lowercase();
    assert!(text.contains("chocks") && text.contains("power") && text.contains(" and "));
    c.denied("ground", "remove chocks and connect imaginary thing")?;
    c.action(
        "cabin",
        "cabin brightness 50 percent",
        "cabin_brightness",
        50.,
    )?;
    let combined = c.action(
        "cabin",
        "how are the passengers and get you turn off cabin lights and get ready for depature?",
        "cabin_brightness",
        0.,
    )?;
    let response = latest(&combined["state"]).to_lowercase();
    assert!(
        response.contains("cabin") && response.contains("departure"),
        "{response}"
    );
    assert_eq!(
        combined["state"]["transcript"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["speaker"],
        "ATTENDANT"
    );
    for text in [
        "turn off cabin lights, can i have a coffee",
        "can i have a coffee and turn cabin lights off please",
        "turn off cabin ligths and can i have a cofee",
        "how are the passengers, turn off cabin lights, can i have a coffee",
    ] {
        let done = c.action("cabin", text, "cabin_brightness", 0.)?;
        let reply = latest(&done["state"]).to_lowercase();
        assert!(
            reply.contains("coffee") && reply.contains("lights"),
            "{reply}"
        );
    }
    for text in [
        "prepare cabin for landing",
        "I have turned off the seat belt sign, it won't be for long so keep an eye on the passengers please.",
    ] {
        let service = c.request("cabin", text)?;
        accepted(&service);
        assert!(
            service["state"]["crewActions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            service["state"]["transcript"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["speaker"],
            "ATTENDANT"
        );
    }
    let landing = c.action(
        "cabin",
        "prepare cabin for landing and turn off lights please",
        "cabin_brightness",
        0.,
    )?;
    assert!(latest(&landing["state"]).to_lowercase().contains("landing"));
    let pending = c.request("cabin", "turn on cabin lights")?;
    let task = pending["state"]["crewActions"][0].clone();
    let watch = c.request("cabin", "keep an eye on the passengers please")?;
    accepted(&watch);
    assert_eq!(
        watch["state"]["crewActions"][0]["sequence"],
        task["sequence"]
    );
    c.ack(&task, true)?;
    let armed = c.action("cabin", "arm slides and crosscheck", "slides", 1.)?;
    assert!(
        latest(&armed["state"])
            .to_lowercase()
            .contains("armed and cross-checked")
    );
    let query = c.request("cabin", "crosscheck")?;
    accepted(&query);
    assert!(query["state"]["crewActions"].as_array().unwrap().is_empty());
    for text in [
        "get passengers ready, can i have a coffee and can you cross check and arm doors please",
        "corss chek and arm dorrs please and coffee please",
        "can i please have a coffe and can you cross check and arm doors",
        "arm and cross check, can you bring me a quick coffee before we push back?",
    ] {
        let request = c.request("cabin", text)?;
        let tasks = request["state"]["crewActions"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["control"], "slides");
        let done = c.ack(&tasks[0], true)?;
        assert_eq!(count(&done["state"]), count(&request["state"]) + 1);
        let text = latest(&done["state"]).to_lowercase();
        assert!(text.contains("armed") && text.contains("cross-check") && text.contains("coffee"));
    }
    c.action("cabin", "doors to manual", "slides", 0.)?;
    for text in ["is beacon on?", "don't turn beacon on"] {
        assert!(
            c.request("copilot", text)?["state"]["crewActions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    for (role, text) in [
        ("copilot", "gear up"),
        ("copilot", "set heading 999"),
        ("cabin", "set heading 270"),
    ] {
        c.denied(role, text)?;
    }
    for (role, speaker) in [
        ("ground", "GROUND"),
        ("cabin", "ATTENDANT"),
        ("copilot", "COPILOT"),
    ] {
        let farewell = c.request(role, "have a good day done there sir")?;
        accepted(&farewell);
        let state = &farewell["state"];
        assert!(state["crewActions"].as_array().unwrap().is_empty());
        let text = latest(state).to_lowercase();
        assert!(
            !text.contains("confirm") && !text.contains("approval"),
            "{text}"
        );
        assert_eq!(
            state["transcript"].as_array().unwrap().last().unwrap()["speaker"],
            speaker
        );
    }
    let started = c.request("copilot", "read before start checklist")?;
    contains(&started, "Seat belt");
    rejected(&c.request("copilot", "on")?);
    c.values["seatbelts"] = json!(1);
    contains(&c.request("copilot", "on")?, "Beacon");
    c.values["beacon"] = json!(1);
    contains(&c.request("copilot", "set")?, "complete");
    c.values["seatbelts"] = json!(0);
    c.values["beacon"] = json!(0);
    let start = c.request("copilot", "perform before start checklist")?;
    let task = &start["state"]["crewActions"][0];
    assert_eq!(task["control"], "seatbelts");
    let next = c.ack(task, true)?;
    let task = &next["state"]["crewActions"][0];
    assert_eq!(task["control"], "beacon");
    let failed = c.ack(task, false)?;
    assert!(
        failed["state"]["crewActions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(latest(&failed["state"]).to_lowercase().contains("not"));
    let button = c.profile["controls"]
        .as_object()
        .unwrap()
        .values()
        .find(|v| v["momentary"].as_bool() == Some(true))
        .unwrap();
    let text = format!(
        "please {}",
        button["label"].as_str().unwrap().to_lowercase()
    );
    let v = c.request("copilot", &text)?;
    let task = &v["state"]["crewActions"][0];
    assert!(
        latest(&c.ack(task, true)?["state"])
            .to_lowercase()
            .contains("button pressed")
    );
    let hello = c.request("cabin", "How are you?")?;
    accepted(&hello);
    assert!(
        !hello["result"]["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("landing")
    );
    c.action("ground", "start pushback", "pushback_start", 1.)?;
    for text in [
        "don't start pushback",
        "stop pushback",
        "can you start pushback?",
    ] {
        assert!(
            c.request("ground", text)?["state"]["crewActions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    c.action(
        "ground",
        "request pushback 80 meters 60 degrees right",
        "pushback_start",
        1.,
    )?;
    c.denied("ground", "connect high pressure air")?;
    m.set(json!({"reply":"Chocks removed. Ha, coffee first! Have a good flight."}));
    e.settings(json!({"aiEnabled":true,"aiModel":"test","aiUrl":m.url()}))?;
    let narrated = c.action(
        "ground",
        "remove chocks. have a beer for me haha",
        "chocks",
        0.,
    )?;
    assert!(latest(&narrated["state"]).contains("coffee first"));
    assert!(m.prompt().contains("confirmed result"));
    assert!(m.prompt().contains("have a beer for me"));
    m.set(json!({"reply":"External power disconnected and chocks removed, Captain. Ha, thanks! Enjoy your flight."}));
    let banter = c.request("ground", "disoconnect external power and chocks please kind sir, and have a beer for me after that! you deserve it haha")?;
    accepted(&banter);
    let tasks = banter["state"]["crewActions"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|task| task["value"] == 0.0));
    c.ack(&tasks[0], true)?;
    let done = c.ack(&tasks[1], true)?;
    assert!(latest(&done["state"]).contains("Ha, thanks"));
    assert!(
        m.requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.to_string().contains("you deserve it haha"))
    );
    for (role, request, control, value, response) in [
        (
            "cabin",
            "turn off cabin lights and have a beer for me",
            "cabin_brightness",
            0.,
            "Cabin lights off. Ha, coffee will do for now!",
        ),
        (
            "copilot",
            "set heading 270 and have a beer for me",
            "heading",
            270.,
            "Heading 270 set. Ha, after we finish!",
        ),
    ] {
        m.set(json!({"reply":response}));
        let done = c.action(role, request, control, value)?;
        assert!(latest(&done["state"]).contains("Ha,"));
    }
    m.set(json!({"actions":[],"services":["coffee","snack"],"cross_check":false}));
    let refreshments = c.request("cabin", "do you have time to get me a coffee and sandwich? wehave alittle time before push back. also do you know a good joke?")?;
    accepted(&refreshments);
    assert!(
        refreshments["state"]["crewActions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let requests = m.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.to_string().contains("wehave alittle time"))
    );
    drop(requests);
    let reply = latest(&refreshments["state"]).to_lowercase();
    assert!(
        reply.contains("coffee") && (reply.contains("snack") || reply.contains("something to eat")),
        "{reply}"
    );
    m.set(json!({"reply":"Slides could not be confirmed, but we're in the clear for arming."}));
    let doors = c.request("cabin", "arm doors")?;
    let denied = c.ack(&doors["state"]["crewActions"][0], false)?;
    let spoken = latest(&denied["state"]).to_lowercase();
    assert!(!spoken.contains("clear for arming"));
    assert!(spoken.contains("not") || spoken.contains("unable"));
    m.set(json!({"reply":"They seem comfortable, Captain. It's calm back here."}));
    let mood = c.request("cabin", "how are the passengers? are they happy?")?;
    accepted(&mood);
    assert!(latest(&mood["state"]).contains("comfortable"));
    m.set(json!({"actions":[{"control":"heading","value":275}]}));
    e.settings(json!({"aiEnabled":true,"aiModel":"test","aiUrl":m.url()}))?;
    c.action(
        "copilot",
        "please choose heading two seven five",
        "heading",
        275.,
    )?;
    let requests = m.requests.lock().unwrap();
    let classifier = requests
        .iter()
        .rev()
        .find_map(|request| {
            request["messages"][0]["content"]
                .as_str()
                .filter(|prompt| prompt.contains("Allowed controls:"))
        })
        .unwrap();
    assert!(!classifier.contains("Pilot wording examples"));
    assert!(classifier.contains("obvious misspelled"));
    drop(requests);
    m.set(
        json!({"actions":[{"control":"chocks","value":0},{"control":"external_power","value":0}]}),
    );
    let typo = c.request("ground", "remuve choks and exernal pwer")?;
    assert_eq!(typo["state"]["crewActions"].as_array().unwrap().len(), 2);
    let failed = c.ack(&typo["state"]["crewActions"][0], false)?;
    assert_eq!(count(&failed["state"]), count(&typo["state"]));
    assert_eq!(failed["state"]["crewActions"].as_array().unwrap().len(), 1);
    let done = c.ack(&failed["state"]["crewActions"][0], true)?;
    assert_eq!(count(&done["state"]), count(&typo["state"]) + 1);
    let summary = latest(&done["state"]).to_lowercase();
    assert!(
        summary.contains("chocks")
            && summary.contains("confirm")
            && summary.contains("power disconnected")
    );
    for proposal in [
        json!({"actions":[{"control":"heading","value":999}]}),
        json!({"actions":[{"control":"slides","value":1}]}),
        json!({"actions":[{"control":"sim/custom/anything","value":1}]}),
        json!({"actions":[{"control":"heading","value":275}],"services":["coffee"]}),
        json!({"actions":[{"control":"heading","value":275}],"extra_command":"start_engine"}),
    ] {
        m.set(proposal);
        c.denied("copilot", "please choose heading two seven five")?;
    }
    e.settings(json!({"aiEnabled":false}))?;
    let queued = c.request("copilot", "beacon on")?;
    let task = &queued["state"]["crewActions"][0];
    let cleared = e.reset()?;
    assert!(cleared["crewActions"].as_array().unwrap().is_empty());
    assert_eq!(count(&cleared), 0);
    assert_eq!(c.ack(task, true)?["accepted"], false);
    c.observe("different aircraft")?;
    assert!(e.state()?["crewActions"].as_array().unwrap().is_empty());
    e.post("plan", plan())?;
    e.post("telemetry", json!({"latitude":0.003,"longitude":-0.003,"altitudeFeet":0,"onGround":true,"positionValid":true,"radioPower":true,"com1Khz":121800,"groundSpeedKnots":0,"traffic":[]}))?;
    clearance(e)?;
    e.post("telemetry", json!({"latitude":0.003,"longitude":-0.003,"altitudeFeet":0,"onGround":true,"positionValid":true,"radioPower":true,"com1Khz":121900,"groundSpeedKnots":0,"traffic":[]}))?;
    accepted(&e.req("start_pushback")?);
    assert_eq!(e.state()?["phase"], 8);
    c.values["chocks"] = json!(1);
    c.values["external_power"] = json!(1);
    let services = c.request("ground", "disconnect external power and chocks please")?;
    let tasks = services["state"]["crewActions"].as_array().unwrap();
    assert_eq!(tasks.len(), 2, "{services}");
    assert!(
        tasks
            .iter()
            .all(|task| task["role"] == "ground" && task["value"].as_f64() == Some(0.0))
    );
    for task in tasks {
        c.ack(task, true)?;
    }
    let done = e.state()?;
    assert_eq!(done["phase"], 8);
    assert_eq!(
        done["transcript"].as_array().unwrap().last().unwrap()["speaker"],
        "GROUND"
    );
    e.post("telemetry", json!({"latitude":0.003,"longitude":-0.003,"altitudeFeet":0,"onGround":true,"positionValid":true,"radioPower":true,"com1Khz":121900,"groundSpeedKnots":2,"traffic":[]}))?;
    c.denied("ground", "disconnect external power and chocks please")?;
    Ok(())
}
