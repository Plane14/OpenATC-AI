use super::*;
struct Crew<'a> {
    engine: &'a Engine,
    model: &'a Model,
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
        self.model.propose(json!([{"control":control,"value":value}]));
        let v = self.request(role, text)?;
        *self.model.proposal.lock().unwrap() = None;
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
        *self.model.proposal.lock().unwrap() = None;
        if !serde_json::from_str::<Value>(&self.model.answer.lock().unwrap()).is_ok_and(|answer| answer.get("actions").is_some()) {
            let profile: openatc_core::crew::CrewProfile = serde_json::from_value(self.profile.clone())?;
            if let Some(actions) = openatc_core::crew::parse_many(&profile, role, text) {
                self.model.propose(json!(actions.into_iter().map(|(control,value)| json!({"control":control,"value":value})).collect::<Vec<_>>()));
            }
        }
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
    e.settings(json!({"aiEnabled":false}))?;
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
        model: m,
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
    let v = c.request("ground", "remove external power and chocks")?;
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
    m.set(json!({"reply":"Cabin lights off. I will get the passengers ready for departure."}));
    e.settings(json!({"aiEnabled":true,"aiModel":"test","aiUrl":m.url()}))?;
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
    m.set(json!({"reply":"Cabin lights off. I will bring you a coffee."}));
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
    m.set(json!({"reply":"I will keep an eye on the passengers and prepare the cabin."}));
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
    m.set(json!({"reply":"Cabin lights off. I will prepare the cabin for landing."}));
    let landing = c.action(
        "cabin",
        "prepare cabin for landing and turn off lights please",
        "cabin_brightness",
        0.,
    )?;
    assert!(latest(&landing["state"]).to_lowercase().contains("landing"));
    e.settings(json!({"aiEnabled":false}))?;
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
    e.settings(json!({"aiEnabled":true}))?;
    m.set(json!({"reply":"Slides armed and cross-checked. I will bring you coffee."}));
    for text in [
        "get passengers ready, can i have a coffee and can you cross check and arm doors please",
        "corss chek and arm dorrs please and coffee please",
        "can i please have a coffe and can you cross check and arm doors",
        "arm and cross check, can you bring me a quick coffee before we push back?",
    ] {
        m.propose(json!([{"control":"slides","value":1}]));
        let request = c.request("cabin", text)?;
        let tasks = request["state"]["crewActions"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["control"], "slides");
        let done = c.ack(&tasks[0], true)?;
        assert_eq!(count(&done["state"]), count(&request["state"]) + 1);
        let text = latest(&done["state"]).to_lowercase();
        assert!(text.contains("armed") && text.contains("cross-check") && text.contains("coffee"));
    }
    e.settings(json!({"aiEnabled":false}))?;
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
    assert!(m.requests.lock().unwrap().iter().any(|request|request.to_string().contains("have a beer for me")));
    m.set(json!({"reply":"External power disconnected and chocks removed, Captain. Ha, thanks! Enjoy your flight."}));
    m.propose(json!([{"control":"external_power","value":0},{"control":"chocks","value":0}]));
    let banter = c.request("ground", "disoconnect external power and chocks please kind sir, and have a beer for me after that! you deserve it haha")?;
    accepted(&banter);
    let tasks = banter["state"]["crewActions"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|task| task["value"] == 0.0));
    let requests_before_ack = m.requests.lock().unwrap().len();
    c.ack(&tasks[0], true)?;
    let done = c.ack(&tasks[1], true)?;
    assert_eq!(m.requests.lock().unwrap().len(), requests_before_ack);
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
    m.set(json!({"answers":["I'll check for spare lobster and bring a coffee.","The passengers are settled and the crew is cheerful."]}));
    *m.proposal.lock().unwrap() = Some(json!({"conversation_reply":"","actions":[{"control":"cabin_brightness","value":50}],"cross_check":false,"memory":"","conversation_points":["Can I have spare lobster and coffee?","How are the passengers and crew?"]}).to_string());
    let mixed = c.request("cabin", "Can I have spare lobster and coffee? Dim the cabin lights to 50%. How are the passengers and crew?")?;
    accepted(&mixed);
    let task = mixed["state"]["crewActions"][0].clone();
    assert_eq!(task["control"], "cabin_brightness");
    assert_eq!(task["value"], 50.0);
    let before_ack = m.count();
    let done = c.ack(&task, true)?;
    assert_eq!(m.count(), before_ack);
    let spoken = latest(&done["state"]);
    assert!(spoken.contains("50 percent") && spoken.contains("lobster") && spoken.contains("crew is cheerful"));
    m.set(json!({"reply":"I will bring you coffee and something to eat. Here is a quick joke for you.","actions":[],"cross_check":false,"memory":"Captain requested coffee and food."}));
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
    m.propose(json!([{"control":"slides","value":1}]));
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
    assert!(classifier.contains("misspellings") && classifier.contains("reordered words"));
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

/// Exercise the installed model through an isolated engine and acknowledged control fixtures.
pub(super) fn live_actors(e: &mut Engine) -> Result {
    let url = std::env::var("OPENATC_LIVE_AI_URL")?;
    let model = std::env::var("OPENATC_LIVE_AI_MODEL").unwrap_or_else(|_| "qwen2.5-7b-instruct-q4_k_m".into());
    let mut passenger_plan = plan();
    passenger_plan["passengers"] = json!(170);
    e.post("plan", passenger_plan)?;
    e.settings(json!({"aiEnabled":true,"aiUrl":url,"aiModel":model,"llmPhraseVariety":true}))?;
    e.tune(118100,json!({"onGround":true,"positionValid":true,"groundSpeedKnots":0}))?;
    let catalogue: toml::Value = toml::from_str(&fs::read_to_string(repo().join("aircraft/toliss_a320.toml"))?)?;
    let profile = serde_json::to_value(&catalogue["aircraft"]["crew"])?;
    let available = profile["controls"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    let values = Arc::new(Mutex::new(json!({"external_power":1,"chocks":1,"slides":0,"cabin_brightness":100,"heading":0,"beacon":0})));
    e.post("crew/observe",json!({"aircraft":"ToLiss neo","profile":profile,"values":values.lock().unwrap().clone(),"available":available}))?;
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let samples = values.clone();
    let origin = e.url.clone();
    let observer = thread::spawn(move || {
        let client = Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
        while !stop.load(Ordering::SeqCst) {
            let sample = samples.lock().unwrap().clone();
            let _ = client.post(format!("{origin}/crew/observe")).json(&json!({"aircraft":"ToLiss neo","profile":null,"values":sample,"available":available})).send();
            thread::sleep(Duration::from_millis(200));
        }
    });
    let outcome = (|| -> Result {
        let cases = [
            ("ground", "Hey buddy, long time no see! How is your family? Your kids must be getting tall. Can you disoconnect external power and remove those chocks, please? I'll be back in town soon and we should catch up over coffee. And tell me a joke while we wait!", json!({"external_power":0.0,"chocks":0.0})),
            ("ground", "Ha, thanks! Is your family still enjoying living here? I'll be back in town next week, perhaps we can all catch up then.", json!({})),
            ("cabin", "How are you and your family? I really enjoyed catching up last time. I'll be back in town soon! While we chat, can you arm the dorrs and corss chek them, turn off the cabin lights, and see if there is a spare sandwich for me? Also, do you have a good joke?", json!({"slides":1.0,"cabin_brightness":0.0})),
            ("cabin", "I will be back in town soon and would love to catch up with your family again. Can I have a sandwich, and is there any spare lobster from first class? Maybe a meat pie too, a coffee and a snack. How are the passengers? Somewhere between the food and chatter, could you dim the cabin lights to 50%, please? Is the rest of the crew happy? And give me a good joke while we wait!", json!({"cabin_brightness":50.0})),
            ("cabin", "That sounds lovely! What about the joke I asked for? And how are the passengers settling in?", json!({})),
            ("copilot", "Mate, how is your family? We should catch up when I'm back in town. Somewhere in all this chatter, could you set heading two seven five and switch the beacon on, please? And give me your best terrible joke!", json!({"heading":275.0,"beacon":1.0})),
            ("copilot", "Ha, very good. Remind me what heading I asked you to set? And would you like me to ask the attendant for sandwiches for both of us?", json!({})),
            ("ground", "Let's catch up next Thursday at the Harbour Cafe. My dog's name is Miso. Just chatting, leave all the aircraft controls as they are. Tell me a different joke about our catch-up, not the one you told earlier.", json!({})),
            ("ground", "What is my dog's name, and when and where did we agree to catch up? No need to change any controls.", json!({})),
            ("cabin", "There is a football team celebrating their win in the cabin. My favourite snack is a meat pie with tomato sauce. Please keep those details in mind, leave the lights and doors alone, and tell me a fresh joke about the food rather than repeating your earlier joke.", json!({})),
            ("cabin", "What snack did I say was my favourite, and who is celebrating in the cabin? Could you check whether there is a spare plate of first-class caviar for us too? Leave the controls alone.", json!({})),
            ("copilot", "My sister's name is Nora and we are meeting in Hobart after this trip. Remember that for our chat. Don't adjust the heading or beacon. Tell me a different joke about our trip, rather than the one you told earlier.", json!({})),
            ("copilot", "What is my sister's name, and where are we meeting? What heading did you set earlier? Just tell me; do not change it again.", json!({})),
        ];
        let mut report = Vec::new();
        let mut failures = 0;
        for (role, text, expected) in cases {
            if std::env::var("OPENATC_LIVE_ACTOR_ROLE").is_ok_and(|selected| selected != role) { continue; }
            let started = Instant::now();
            let mut response = e.post("request",json!({"role":role,"intent":"conversation","text":text}))?;
            let actions = response["state"]["crewActions"].as_array().unwrap().clone();
            let actual = actions.iter().map(|action|(action["control"].as_str().unwrap().to_owned(),action["value"].clone())).collect::<serde_json::Map<_,_>>();
            let passed = Value::Object(actual.clone()) == expected && response["result"]["accepted"] == true;
            if !passed { failures += 1; }
            for action in &actions {
                values.lock().unwrap()[action["control"].as_str().unwrap()] = action["value"].clone();
                response = e.post("crew/ack",json!({"sequence":action["sequence"],"aircraft":action["aircraft"],"success":true,"detail":"verified fixture control value"}))?;
            }
            let reply = latest(&response["state"]);
            println!("{role}: actions={actual:?}; reply={reply}");
            report.push(json!({"role":role,"request":text,"expected":expected,"actions":actual,"passed":passed,"actionChecksPassed":passed,"conversationReview":"Pending manual review: coverage, continuity, repeated jokes, unsupported claims and control wording", "reply":reply,"seconds":started.elapsed().as_secs_f64()}));
        }
        let folder = repo().join(".ai/actor-validation");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("report.json"),serde_json::to_string_pretty(&report)?)?;
        fs::copy(e.folder.path().join("engine.log"),folder.join("engine.log"))?;
        if failures > 0 { return Err(format!("{failures} live actor action cases failed; inspect .ai/actor-validation/report.json").into()); }
        Ok(())
    })();
    stopped.store(true,Ordering::SeqCst);
    observer.join().unwrap();
    outcome
}
