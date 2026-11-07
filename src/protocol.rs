//! The pinned official protobuf schema is the single source of truth for wire data and forms.
use anyhow::{Context, Result, bail};
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, Value};
use std::sync::OnceLock;

pub const MAX_FRAME: usize = 512;
pub const BROADCAST: u32 = u32::MAX;
pub const TEXT_PORT: i32 = 1;
pub const ADMIN_PORT: i32 = 6;

pub fn pool() -> &'static DescriptorPool {
    static POOL: OnceLock<DescriptorPool> = OnceLock::new();
    POOL.get_or_init(|| {
        DescriptorPool::decode(
            include_bytes!(concat!(env!("OUT_DIR"), "/meshtastic.bin")).as_slice(),
        )
        .expect("Bundled schema")
    })
}

pub fn descriptor(name: &str) -> MessageDescriptor {
    pool()
        .get_message_by_name(&format!("meshtastic.{name}"))
        .expect("Bundled message")
}

pub fn message(name: &str) -> DynamicMessage {
    DynamicMessage::new(descriptor(name))
}

pub fn put(msg: &mut DynamicMessage, name: &str, value: Value) {
    let field = msg.descriptor().get_field_by_name(name).expect(name);
    msg.set_field(&field, value);
}

use prost_reflect::ReflectMessage;

pub fn has(msg: &DynamicMessage, name: &str) -> bool {
    msg.has_field_by_name(name)
}
pub fn nested(msg: &DynamicMessage, name: &str) -> Option<DynamicMessage> {
    if !msg.has_field_by_name(name) {
        return None;
    }
    match msg.get_field_by_name(name)?.as_ref() {
        Value::Message(value) => Some(value.clone()),
        _ => None,
    }
}
pub fn number(msg: &DynamicMessage, name: &str) -> u32 {
    msg.get_field_by_name(name)
        .and_then(|v| match v.as_ref() {
            Value::U32(n) => Some(*n),
            Value::I32(n) | Value::EnumNumber(n) => Some(*n as u32),
            _ => None,
        })
        .unwrap_or_default()
}
pub fn float(msg: &DynamicMessage, name: &str) -> f64 {
    msg.get_field_by_name(name)
        .and_then(|v| match v.as_ref() {
            Value::F32(n) => Some(*n as f64),
            Value::F64(n) => Some(*n),
            _ => None,
        })
        .unwrap_or_default()
}
pub fn string(msg: &DynamicMessage, name: &str) -> String {
    msg.get_field_by_name(name)
        .and_then(|v| match v.as_ref() {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
pub fn bytes(msg: &DynamicMessage, name: &str) -> Vec<u8> {
    msg.get_field_by_name(name)
        .and_then(|v| match v.as_ref() {
            Value::Bytes(b) => Some(b.to_vec()),
            _ => None,
        })
        .unwrap_or_default()
}
pub fn flag(msg: &DynamicMessage, name: &str) -> bool {
    msg.get_field_by_name(name)
        .is_some_and(|v| matches!(v.as_ref(), Value::Bool(true)))
}

pub fn to_radio(name: &str, value: Value) -> DynamicMessage {
    let mut msg = message("ToRadio");
    put(&mut msg, name, value);
    msg
}

pub fn mesh_packet(
    own: u32,
    to: u32,
    channel: u32,
    port: i32,
    payload: Vec<u8>,
    response: bool,
) -> (u32, DynamicMessage) {
    let id = rand::random::<u32>().max(1);
    let mut data = message("Data");
    put(&mut data, "portnum", Value::EnumNumber(port));
    put(&mut data, "payload", Value::Bytes(payload.into()));
    put(&mut data, "want_response", Value::Bool(response));
    let mut packet = message("MeshPacket");
    put(&mut packet, "from", Value::U32(own));
    put(&mut packet, "to", Value::U32(to));
    put(&mut packet, "channel", Value::U32(channel));
    put(&mut packet, "id", Value::U32(id));
    put(&mut packet, "hop_limit", Value::U32(3));
    put(&mut packet, "want_ack", Value::Bool(true));
    put(&mut packet, "decoded", Value::Message(data));
    (id, to_radio("packet", Value::Message(packet)))
}

pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_FRAME {
        bail!("Packet exceeds the Meshtastic {MAX_FRAME}-byte limit");
    }
    let mut frame = vec![0x94, 0xc3, (payload.len() >> 8) as u8, payload.len() as u8];
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Incremental framing, including synchronization after serial boot/debug text.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}
impl FrameDecoder {
    pub fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(data);
        let mut frames = vec![];
        loop {
            let Some(start) = self.buffer.windows(2).position(|w| w == [0x94, 0xc3]) else {
                let last = self.buffer.last().copied();
                self.buffer.clear();
                if last == Some(0x94) {
                    self.buffer.push(0x94);
                }
                break;
            };
            self.buffer.drain(..start);
            if self.buffer.len() < 4 {
                break;
            }
            let len = u16::from_be_bytes([self.buffer[2], self.buffer[3]]) as usize;
            if len == 0 || len > MAX_FRAME {
                self.buffer.drain(..2);
                continue;
            }
            if self.buffer.len() < len + 4 {
                break;
            }
            frames.push(self.buffer[4..len + 4].to_vec());
            self.buffer.drain(..len + 4);
        }
        frames
    }
}

pub fn decode(name: &str, data: &[u8]) -> Result<DynamicMessage> {
    DynamicMessage::decode(descriptor(name), data).with_context(|| format!("Invalid {name} packet"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    #[test]
    fn framing_recovers_from_logs_bad_lengths_and_split_magic() {
        let mut decoder = FrameDecoder::default();
        assert!(decoder.feed(b"Booting...\x94").is_empty());
        assert!(
            decoder
                .feed(&[0xc3, 0xff, 0xff, 0x94, 0xc3, 0, 3, 1])
                .is_empty()
        );
        assert_eq!(
            decoder.feed(&[2, 3, 0x94, 0xc3, 0, 1, 4]),
            vec![vec![1, 2, 3], vec![4]]
        );
    }
    #[test]
    fn framing_enforces_wire_limit() {
        assert!(encode_frame(&vec![0; 513]).is_err());
        assert_eq!(encode_frame(&[1, 2]).unwrap(), vec![0x94, 0xc3, 0, 2, 1, 2]);
    }
    #[test]
    fn config_request_round_trips() {
        let packet = to_radio("want_config_id", Value::U32(123));
        assert_eq!(
            number(
                &decode("ToRadio", &packet.encode_to_vec()).unwrap(),
                "want_config_id"
            ),
            123
        );
    }
}
