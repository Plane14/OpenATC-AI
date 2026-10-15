//! Shared LLM exchange and reply decoding for every actor and request interpreter.
use serde::de::DeserializeOwned;

#[derive(Debug)]
pub enum StructuredError {
    Service(String),
    Invalid(String),
}
impl std::fmt::Display for StructuredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Service(error) | Self::Invalid(error) => f.write_str(error),
        }
    }
}

/// Decode structured output once; the caller still validates role and operational authority.
pub async fn structured<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    temperature: f32,
    system: &str,
    user: &str,
) -> Result<T, StructuredError> {
    structured_format(client, url, model, temperature, system, user, None).await
}

pub async fn structured_format<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    temperature: f32,
    system: &str,
    user: &str,
    schema: Option<serde_json::Value>,
) -> Result<T, StructuredError> {
    structured_history(client, url, model, temperature, system, user, &[], schema).await
}

/// Conversation history keeps user and crew speakers distinct at the provider boundary.
pub async fn structured_history<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    temperature: f32,
    system: &str,
    user: &str,
    history: &[serde_json::Value],
    schema: Option<serde_json::Value>,
) -> Result<T, StructuredError> {
    let content = complete_request(
        client,
        url,
        model,
        temperature,
        system,
        user,
        history,
        true,
        schema,
    )
    .await
    .map_err(StructuredError::Service)?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|error| StructuredError::Invalid(format!("Invalid AI JSON: {error}")))?;
    if !value.is_object() {
        return Err(StructuredError::Invalid(
            "AI reply must be a JSON object".into(),
        ));
    }
    serde_json::from_value(value)
        .map_err(|error| StructuredError::Invalid(format!("Invalid AI reply schema: {error}")))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Narration {
    reply: String,
}

/// Spoken structured output uses the same immersion checks as ordinary conversation.
pub async fn narration(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    temperature: f32,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let result: Narration = structured(client, url, model, temperature, system, user)
        .await
        .map_err(|error| error.to_string())?;
    spoken(&result.reply)?;
    Ok(openatc_core::dialogue::speech_text(&result.reply))
}

pub(crate) fn spoken(reply: &str) -> Result<(), String> {
    let lower = reply.to_lowercase();
    if reply.trim().is_empty()
        || reply.len() > 65536
        || [
            "simulat",
            "plugin",
            "language model",
            "artificial intelligence",
        ]
        .iter()
        .any(|word| lower.contains(word))
    {
        return Err("Invalid spoken AI reply".into());
    }
    Ok(())
}

/// Older compatible servers may omit this capability; new servers count exactly.
pub async fn prompt_fits(
    client: &reqwest::Client,
    url: &str,
    system: &str,
    user: &str,
) -> Option<bool> {
    prompt_fits_history(client, url, system, user, &[]).await
}

pub async fn prompt_fits_history(
    client: &reqwest::Client,
    url: &str,
    system: &str,
    user: &str,
    history: &[serde_json::Value],
) -> Option<bool> {
    let mut request = client.post(format!("{}/v1/tokenize", url.trim_end_matches('/')));
    if let Ok(key) = std::env::var("OPENATC_AI_KEY") {
        request = request.bearer_auth(key);
    }
    let response = request
        .timeout(std::time::Duration::from_secs(5))
        .json(&serde_json::json!({"messages":messages(system,user,history),"max_tokens":512}))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response
        .json::<serde_json::Value>()
        .await
        .ok()?
        .get("fits")?
        .as_bool()
}

/// Format bounded exchanges after the caller selects the appropriate actor history.
pub fn history<'a>(
    exchanges: impl IntoIterator<Item = (&'a str, &'a str)>,
    count: usize,
    user_characters: usize,
    reply_characters: usize,
) -> Vec<serde_json::Value> {
    exchanges
        .into_iter()
        .take(count.min(16))
        .map(|(user, reply)| {
            serde_json::json!({
                "user":user.chars().take(user_characters.min(1000)).collect::<String>(),
                "reply":reply.chars().take(reply_characters.min(1200)).collect::<String>(),
            })
        })
        .collect()
}

fn auth_header(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|key| format!("Bearer {key}"))
}

/// POST JSON to an OpenAI-compatible chat endpoint, returning the body text.
pub async fn text(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    temperature: f32,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let reply = complete_request(
        client,
        url,
        model,
        temperature,
        system,
        user,
        &[],
        false,
        None,
    )
    .await?;
    spoken(&reply)?;
    Ok(openatc_core::dialogue::speech_text(&reply))
}

fn messages(system: &str, user: &str, history: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut messages = vec![serde_json::json!({"role":"system","content":system})];
    messages.extend_from_slice(history);
    messages.push(serde_json::json!({"role":"user","content":user}));
    messages
}

async fn complete_request(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    temperature: f32,
    system: &str,
    user: &str,
    history: &[serde_json::Value],
    structured: bool,
    schema: Option<serde_json::Value>,
) -> Result<String, String> {
    let mut body = serde_json::json!({
        "model": model,
        "temperature": temperature,
        "max_tokens": 512,
        "messages": messages(system,user,history),
    });
    if structured {
        body["response_format"] = match schema {
            Some(schema) => {
                serde_json::json!({"type":"json_schema","json_schema":{"name":"crew_reply","strict":true,"schema":schema}})
            }
            None => serde_json::json!({"type":"json_object"}),
        };
    }
    let mut call = client
        .post(format!("{}/v1/chat/completions", url.trim_end_matches('/')))
        .json(&body);
    if let Some(authorization) = auth_header("OPENATC_AI_KEY") {
        call = call.header("Authorization", authorization);
    }
    let reply = call
        .send()
        .await
        .map_err(|_| "AI endpoint unavailable.".to_owned())?;
    if !reply.status().is_success() {
        return Err("AI endpoint unavailable.".to_owned());
    }
    let parsed: serde_json::Value = reply
        .json()
        .await
        .map_err(|_| "AI endpoint unavailable.".to_owned())?;
    let content = chat_content(&parsed);
    if content.trim().is_empty() {
        return Err("Empty AI reply".into());
    }
    Ok(content)
}

/// Extract the assistant message from a chat completion body.
#[must_use]
pub fn chat_content(parsed: &serde_json::Value) -> String {
    parsed
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Trim quotes and whitespace and limit the reply to 500 characters.
#[must_use]
pub fn clean_reply(mut text: String) -> String {
    let trimmed = text
        .trim_matches(|value: char| value.is_whitespace() || value == '"' || value == '\'')
        .to_owned();
    text = trimmed;
    if text.len() > 500 {
        text = text.chars().take(500).collect();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use serde_json::json;

    #[tokio::test]
    async fn token_budget_and_generation_receive_identical_ordered_chat_messages() {
        use std::sync::{Arc, Mutex};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server = tokio::spawn({
            let seen = requests.clone();
            async move {
                let budget = seen.clone();
                axum::serve(listener,Router::new()
                .route("/v1/tokenize",post(move |Json(body):Json<serde_json::Value>| {
                    budget.lock().unwrap().push(body);
                    async{Json(json!({"fits":true}))}
                }))
                .route("/v1/chat/completions",post(move |Json(body):Json<serde_json::Value>| {
                    seen.lock().unwrap().push(body);
                    async{Json(json!({"choices":[{"message":{"content":"{\"reply\":\"Your sister is Nora.\"}"}}]}))}
                }))).await.unwrap();
            }
        });
        let history = vec![
            json!({"role":"user","content":"My sister is Nora."}),
            json!({"role":"assistant","content":"I'll remember that."}),
        ];
        let client = reqwest::Client::new();
        assert_eq!(
            prompt_fits_history(&client, &origin, "role", "What is her name?", &history).await,
            Some(true)
        );
        let result: Narration = structured_history(
            &client,
            &origin,
            "test",
            0.0,
            "role",
            "What is her name?",
            &history,
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.reply, "Your sister is Nora.");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["messages"], requests[1]["messages"]);
        assert_eq!(requests[0]["max_tokens"], requests[1]["max_tokens"]);
        assert_eq!(requests[0]["messages"][1], history[0]);
        assert_eq!(requests[0]["messages"][2], history[1]);
        assert_eq!(requests[0]["messages"][3]["content"], "What is her name?");
        server.abort();
    }

    async fn model(answer: serde_json::Value) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let app = Router::new().route(
                "/v1/chat/completions",
                post(move || {
                    let answer = answer.clone();
                    async move { Json(answer) }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn shared_decoder_preserves_multi_action_intents_and_rejects_extra_fields() {
        let content = json!({"actions":[{"control":"chocks","value":0},{"control":"external_power","value":0}],"cross_check":false}).to_string();
        let (origin, task) = model(json!({"choices":[{"message":{"content":content}}]})).await;
        let intent: openatc_core::crew::request::Intent = structured(
            &reqwest::Client::new(),
            &origin,
            "test",
            0.0,
            "classify",
            "request",
        )
        .await
        .unwrap();
        assert_eq!(intent.actions.len(), 2);
        assert_eq!(intent.actions[1].control, "external_power");
        task.abort();
        let content = json!({"control_targets":{"chocks":0,"external_power":0,"beacon":null},"cross_check":false,"conversation_points":["How is your family?"]}).to_string();
        let (origin, task) = model(json!({"choices":[{"message":{"content":content}}]})).await;
        let intent: openatc_core::crew::request::Intent = structured(
            &reqwest::Client::new(),
            &origin,
            "test",
            0.0,
            "classify",
            "request",
        )
        .await
        .unwrap();
        assert_eq!(intent.control_targets["chocks"], Some(0.0));
        assert_eq!(intent.control_targets["beacon"], None);
        task.abort();
        let (origin, task) = model(json!({"choices":[{"message":{"content":"{\"reply\":\"Hello\",\"services\":[\"coffee\"]}"}}]})).await;
        assert!(
            structured::<openatc_core::crew::request::Intent>(
                &reqwest::Client::new(),
                &origin,
                "test",
                0.0,
                "classify",
                "request"
            )
            .await
            .is_err()
        );
        task.abort();
        let (origin, task) = model(
            json!({"choices":[{"message":{"content":"{\"reply\":\"Hello\",\"actions\":[{}]}"}}]}),
        )
        .await;
        assert!(
            narration(
                &reqwest::Client::new(),
                &origin,
                "test",
                0.0,
                "reply",
                "request"
            )
            .await
            .is_err()
        );
        task.abort();
    }

    #[tokio::test]
    async fn all_spoken_paths_reject_empty_or_immersion_breaking_replies() {
        for reply in ["", "The simulator reports everything ready."] {
            let (origin, task) = model(json!({"choices":[{"message":{"content":reply}}]})).await;
            assert!(
                text(
                    &reqwest::Client::new(),
                    &origin,
                    "test",
                    0.0,
                    "reply",
                    "request"
                )
                .await
                .is_err()
            );
            task.abort();
            let content = json!({"reply":reply}).to_string();
            let (origin, task) = model(json!({"choices":[{"message":{"content":content}}]})).await;
            assert!(
                narration(
                    &reqwest::Client::new(),
                    &origin,
                    "test",
                    0.0,
                    "reply",
                    "request"
                )
                .await
                .is_err()
            );
            task.abort();
        }
    }

    #[tokio::test]
    async fn plain_and_structured_replies_share_the_reserved_output_budget() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/v1/chat/completions",
                    post(|Json(body): Json<serde_json::Value>| async move {
                        assert_eq!(body["max_tokens"], 512);
                        let content = if body.get("response_format").is_some() {
                            json!({"reply":"All set, Captain."}).to_string()
                        } else {
                            "All set, Captain.".to_owned()
                        };
                        Json(json!({"choices":[{"message":{"content":content}}]}))
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let client = reqwest::Client::new();
        assert_eq!(
            text(&client, &origin, "test", 0.0, "role", "hello")
                .await
                .unwrap(),
            "All set, Captain."
        );
        assert_eq!(
            narration(&client, &origin, "test", 0.0, "role", "hello")
                .await
                .unwrap(),
            "All set, Captain."
        );
        task.abort();
    }

    #[test]
    fn prompt_labels_configured_examples_and_keeps_live_facts_separate() {
        let guidance = crate::support::Guidance::default();
        let text = super::prompt(
            "role instructions",
            guidance.get("examples"),
            &["example phrase".into()],
            "live facts",
        );
        assert!(text.contains("fictional examples"));
        assert!(text.find("example phrase").unwrap() < text.find("live facts").unwrap());
    }

    #[test]
    fn history_and_cleaning_preserve_unicode_boundaries() {
        let words = "é".repeat(600);
        let result = clean_reply(words.clone());
        assert_eq!(result.chars().count(), 500);
        let exchanges = history([(words.as_str(), words.as_str())], 4, 512, 768);
        assert_eq!(exchanges[0]["user"].as_str().unwrap().chars().count(), 512);
        assert_eq!(exchanges[0]["reply"].as_str().unwrap().chars().count(), 600);
    }
}

/// Give every actor the same explicit distinction between examples and current facts.
pub fn prompt(
    instructions: &str,
    example_guidance: &str,
    examples: &[String],
    context: &str,
) -> String {
    format!(
        "{instructions}\n{example_guidance}\n{}\n{context}",
        examples
            .iter()
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}
