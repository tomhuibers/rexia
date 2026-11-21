//! The control connection (`rex.control.v1`): HELLO/WELCOME, requests matched to replies
//! by id, and the server's ordered event stream.
use super::endpoint::{self, Endpoint};
use anyhow::{Context, Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub const PROTOCOL: &str = "rex.control.v1";
/// The oldest API revision rexia speaks.
pub const MIN_API_REVISION: u32 = 8;
/// The newest API revision rexia speaks: Rex v0.1.0-preview.21.
pub const API_REVISION: u32 = 18;
pub const PROMPT_REVISION: u32 = 17;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const ACTION_TIMEOUT: Duration = Duration::from_secs(3660);

/// Who this client is, as the server lists it in `client.list` and `rex client ls`.
#[derive(Clone, Debug)]
pub struct Identity {
    pub name: String,
    pub version: String,
    pub label: Option<String>,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            name: "rexia".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            label: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Welcome {
    #[serde(default)]
    pub api_revision: u32,
    #[serde(default)]
    pub api_server_revision: u32,
    #[serde(default)]
    pub client: Option<WelcomeClient>,
}

#[derive(Clone, Default, Deserialize)]
pub struct WelcomeClient {
    pub id: String,
    pub reconnect_token: String,
}

impl std::fmt::Debug for WelcomeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WelcomeClient")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// An error the server returned for a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.code)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for ApiError {}

/// The control connection ended; requests that were waiting can't be answered.
#[derive(Debug)]
pub struct Disconnected(pub String);

impl std::fmt::Display for Disconnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "disconnected: {}", self.0)
    }
}

impl std::error::Error for Disconnected {}

/// A method's result, with the `seq` of the last event queued before it started.
#[derive(Clone, Debug)]
pub struct Reply {
    pub seq: u64,
    pub value: Value,
}

/// A server event. Session events carry their session's id.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub session_id: Option<String>,
    pub name: String,
    pub payload: Value,
}

impl Event {
    pub fn from_wire(seq: u64, payload: &Value) -> Option<Self> {
        let kind = payload["type"].as_str()?;
        if kind == "session_event" {
            let inner = &payload["payload"];
            return Some(Self {
                seq,
                session_id: inner["session_id"].as_str().map(str::to_owned),
                name: inner["event"]["type"].as_str()?.to_owned(),
                payload: inner["event"]["payload"].clone(),
            });
        }
        Some(Self {
            seq,
            session_id: None,
            name: kind.to_owned(),
            payload: payload["payload"].clone(),
        })
    }
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Reply>>>>>;

/// One control connection. Cloning shares it.
#[derive(Clone)]
pub struct Control {
    outgoing: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
    pub welcome: Welcome,
}

pub fn hello(identity: &Identity, reconnect_token: Option<&str>) -> Value {
    let mut client = json!({
        "name": identity.name,
        "version": identity.version,
        "platform": "linux",
        "kind": "app",
    });
    if let Some(label) = identity.label.as_deref().filter(|l| !l.trim().is_empty()) {
        client["label"] = json!(label);
    }
    let mut hello = json!({
        "type": "hello",
        "api_revision": API_REVISION,
        "api_min_revision": MIN_API_REVISION,
        "client": client,
    });
    if let Some(token) = reconnect_token {
        hello["reconnect_token"] = json!(token);
    }
    hello
}

impl Control {
    /// Connect, say HELLO and wait for WELCOME. Events arrive on the returned receiver,
    /// which ends when the connection does.
    pub async fn connect(
        endpoint: &Endpoint,
        identity: &Identity,
        reconnect_token: Option<&str>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Event>)> {
        let mut socket = endpoint::connect(endpoint, "/api/control", PROTOCOL).await?;
        let greeting = hello(identity, reconnect_token).to_string();
        socket.send(Message::Text(greeting.into())).await?;
        let welcome = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(&text)?;
                        match value["type"].as_str() {
                            Some("welcome") => {
                                return Ok::<_, anyhow::Error>(serde_json::from_value::<Welcome>(
                                    value,
                                )?);
                            }
                            Some("error") | Some("response") => {
                                return Err(anyhow!(
                                    "Rex refused the connection: {}",
                                    value["error"]["message"]
                                        .as_str()
                                        .or(value["message"].as_str())
                                        .unwrap_or("unknown error")
                                ));
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                        return Err(anyhow!("Rex closed the connection: {reason}"));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                    None => return Err(anyhow!("Rex closed the connection")),
                }
            }
        })
        .await
        .context("Rex did not answer HELLO")??;
        anyhow::ensure!(
            (MIN_API_REVISION..=API_REVISION).contains(&welcome.api_revision),
            "this server speaks API revision {}, rexia needs {MIN_API_REVISION}–{API_REVISION}",
            welcome.api_revision
        );

        let (mut sink, mut stream) = socket.split();
        let (outgoing, mut queue) = mpsc::unbounded_channel::<Message>();
        let (events, receiver) = mpsc::unbounded_channel();
        let pending: Pending = Arc::default();

        tokio::spawn(async move {
            while let Some(message) = queue.recv().await {
                let closing = matches!(message, Message::Close(_));
                if sink.send(message).await.is_err() || closing {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let reader_pending = pending.clone();
        let pong = outgoing.clone();
        tokio::spawn(async move {
            let reason = loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(error) = dispatch(&text, &reader_pending, &events) {
                            break format!("{error:#}");
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = pong.send(Message::Pong(data));
                    }
                    Some(Ok(Message::Close(frame))) => {
                        break frame
                            .map(|f| f.reason.to_string())
                            .filter(|r| !r.is_empty())
                            .unwrap_or_else(|| "the server closed the connection".into());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => break error.to_string(),
                    None => break "the connection ended".into(),
                }
            };
            let waiting: Vec<_> = reader_pending
                .lock()
                .expect("pending requests")
                .drain()
                .collect();
            for (_, reply) in waiting {
                let _ = reply.send(Err(Disconnected(reason.clone()).into()));
            }
        });

        Ok((
            Self {
                outgoing,
                pending,
                next_id: Arc::default(),
                welcome,
            },
            receiver,
        ))
    }

    pub fn client_id(&self) -> Option<&str> {
        self.welcome.client.as_ref().map(|c| c.id.as_str())
    }

    pub fn reconnect_token(&self) -> Option<&str> {
        self.welcome
            .client
            .as_ref()
            .map(|c| c.reconnect_token.as_str())
    }

    /// Send a request and wait for its reply.
    pub async fn call(&self, method: &str, payload: Value) -> Result<Reply> {
        self.start_call(method, payload, REQUEST_TIMEOUT)?.await
    }

    /// Send now so an action can wait for a UI prompt without blocking the event loop.
    pub(super) fn start_call(
        &self,
        method: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<impl Future<Output = Result<Reply>> + use<>> {
        let id = (self.next_id.fetch_add(1, Ordering::Relaxed) + 1).to_string();
        let (reply, answer) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending requests")
            .insert(id.clone(), reply);
        let message = json!({"type": "request", "id": id, "method": method, "payload": payload});
        if self
            .outgoing
            .send(Message::Text(message.to_string().into()))
            .is_err()
        {
            self.pending.lock().expect("pending requests").remove(&id);
            return Err(Disconnected("the connection is closed".into()).into());
        }
        let pending = self.pending.clone();
        let method = method.to_owned();
        Ok(async move {
            match tokio::time::timeout(timeout, answer).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(Disconnected("the connection is closed".into()).into()),
                Err(_) => {
                    pending.lock().expect("pending requests").remove(&id);
                    Err(anyhow!("{method} timed out; it was not retried"))
                }
            }
        })
    }

    /// In-memory transport for host tests: exercise real request queuing and reply
    /// dispatch without opening a socket or requiring a running Rex server.
    #[cfg(test)]
    pub(super) fn test_pair() -> (Self, mpsc::UnboundedReceiver<Message>) {
        let (outgoing, requests) = mpsc::unbounded_channel();
        (
            Self {
                outgoing,
                pending: Arc::default(),
                next_id: Arc::default(),
                welcome: Welcome {
                    api_revision: API_REVISION,
                    ..Default::default()
                },
            },
            requests,
        )
    }

    #[cfg(test)]
    pub(super) fn test_response(&self, response: Value) {
        let (events, _) = mpsc::unbounded_channel();
        dispatch(&response.to_string(), &self.pending, &events).expect("test response");
    }

    /// The value of a successful call.
    pub async fn value(&self, method: &str, payload: Value) -> Result<Value> {
        Ok(self.call(method, payload).await?.value)
    }

    pub fn close(&self) {
        let _ = self.outgoing.send(Message::Close(None));
    }
}

fn dispatch(text: &str, pending: &Pending, events: &mpsc::UnboundedSender<Event>) -> Result<()> {
    let value: Value = serde_json::from_str(text).context("unreadable message from Rex")?;
    match value["type"].as_str() {
        Some("response") => {
            let id = value["id"].as_str().unwrap_or_default();
            let Some(reply) = pending.lock().expect("pending requests").remove(id) else {
                return Ok(());
            };
            let result = if value["error"].is_object() {
                Err(ApiError {
                    code: value["error"]["code"].as_str().unwrap_or("internal").into(),
                    message: value["error"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .into(),
                }
                .into())
            } else {
                Ok(Reply {
                    seq: value["seq"].as_u64().unwrap_or(0),
                    value: match value.get("payload") {
                        Some(Value::Null) | None => json!({}),
                        Some(payload) => payload.clone(),
                    },
                })
            };
            let _ = reply.send(result);
        }
        Some("event") => {
            let seq = value["seq"].as_u64().unwrap_or(0);
            if let Some(event) = Event::from_wire(seq, &value["payload"]) {
                let _ = events.send(event);
            }
        }
        _ => {}
    }
    Ok(())
}

/// The API error code of a failed call, if the server returned one.
pub fn error_code(error: &anyhow::Error) -> Option<&str> {
    error.downcast_ref::<ApiError>().map(|e| e.code.as_str())
}

pub fn is_disconnect(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Disconnected>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_names_the_supported_range_and_label() {
        let identity = Identity {
            label: Some("laptop".into()),
            ..Default::default()
        };
        let value = hello(&identity, Some("token"));
        assert_eq!(value["type"], "hello");
        assert_eq!(value["api_revision"], API_REVISION);
        assert_eq!(value["api_min_revision"], MIN_API_REVISION);
        assert_eq!(value["client"]["kind"], "app");
        assert_eq!(value["client"]["platform"], "linux");
        assert_eq!(value["client"]["label"], "laptop");
        assert_eq!(value["reconnect_token"], "token");
        let anonymous = hello(&Identity::default(), None);
        assert!(anonymous["client"].get("label").is_none());
        assert!(anonymous.get("reconnect_token").is_none());
    }

    #[test]
    fn session_events_carry_their_session() {
        let wire = json!({"type":"session_event","payload":{"session_id":"session:1",
            "event":{"type":"block_closed","payload":{"block_id":"b"}}}});
        let event = Event::from_wire(3, &wire).unwrap();
        assert_eq!(event.session_id.as_deref(), Some("session:1"));
        assert_eq!(event.name, "block_closed");
        assert_eq!(event.payload["block_id"], "b");
        let server =
            Event::from_wire(4, &json!({"type":"hosts_changed","payload":{"hash":"x"}})).unwrap();
        assert_eq!(server.session_id, None);
        assert_eq!(server.name, "hosts_changed");
    }

    #[tokio::test]
    async fn replies_and_errors_reach_their_request() {
        let pending: Pending = Arc::default();
        let (events, mut received) = mpsc::unbounded_channel();
        let (ok, ok_reply) = oneshot::channel();
        let (bad, bad_reply) = oneshot::channel();
        pending.lock().unwrap().insert("1".into(), ok);
        pending.lock().unwrap().insert("2".into(), bad);
        dispatch(
            r#"{"type":"response","id":"1","seq":7,"payload":{"revision":9}}"#,
            &pending,
            &events,
        )
        .unwrap();
        dispatch(
            r#"{"type":"response","id":"2","error":{"code":"session_not_found","message":"no such session"}}"#,
            &pending,
            &events,
        )
        .unwrap();
        dispatch(
            r#"{"type":"response","id":"stale","payload":{}}"#,
            &pending,
            &events,
        )
        .unwrap();
        dispatch(
            r#"{"type":"event","seq":8,"payload":{"type":"session_created","payload":{}}}"#,
            &pending,
            &events,
        )
        .unwrap();
        let reply = ok_reply.await.unwrap().unwrap();
        assert_eq!((reply.seq, reply.value["revision"].as_u64()), (7, Some(9)));
        let error = bad_reply.await.unwrap().unwrap_err();
        assert_eq!(error_code(&error), Some("session_not_found"));
        assert_eq!(received.recv().await.unwrap().name, "session_created");
    }
}
