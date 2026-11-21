//! Request fencing for a configured gameplay worker incarnation.
use crate::{MAX_LINE, ParseError, identifier, protocol, recover_request_id};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::{collections::BTreeMap, fmt, io, io::Read};

pub const CAPABILITY: &str = "worker_registry_v1";
pub const WORKER_ID_ERROR: &str =
    "--worker-id requires 1..=64 ASCII letters, digits, hyphens or underscores";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerIdentity {
    pub worker_id: String,
    pub incarnation_id: String,
}

impl WorkerIdentity {
    pub fn start(worker_id: String) -> io::Result<Self> {
        if !identifier(&worker_id, 1) {
            return Err(io::Error::other(WORKER_ID_ERROR));
        }
        let mut incarnation = [0; 16];
        std::fs::File::open("/dev/urandom")?
            .read_exact(&mut incarnation)
            .map_err(|error| io::Error::other(format!("worker incarnation entropy: {error}")))?;
        Ok(Self {
            worker_id,
            incarnation_id: protocol::encode_hex(&incarnation),
        })
    }

    /// Validate the bridge fence before dispatch and leave the typed request
    /// schema intact. The second result indicates an exact fence, allowing
    /// receipt polling. Without a fence, only discovery is allowed.
    pub fn strip_request_envelope(&self, line: &[u8]) -> Result<(Vec<u8>, bool), ParseError> {
        let error = |code| ParseError {
            request_id: recover_request_id(line),
            code,
        };
        if line.len() > MAX_LINE {
            return Err(error("line_too_long"));
        }
        let RequestObject(mut body) =
            serde_json::from_slice(line).map_err(|_| error("invalid_json_or_schema"))?;
        let fenced = match body.remove("worker_identity") {
            Some(raw) => {
                if !serde_json::from_str::<Self>(raw.get()).is_ok_and(|actual| actual == *self) {
                    return Err(error("worker_identity_mismatch"));
                }
                true
            }
            None => {
                let hello = body.get("op").is_some_and(|raw| {
                    serde_json::from_str::<String>(raw.get()).is_ok_and(|op| op == "hello")
                });
                if !hello {
                    return Err(error("worker_identity_required"));
                }
                false
            }
        };
        serde_json::to_vec(&body)
            .map(|body| (body, fenced))
            .map_err(|_| error("invalid_json_or_schema"))
    }
}

// Hold the body values as raw JSON so the normal typed deserializer still
// detects duplicate keys inside nested request fields.
struct RequestObject<'a>(BTreeMap<String, &'a RawValue>);

impl<'de> Deserialize<'de> for RequestObject<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RequestVisitor;

        impl<'de> Visitor<'de> for RequestVisitor {
            type Value = RequestObject<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bridge request object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut fields = BTreeMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    if fields.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate request field"));
                    }
                }
                Ok(RequestObject(fields))
            }
        }

        deserializer.deserialize_map(RequestVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Engine, IO_DEADLINE, Request, Response, loadout_selection_tests, parse_request,
        progression::{RewardKind, RewardRecipient},
        read_line,
    };
    use serde_json::{Value, json};
    use std::{
        io::Write,
        net::{Ipv4Addr, TcpListener, TcpStream},
    };

    fn poll_wire(engine: &Engine<Vec<u8>>, worker: &WorkerIdentity, request: Value) -> Value {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let mut client = TcpStream::connect_timeout(&address, IO_DEADLINE).unwrap();
        client.set_write_timeout(Some(IO_DEADLINE)).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        writeln!(client, "{request}").unwrap();

        let line = read_line(&mut stream, IO_DEADLINE).unwrap();
        let mut response = match worker.strip_request_envelope(&line) {
            Ok((body, fenced)) => {
                let Request::Hello { request_id, .. } = parse_request(&body).unwrap() else {
                    panic!("expected a receipt poll");
                };
                engine.describe_worker(request_id, fenced)
            }
            Err(error) => Response::error(error.request_id, error.code),
        };
        response.worker_identity = Some(worker.clone());
        stream.set_write_timeout(Some(IO_DEADLINE)).unwrap();
        writeln!(stream, "{}", serde_json::to_string(&response).unwrap()).unwrap();
        serde_json::from_slice(&read_line(&mut client, IO_DEADLINE).unwrap()).unwrap()
    }

    #[test]
    fn fenced_hello_reads_pending_receipts_and_unfenced_discovery_keeps_them_private() {
        let mut engine = loadout_selection_tests::fixture();
        let selection = loadout_selection_tests::select(&mut engine, 1, 1);
        assert!(selection.ok && selection.loadout_selections.len() == 1);
        let active = engine.active.as_mut().unwrap();
        active.reservation.character.account_id = Some("synthetic-account".into());
        active.reservation.character.progression = Some(Default::default());
        let reward = RewardRecipient::from_active(active).unwrap().event(
            "synthetic-fenced-receipt-poll".into(),
            RewardKind::EnemyKilled,
            1,
        );
        engine.commit_rewards(vec![reward.clone()]);
        let identity = WorkerIdentity {
            worker_id: "synthetic-worker".into(),
            incarnation_id: "synthetic-current-incarnation".into(),
        };
        let sequence = engine.sequence;
        let log_bytes = engine.written;
        let wall = engine.last_wall_secs;
        let tick = engine.last_world_tick;

        let mut request = json!({"op":"hello","v":2,"request_id":"synthetic-poll"});
        let discovery = poll_wire(&engine, &identity, request.clone());
        assert_eq!(discovery["ok"], true);
        assert_eq!(discovery["worker_identity"], json!(identity));
        assert!(discovery.get("rewards").is_none());
        assert!(discovery.get("loadout_selections").is_none());
        request["worker_identity"] = json!({
            "worker_id": &identity.worker_id,
            "incarnation_id": "synthetic-superseded-incarnation"
        });
        let stale = poll_wire(&engine, &identity, request.clone());
        assert_eq!(stale["error"], "worker_identity_mismatch");
        assert!(stale.get("rewards").is_none());
        assert!(stale.get("loadout_selections").is_none());

        request["worker_identity"] = json!(identity);
        for _ in 0..2 {
            let reply = poll_wire(&engine, &identity, request.clone());
            assert_eq!(reply["ok"], true);
            assert_eq!(reply["request_id"], "synthetic-poll");
            assert_eq!(reply["worker_identity"], json!(identity));
            assert_eq!(reply["messages"], json!([]));
            assert_eq!(reply["rewards"], json!([reward]));
            assert_eq!(
                reply["loadout_selections"],
                json!(selection.loadout_selections)
            );
        }
        assert_eq!(engine.pending_reward_batch(), [reward]);
        assert_eq!(
            engine.pending_loadout_selections(),
            selection.loadout_selections
        );
        assert_eq!(engine.sequence, sequence);
        assert_eq!(engine.written, log_bytes);
        assert_eq!(engine.last_wall_secs, wall);
        assert_eq!(engine.last_world_tick, tick);
    }
}
