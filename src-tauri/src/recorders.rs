//! Local recorder control for OBS Studio and Meld Studio.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message,
    MaybeTlsStream,
    WebSocketStream,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub kind: String,
    pub reachable: bool,
    pub recording: bool,
    pub replay_buffer_active: bool,
    pub detail: String,
}

fn obs_authentication(password: &str, salt: &str, challenge: &str) -> String {
    let secret = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

async fn next_json(socket: &mut Socket) -> Result<Value, String> {
    loop {
        let message = tokio::time::timeout(SOCKET_TIMEOUT, socket.next())
            .await
            .map_err(|_| "The recorder did not respond in time".to_string())?
            .ok_or_else(|| "The recorder closed the connection".to_string())?
            .map_err(|error| format!("Recorder connection error: {error}"))?;
        match message {
            Message::Text(text) => {
                return serde_json::from_str(text.as_ref())
                    .map_err(|error| format!("Recorder returned invalid data: {error}"));
            }
            Message::Close(_) => return Err("The recorder closed the connection".to_string()),
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| format!("Recorder connection error: {error}"))?;
            }
            _ => {}
        }
    }
}

async fn obs_connect(port: u16, password: &str) -> Result<Socket, String> {
    let url = format!("ws://127.0.0.1:{port}");
    let (mut socket, _) = tokio::time::timeout(SOCKET_TIMEOUT, connect_async(&url))
        .await
        .map_err(|_| "OBS connection timed out".to_string())?
        .map_err(|error| format!("Could not connect to OBS at {url}: {error}"))?;
    let hello = next_json(&mut socket).await?;
    if hello.get("op").and_then(Value::as_i64) != Some(0) {
        return Err("OBS returned an unexpected handshake".to_string());
    }

    let mut identify = json!({
        "op": 1,
        "d": {
            "rpcVersion": 1,
            "eventSubscriptions": 64
        }
    });
    if let Some(authentication) = hello.pointer("/d/authentication") {
        let challenge = authentication
            .get("challenge")
            .and_then(Value::as_str)
            .ok_or_else(|| "OBS authentication challenge was missing".to_string())?;
        let salt = authentication
            .get("salt")
            .and_then(Value::as_str)
            .ok_or_else(|| "OBS authentication salt was missing".to_string())?;
        if password.is_empty() {
            return Err("OBS requires its WebSocket password. Add it in Clip Sources.".to_string());
        }
        identify["d"]["authentication"] = json!(obs_authentication(password, salt, challenge));
    }
    socket
        .send(Message::Text(identify.to_string().into()))
        .await
        .map_err(|error| format!("Could not identify with OBS: {error}"))?;
    let identified = next_json(&mut socket).await?;
    if identified.get("op").and_then(Value::as_i64) != Some(2) {
        return Err("OBS rejected the WebSocket credentials".to_string());
    }
    Ok(socket)
}

async fn obs_request(
    socket: &mut Socket,
    request_type: &str,
    request_id: &str,
) -> Result<Value, String> {
    socket
        .send(Message::Text(
            json!({
                "op": 6,
                "d": {
                    "requestType": request_type,
                    "requestId": request_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| format!("Could not send {request_type} to OBS: {error}"))?;
    loop {
        let message = next_json(socket).await?;
        if message.get("op").and_then(Value::as_i64) != Some(7)
            || message.pointer("/d/requestId").and_then(Value::as_str) != Some(request_id)
        {
            continue;
        }
        let successful = message
            .pointer("/d/requestStatus/result")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !successful {
            let detail = message
                .pointer("/d/requestStatus/comment")
                .and_then(Value::as_str)
                .unwrap_or("OBS rejected the request");
            return Err(detail.to_string());
        }
        return Ok(message
            .pointer("/d/responseData")
            .cloned()
            .unwrap_or_else(|| json!({})));
    }
}

pub async fn obs_status(port: u16, password: &str) -> Result<RecorderStatus, String> {
    let mut socket = obs_connect(port, password).await?;
    let replay = obs_request(&mut socket, "GetReplayBufferStatus", "clipgoblin-replay-status")
        .await?;
    let record = obs_request(&mut socket, "GetRecordStatus", "clipgoblin-record-status").await?;
    let replay_active = replay
        .get("outputActive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let recording = record
        .get("outputActive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(RecorderStatus {
        kind: "obs".to_string(),
        reachable: true,
        recording,
        replay_buffer_active: replay_active,
        detail: if replay_active {
            "OBS is connected and Replay Buffer is ready".to_string()
        } else {
            "OBS is connected. Start Replay Buffer before saving a clip.".to_string()
        },
    })
}

pub async fn obs_save_replay(port: u16, password: &str) -> Result<PathBuf, String> {
    let mut socket = obs_connect(port, password).await?;
    socket
        .send(Message::Text(
            json!({
                "op": 6,
                "d": {
                    "requestType": "SaveReplayBuffer",
                    "requestId": "clipgoblin-save-replay"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| format!("Could not ask OBS to save the replay: {error}"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut accepted = false;
    while tokio::time::Instant::now() < deadline {
        let message = next_json(&mut socket).await?;
        match message.get("op").and_then(Value::as_i64) {
            Some(7)
                if message.pointer("/d/requestId").and_then(Value::as_str)
                    == Some("clipgoblin-save-replay") =>
            {
                if !message
                    .pointer("/d/requestStatus/result")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    let detail = message
                        .pointer("/d/requestStatus/comment")
                        .and_then(Value::as_str)
                        .unwrap_or("OBS could not save the replay");
                    return Err(detail.to_string());
                }
                accepted = true;
            }
            Some(5)
                if message.pointer("/d/eventType").and_then(Value::as_str)
                    == Some("ReplayBufferSaved") =>
            {
                let path = message
                    .pointer("/d/eventData/savedReplayPath")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "OBS saved the replay but did not return its path".to_string())?;
                return Ok(PathBuf::from(path));
            }
            _ => {}
        }
    }
    if accepted {
        Err("OBS accepted the replay request but the saved file was not reported".to_string())
    } else {
        Err("OBS did not accept the replay request".to_string())
    }
}

fn meld_method_index(init: &Value, method_name: &str) -> Option<i64> {
    init.pointer("/data/meld/methods")?
        .as_array()?
        .iter()
        .find_map(|method| {
            let values = method.as_array()?;
            (values.first()?.as_str()? == method_name)
                .then(|| values.get(1)?.as_i64())
                .flatten()
        })
}

fn meld_property_bool(init: &Value, property_name: &str) -> bool {
    init.pointer("/data/meld/properties")
        .and_then(Value::as_array)
        .and_then(|properties| {
            properties.iter().find_map(|property| {
                let values = property.as_array()?;
                (values.get(1)?.as_str()? == property_name)
                    .then(|| values.get(3)?.as_bool())
                    .flatten()
            })
        })
        .unwrap_or(false)
}

async fn meld_connect() -> Result<(Socket, Value), String> {
    let url = "ws://127.0.0.1:13376";
    let (mut socket, _) = tokio::time::timeout(SOCKET_TIMEOUT, connect_async(url))
        .await
        .map_err(|_| "Meld connection timed out".to_string())?
        .map_err(|error| format!("Could not connect to Meld at {url}: {error}"))?;
    socket
        .send(Message::Text(json!({ "type": 3 }).to_string().into()))
        .await
        .map_err(|error| format!("Could not initialize Meld control: {error}"))?;
    let init = next_json(&mut socket).await?;
    if init.get("type").and_then(Value::as_i64) != Some(3)
        || init.pointer("/data/meld").is_none()
    {
        return Err("Meld returned an unexpected control handshake".to_string());
    }
    Ok((socket, init))
}

pub async fn meld_status() -> Result<RecorderStatus, String> {
    let (_socket, init) = meld_connect().await?;
    let recording = meld_property_bool(&init, "isRecording");
    let streaming = meld_property_bool(&init, "isStreaming");
    Ok(RecorderStatus {
        kind: "meld".to_string(),
        reachable: true,
        recording,
        replay_buffer_active: recording || streaming,
        detail: if recording || streaming {
            "Meld is connected and can record a clip".to_string()
        } else {
            "Meld is connected. Start streaming or recording before saving a clip.".to_string()
        },
    })
}

pub async fn meld_record_clip() -> Result<(), String> {
    let (mut socket, init) = meld_connect().await?;
    let method = meld_method_index(&init, "sendCommand")
        .ok_or_else(|| "This Meld version does not expose sendCommand".to_string())?;
    socket
        .send(Message::Text(
            json!({
                "type": 6,
                "object": "meld",
                "method": method,
                "args": ["meld.recordClip"],
                "id": 1
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| format!("Could not ask Meld to record a clip: {error}"))?;
    loop {
        let response = next_json(&mut socket).await?;
        if response.get("type").and_then(Value::as_i64) == Some(10)
            && response.get("id").and_then(Value::as_i64) == Some(1)
        {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{meld_method_index, meld_property_bool, obs_authentication};
    use serde_json::json;

    #[test]
    fn obs_auth_is_stable_and_challenge_sensitive() {
        let first = obs_authentication("secret", "salt", "challenge");
        assert_eq!(first, obs_authentication("secret", "salt", "challenge"));
        assert_ne!(first, obs_authentication("secret", "salt", "other"));
    }

    #[test]
    fn meld_webchannel_metadata_is_discovered_by_name() {
        let init = json!({
            "data": { "meld": {
                "methods": [["sendCommand", 7]],
                "properties": [
                    [0, "isRecording", 2, true],
                    [1, "isStreaming", 3, false]
                ]
            }}
        });
        assert_eq!(meld_method_index(&init, "sendCommand"), Some(7));
        assert!(meld_property_bool(&init, "isRecording"));
        assert!(!meld_property_bool(&init, "isStreaming"));
    }
}
