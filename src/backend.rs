use crate::{
    model::{ChatMessage, Command, Connection, Event, Node},
    protocol::{self as p, put},
    settings,
    transport::{self, Link},
};
use anyhow::{Context, Result, bail};
use prost::Message;
use prost_reflect::{DynamicMessage, ReflectMessage, Value};
use std::{
    collections::HashMap,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub struct Backend {
    pub commands: mpsc::UnboundedSender<Command>,
}
impl Backend {
    pub fn start(events: Sender<Event>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("meshtastic-radio".into())
            .spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("Async runtime");
                runtime.block_on(run(rx, events));
            })
            .expect("Radio worker thread");
        Self { commands: tx }
    }
}

struct PendingSave {
    key: String,
    deadline: Instant,
}
#[derive(Default)]
struct Session {
    own: u32,
    nonce: u32,
    ready: bool,
    label: String,
    passkey: Vec<u8>,
    passkey_time: Option<Instant>,
    nodes: HashMap<u32, Node>,
    saves: HashMap<u32, PendingSave>,
    reads: HashMap<String, Instant>,
    waiting_save: Option<(String, DynamicMessage, Instant)>,
}

impl Session {
    fn admin(&self, mut message: DynamicMessage, response: bool) -> Result<(u32, DynamicMessage)> {
        if !self.ready || self.own == 0 {
            bail!("Wait for the node configuration to finish loading");
        }
        if !self.passkey.is_empty() {
            put(
                &mut message,
                "session_passkey",
                Value::Bytes(self.passkey.clone().into()),
            );
        }
        Ok(p::mesh_packet(
            self.own,
            self.own,
            0,
            p::ADMIN_PORT,
            message.encode_to_vec(),
            response,
        ))
    }

    fn handle(
        &mut self,
        packet: DynamicMessage,
        events: &Sender<Event>,
    ) -> Result<Vec<DynamicMessage>> {
        let mut outgoing = vec![];
        if let Some(info) = p::nested(&packet, "my_info") {
            self.own = p::number(&info, "my_node_num");
        }
        if let Some(info) = p::nested(&packet, "node_info") {
            self.update_node(&info, events);
        }
        if let Some(config) = p::nested(&packet, "config") {
            self.config(&config, "config", events);
        }
        if let Some(config) = p::nested(&packet, "moduleConfig") {
            self.config(&config, "module", events);
        }
        if let Some(channel) = p::nested(&packet, "channel") {
            self.reads
                .remove(&format!("channel.{}", p::number(&channel, "index")));
            emit(
                events,
                Event::Setting {
                    key: format!("channel.{}", p::number(&channel, "index")),
                    message: channel,
                },
            );
        }
        if let Some(metadata) = p::nested(&packet, "metadata") {
            emit(
                events,
                Event::Log(format!(
                    "Firmware {}",
                    p::string(&metadata, "firmware_version")
                )),
            );
        }
        if let Some(log) = p::nested(&packet, "log_record") {
            emit(events, Event::Log(p::string(&log, "message")));
        }
        if let Some(notification) = p::nested(&packet, "clientNotification") {
            emit(events, Event::Log(p::string(&notification, "message")));
        }
        if p::number(&packet, "config_complete_id") == self.nonce && self.nonce != 0 {
            if self.own == 0 {
                bail!("Node finished configuration without reporting its identity");
            }
            self.ready = true;
            emit(
                events,
                Event::Ready {
                    own: self.own,
                    label: self.label.clone(),
                },
            );
            outgoing.push(self.admin(settings::admin_request("user")?, true)?.1);
        }
        if p::flag(&packet, "rebooted") {
            self.ready = false;
            self.passkey.clear();
            self.passkey_time = None;
            self.nonce = rand::random::<u32>().max(1);
            emit(
                events,
                Event::Connecting("Node rebooted; reading configuration…".into()),
            );
            outgoing.push(p::to_radio("want_config_id", Value::U32(self.nonce)));
        }
        if let Some(mesh) = p::nested(&packet, "packet")
            && let Some(data) = p::nested(&mesh, "decoded")
        {
            let payload = p::bytes(&data, "payload");
            let from = p::number(&mesh, "from");
            match p::number(&data, "portnum") as i32 {
                p::TEXT_PORT => {
                    let timestamp = p::number(&mesh, "rx_time");
                    emit(
                        events,
                        Event::Message(ChatMessage {
                            id: p::number(&mesh, "id"),
                            from,
                            to: p::number(&mesh, "to"),
                            channel: p::number(&mesh, "channel"),
                            text: String::from_utf8_lossy(&payload).into_owned(),
                            timestamp: if timestamp == 0 {
                                chrono::Utc::now().timestamp()
                            } else {
                                timestamp as i64
                            },
                            outgoing: from == self.own,
                            status: "Received".into(),
                        }),
                    );
                }
                p::ADMIN_PORT if from == self.own => {
                    let admin = p::decode("AdminMessage", &payload)?;
                    let passkey = p::bytes(&admin, "session_passkey");
                    if !passkey.is_empty() {
                        self.passkey = passkey;
                        self.passkey_time = Some(Instant::now());
                    }
                    if let Some(config) = p::nested(&admin, "get_config_response") {
                        self.config(&config, "config", events);
                    }
                    if let Some(config) = p::nested(&admin, "get_module_config_response") {
                        self.config(&config, "module", events);
                    }
                    if let Some(user) = p::nested(&admin, "get_owner_response") {
                        self.reads.remove("user");
                        emit(
                            events,
                            Event::Setting {
                                key: "user".into(),
                                message: user,
                            },
                        );
                    }
                    if let Some(channel) = p::nested(&admin, "get_channel_response") {
                        self.reads
                            .remove(&format!("channel.{}", p::number(&channel, "index")));
                        emit(
                            events,
                            Event::Setting {
                                key: format!("channel.{}", p::number(&channel, "index")),
                                message: channel,
                            },
                        );
                    }
                    for (response, setter, key) in [
                        ("get_ringtone_response", "set_ringtone_message", "ringtone"),
                        (
                            "get_canned_message_module_messages_response",
                            "set_canned_message_module_messages",
                            "canned_text",
                        ),
                    ] {
                        if p::has(&admin, response) {
                            self.reads.remove(key);
                            let mut message = p::message("AdminMessage");
                            put(
                                &mut message,
                                setter,
                                Value::String(p::string(&admin, response)),
                            );
                            emit(
                                events,
                                Event::Setting {
                                    key: key.into(),
                                    message,
                                },
                            );
                        }
                    }
                    if self.passkey_time.is_some()
                        && let Some((key, message, _)) = self.waiting_save.take()
                    {
                        outgoing.push(self.start_save(key, message)?);
                    }
                }
                5 => {
                    // ROUTING_APP: a successful write is only reported after the radio ACKs it.
                    let routing = p::decode("Routing", &payload)?;
                    let request_id = p::number(&data, "request_id");
                    let error = p::number(&routing, "error_reason");
                    let status = if error == 0 {
                        "Acknowledged by radio".into()
                    } else {
                        format!(
                            "Radio rejected packet: {}",
                            enum_label(&routing, "error_reason", error as i32)
                        )
                    };
                    emit(
                        events,
                        Event::Delivery {
                            id: request_id,
                            status: status.clone(),
                        },
                    );
                    if let Some(save) = self.saves.remove(&request_id) {
                        if error == 0 {
                            emit(
                                events,
                                Event::Saved {
                                    key: save.key.clone(),
                                },
                            );
                            outgoing.push(self.admin(settings::admin_request(&save.key)?, true)?.1);
                        } else {
                            emit(
                                events,
                                Event::SaveFailed {
                                    key: save.key,
                                    error: status,
                                },
                            );
                        }
                    }
                }
                4 => {
                    // NODEINFO_APP
                    if let Ok(user) = p::decode("User", &payload) {
                        let node = self.nodes.entry(from).or_insert_with(|| Node {
                            num: from,
                            ..Node::default()
                        });
                        apply_user(node, &user);
                        node.last_heard = p::number(&mesh, "rx_time");
                        node.snr = p::float(&mesh, "rx_snr");
                        emit(events, Event::Node(node.clone()));
                    }
                }
                3 => {
                    // POSITION_APP
                    if let Ok(position) = p::decode("Position", &payload) {
                        let node = self.nodes.entry(from).or_insert_with(|| Node {
                            num: from,
                            ..Node::default()
                        });
                        node.position = coordinates(&position);
                        emit(events, Event::Node(node.clone()));
                    }
                }
                _ => {}
            }
        }
        Ok(outgoing)
    }

    fn config(&mut self, wrapper: &DynamicMessage, prefix: &str, events: &Sender<Event>) {
        if let Some((key, message)) = settings::unwrap_config(wrapper, prefix) {
            self.reads.remove(&key);
            emit(events, Event::Setting { key, message });
        }
    }
    fn update_node(&mut self, info: &DynamicMessage, events: &Sender<Event>) {
        let num = p::number(info, "num");
        let node = self.nodes.entry(num).or_insert_with(|| Node {
            num,
            ..Node::default()
        });
        if let Some(user) = p::nested(info, "user") {
            apply_user(node, &user);
        }
        node.last_heard = p::number(info, "last_heard");
        node.snr = p::float(info, "snr");
        if let Some(position) = p::nested(info, "position") {
            node.position = coordinates(&position);
        }
        if p::has(info, "hops_away") {
            node.hops = Some(p::number(info, "hops_away"));
        }
        if let Some(metrics) = p::nested(info, "device_metrics") {
            node.battery = Some(p::number(&metrics, "battery_level"));
            node.voltage = Some(p::float(&metrics, "voltage"));
        }
        emit(events, Event::Node(node.clone()));
    }
    fn start_save(&mut self, key: String, message: DynamicMessage) -> Result<DynamicMessage> {
        let admin = settings::admin_set(&key, message, &self.passkey)?;
        let (id, packet) = self.admin(admin, false)?;
        if packet.encoded_len() > p::MAX_FRAME {
            bail!("These settings exceed the radio packet limit");
        }
        self.saves.insert(
            id,
            PendingSave {
                key,
                deadline: Instant::now() + Duration::from_secs(20),
            },
        );
        Ok(packet)
    }
}

fn apply_user(node: &mut Node, user: &DynamicMessage) {
    node.hardware = enum_label(user, "hw_model", p::number(user, "hw_model") as i32);
    node.long_name = p::string(user, "long_name");
    node.short_name = p::string(user, "short_name");
    node.role = enum_label(user, "role", p::number(user, "role") as i32);
}
fn coordinates(position: &DynamicMessage) -> Option<(f64, f64)> {
    if !p::has(position, "latitude_i") || !p::has(position, "longitude_i") {
        return None;
    }
    Some((
        p::number(position, "latitude_i") as i32 as f64 * 1e-7,
        p::number(position, "longitude_i") as i32 as f64 * 1e-7,
    ))
}
fn enum_label(msg: &DynamicMessage, field: &str, value: i32) -> String {
    msg.descriptor()
        .get_field_by_name(field)
        .and_then(|f| {
            if let prost_reflect::Kind::Enum(e) = f.kind() {
                e.get_value(value).map(|v| v.name().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| value.to_string())
}
fn emit(events: &Sender<Event>, event: Event) {
    let _ = events.send(event);
}
fn write(link: &Link, message: DynamicMessage) -> Result<()> {
    link.send(message.encode_to_vec())
}

async fn run(mut commands: mpsc::UnboundedReceiver<Command>, events: Sender<Event>) {
    let mut link: Option<Link> = None;
    let mut session = Session::default();
    let mut handshake_deadline: Option<Instant> = None;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    let mut last_heartbeat = Instant::now();
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match command {
                    Command::Connect(config) => {
                        if let Some(old) = link.take() { old.close().await; }
                        emit(&events, Event::Connecting(config.label()));
                        session = Session { nonce: rand::random::<u32>().max(1), label: config.label(), ..Session::default() };
                        match tokio::time::timeout(Duration::from_secs(25), transport::connect(&config)).await {
                            Ok(Ok(new_link)) => {
                                if let Err(error) = write(&new_link, p::to_radio("want_config_id", Value::U32(session.nonce))) {
                                    emit(&events, Event::Disconnected(error.to_string())); new_link.close().await;
                                } else { link = Some(new_link); handshake_deadline = Some(Instant::now() + Duration::from_secs(25)); }
                            }
                            Ok(Err(error)) => emit(&events, Event::Disconnected(format!("{error:#}"))),
                            Err(_) => emit(&events, Event::Disconnected("Connection timed out".into())),
                        }
                    }
                    Command::Disconnect => {
                        if let Some(old) = link.take() {
                            let _ = write(&old, p::to_radio("disconnect", Value::Bool(true)));
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            old.close().await;
                        }
                        session = Session::default(); handshake_deadline = None;
                        emit(&events, Event::Disconnected("Disconnected".into()));
                    }
                    Command::ScanBluetooth => {
                        let events = events.clone();
                        tokio::spawn(async move {
                            match transport::scan_bluetooth().await {
                                Ok(devices) => emit(&events, Event::BluetoothDevices(devices)),
                                Err(error) => emit(&events, Event::Error(format!("{error:#}"))),
                            }
                        });
                    }
                    command => {
                        let Some(link) = link.as_ref() else { emit(&events, Event::Error("Connect to a node first".into())); continue; };
                        let result: Result<()> = (|| {
                            match command {
                                Command::SendText { text, to, channel } => {
                                    if !session.ready { bail!("Wait for the connection to finish"); }
                                    if text.trim().is_empty() { bail!("Enter a message"); }
                                    if text.len() > 228 { bail!("Messages must be at most 228 UTF-8 bytes"); }
                                    let (id, packet) = p::mesh_packet(session.own, to, channel, p::TEXT_PORT, text.as_bytes().to_vec(), false);
                                    write(link, packet)?;
                                    emit(&events, Event::Message(ChatMessage { id, from: session.own, to, channel, text,
                                        timestamp: chrono::Utc::now().timestamp(), outgoing: true, status: "Sent to transport · awaiting radio acknowledgement".into() }));
                                }
                                Command::ReadSetting(key) => {
                                    write(link, session.admin(settings::admin_request(&key)?, true)?.1)?;
                                    session.reads.insert(key, Instant::now() + Duration::from_secs(12));
                                }
                                Command::SaveSetting { key, message } => {
                                    if !session.ready { bail!("Connect and wait for the node settings first"); }
                                    if session.waiting_save.is_some() { bail!("Another setting is waiting for an admin session"); }
                                    if session.passkey_time.is_none_or(|time| time.elapsed() > Duration::from_secs(240)) {
                                        write(link, session.admin(settings::admin_request("user")?, true)?.1)?;
                                        session.waiting_save = Some((key, message, Instant::now() + Duration::from_secs(15)));
                                    } else { write(link, session.start_save(key, message)?)?; }
                                }
                                _ => unreachable!(),
                            }
                            Ok(())
                        })();
                        if let Err(error) = result { emit(&events, Event::Error(format!("{error:#}"))); }
                    }
                }
            }
            incoming = async { match link.as_mut() { Some(link) => link.incoming.recv().await, None => std::future::pending().await } } => {
                match incoming {
                    Some(Ok(bytes)) => match p::decode("FromRadio", &bytes).and_then(|packet| session.handle(packet, &events)) {
                        Ok(packets) => {
                            if session.ready { handshake_deadline = None; }
                            for packet in packets { if let Some(link) = link.as_ref() && let Err(error) = write(link, packet) { emit(&events, Event::Error(error.to_string())); } }
                        }
                        Err(error) => emit(&events, Event::Error(format!("{error:#}"))),
                    },
                    failure => {
                        let error = match failure { Some(Err(error)) => format!("{error:#}"), _ => "Radio connection closed".into() };
                        if let Some(old) = link.take() { old.close().await; }
                        session = Session::default(); handshake_deadline = None;
                        emit(&events, Event::Disconnected(error));
                    }
                }
            }
            _ = heartbeat.tick() => {
                if handshake_deadline.is_some_and(|time| Instant::now() > time) {
                    if let Some(old) = link.take() { old.close().await; }
                    session = Session::default(); handshake_deadline = None;
                    emit(&events, Event::Disconnected("Node did not finish the Meshtastic handshake. Check its firmware and close other clients".into()));
                }
                if last_heartbeat.elapsed() > Duration::from_secs(15) {
                    if let Some(link) = link.as_ref() { let _ = write(link, p::to_radio("heartbeat", Value::Message(p::message("Heartbeat")))); }
                    last_heartbeat = Instant::now();
                }
                session.saves.retain(|_, save| {
                    if Instant::now() > save.deadline {
                        emit(&events, Event::SaveFailed { key: save.key.clone(), error: "No acknowledgement received. Reconnect and reload to check whether the node applied the change".into() }); false
                    } else { true }
                });
                session.reads.retain(|key, deadline| {
                    if Instant::now() > *deadline { emit(&events, Event::ReadFailed { key: key.clone() }); false } else { true }
                });
                if session.waiting_save.as_ref().is_some_and(|(_, _, deadline)| Instant::now() > *deadline) {
                    let (key, _, _) = session.waiting_save.take().unwrap();
                    emit(&events, Event::SaveFailed { key, error: "The node did not return an admin session key; no settings were written".into() });
                }
            }
        }
    }
    if let Some(old) = link.take() {
        old.close().await;
    }
}

/// Real read-only device verification, without starting GTK or changing the radio.
pub fn probe_usb() -> gtk::glib::ExitCode {
    let result: Result<()> = (|| {
        let port = std::env::args().skip_while(|arg| arg != "--probe-usb").nth(1)
            .or_else(|| serialport::available_ports().ok()?.into_iter().find(|port| matches!(&port.port_type, serialport::SerialPortType::UsbPort(info) if info.vid != 0x04e8)).map(|port| port.port_name))
            .context("No USB radio found")?;
        let (tx, rx) = std::sync::mpsc::channel();
        let backend = Backend::start(tx);
        backend
            .commands
            .send(Command::Connect(Connection::Usb { port, baud: 115200 }))?;
        let mut nodes = 0;
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            if Instant::now() > deadline {
                bail!("Probe timed out");
            }
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Event::Node(_)) => nodes += 1,
                Ok(Event::Ready { own, label }) => {
                    println!(
                        "Handshake successful: {label}; own node !{own:08x}; {nodes} nodes received"
                    );
                    break;
                }
                Ok(Event::Disconnected(error)) => bail!("{error}"),
                Ok(Event::Error(error)) => eprintln!("{error}"),
                _ => {}
            }
        }
        backend.commands.send(Command::Disconnect)?;
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(3)) {
            if matches!(event, Event::Disconnected(_)) {
                break;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => gtk::glib::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            gtk::glib::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_matching_handshake_can_mark_ready() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut session = Session {
            own: 123,
            nonce: 42,
            ..Session::default()
        };
        let mut from = p::message("FromRadio");
        put(&mut from, "config_complete_id", Value::U32(41));
        session.handle(from, &tx).unwrap();
        assert!(!session.ready);
        assert!(rx.try_recv().is_err());
        let mut from = p::message("FromRadio");
        put(&mut from, "config_complete_id", Value::U32(42));
        session.handle(from, &tx).unwrap();
        assert!(session.ready);
        assert!(matches!(
            rx.try_recv().unwrap(),
            Event::Ready { own: 123, .. }
        ));
    }
    #[test]
    fn save_includes_admin_session_and_correct_destination() {
        let mut session = Session {
            own: 123,
            ready: true,
            passkey: vec![1, 2, 3],
            ..Session::default()
        };
        let config = p::message("Config.BluetoothConfig");
        let packet = session
            .start_save("config.bluetooth".into(), config)
            .unwrap();
        let mesh = p::nested(&packet, "packet").unwrap();
        assert_eq!(p::number(&mesh, "to"), 123);
        let data = p::nested(&mesh, "decoded").unwrap();
        let admin = p::decode("AdminMessage", &p::bytes(&data, "payload")).unwrap();
        assert_eq!(p::bytes(&admin, "session_passkey"), vec![1, 2, 3]);
        assert!(p::has(&admin, "set_config"));
    }

    async fn wait_event(
        events: &std::sync::mpsc::Receiver<Event>,
        check: impl Fn(&Event) -> bool,
    ) -> Event {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            while let Ok(event) = events.try_recv() {
                if check(&event) {
                    return event;
                }
            }
            assert!(
                Instant::now() < until,
                "Timed out waiting for backend event"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn from_radio(field: &str, value: Value) -> DynamicMessage {
        let mut from = p::message("FromRadio");
        put(&mut from, field, value);
        from
    }
    fn admin_reply(admin: DynamicMessage, request_id: u32) -> DynamicMessage {
        let (_, to) = p::mesh_packet(123, 123, 0, p::ADMIN_PORT, admin.encode_to_vec(), false);
        let mut mesh = p::nested(&to, "packet").unwrap();
        let mut data = p::nested(&mesh, "decoded").unwrap();
        put(&mut data, "request_id", Value::U32(request_id));
        put(&mut mesh, "decoded", Value::Message(data));
        from_radio("packet", Value::Message(mesh))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tcp_firmware_round_trip_handshake_text_settings_ack_and_disconnect() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let firmware = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut decoder = p::FrameDecoder::default();
            let mut buffer = [0u8; 2048];
            let mut bluetooth = p::message("Config.BluetoothConfig");
            put(&mut bluetooth, "enabled", Value::Bool(true));
            let mut text_seen = false;
            let mut save_seen = false;
            loop {
                let len = socket.read(&mut buffer).await.unwrap();
                if len == 0 {
                    break;
                }
                for bytes in decoder.feed(&buffer[..len]) {
                    let packet = p::decode("ToRadio", &bytes).unwrap();
                    let mut replies = vec![];
                    if p::has(&packet, "want_config_id") {
                        let mut own = p::message("MyNodeInfo");
                        put(&mut own, "my_node_num", Value::U32(123));
                        let mut user = p::message("User");
                        put(&mut user, "long_name", Value::String("Test radio".into()));
                        let mut info = p::message("NodeInfo");
                        put(&mut info, "num", Value::U32(123));
                        put(&mut info, "user", Value::Message(user));
                        replies.push(from_radio("my_info", Value::Message(own)));
                        replies.push(from_radio("node_info", Value::Message(info)));
                        replies.push(from_radio(
                            "config_complete_id",
                            Value::U32(p::number(&packet, "want_config_id")),
                        ));
                    }
                    if let Some(mesh) = p::nested(&packet, "packet") {
                        let data = p::nested(&mesh, "decoded").unwrap();
                        if p::number(&data, "portnum") == p::TEXT_PORT as u32 {
                            assert_eq!(p::bytes(&data, "payload"), b"Hello mesh");
                            assert_eq!(p::number(&mesh, "to"), p::BROADCAST);
                            text_seen = true;
                        } else if p::number(&data, "portnum") == p::ADMIN_PORT as u32 {
                            let admin =
                                p::decode("AdminMessage", &p::bytes(&data, "payload")).unwrap();
                            let mut response = p::message("AdminMessage");
                            put(
                                &mut response,
                                "session_passkey",
                                Value::Bytes(vec![7, 8, 9].into()),
                            );
                            if p::has(&admin, "get_owner_request") {
                                put(
                                    &mut response,
                                    "get_owner_response",
                                    Value::Message(p::message("User")),
                                );
                            }
                            if p::has(&admin, "get_config_request") {
                                let mut config = p::message("Config");
                                put(&mut config, "bluetooth", Value::Message(bluetooth.clone()));
                                put(&mut response, "get_config_response", Value::Message(config));
                            }
                            if let Some(config) = p::nested(&admin, "set_config") {
                                assert_eq!(p::bytes(&admin, "session_passkey"), vec![7, 8, 9]);
                                bluetooth = p::nested(&config, "bluetooth").unwrap();
                                assert!(!p::flag(&bluetooth, "enabled"));
                                save_seen = true;
                                let mut routing = p::message("Routing");
                                put(&mut routing, "error_reason", Value::EnumNumber(0));
                                let (_, to) =
                                    p::mesh_packet(123, 123, 0, 5, routing.encode_to_vec(), false);
                                let mut reply_mesh = p::nested(&to, "packet").unwrap();
                                let mut reply_data = p::nested(&reply_mesh, "decoded").unwrap();
                                put(
                                    &mut reply_data,
                                    "request_id",
                                    Value::U32(p::number(&mesh, "id")),
                                );
                                put(&mut reply_mesh, "decoded", Value::Message(reply_data));
                                replies.push(from_radio("packet", Value::Message(reply_mesh)));
                            } else {
                                replies.push(admin_reply(response, p::number(&mesh, "id")));
                            }
                        }
                    }
                    for reply in replies {
                        socket
                            .write_all(&p::encode_frame(&reply.encode_to_vec()).unwrap())
                            .await
                            .unwrap();
                    }
                }
            }
            (text_seen, save_seen)
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let (events, receiver) = std::sync::mpsc::channel();
        let actor = tokio::spawn(run(rx, events));
        tx.send(Command::Connect(Connection::Wifi {
            host: "127.0.0.1".into(),
            port,
        }))
        .unwrap();
        assert!(matches!(
            wait_event(&receiver, |e| matches!(e, Event::Ready { .. })).await,
            Event::Ready { own: 123, .. }
        ));
        tx.send(Command::ReadSetting("config.bluetooth".into()))
            .unwrap();
        let event = wait_event(
            &receiver,
            |e| matches!(e, Event::Setting { key, .. } if key == "config.bluetooth"),
        )
        .await;
        let Event::Setting { mut message, .. } = event else {
            unreachable!()
        };
        assert!(p::flag(&message, "enabled"));
        tx.send(Command::SendText {
            text: "Hello mesh".into(),
            to: p::BROADCAST,
            channel: 0,
        })
        .unwrap();
        wait_event(&receiver, |e| matches!(e, Event::Message(_))).await;
        put(&mut message, "enabled", Value::Bool(false));
        tx.send(Command::SaveSetting {
            key: "config.bluetooth".into(),
            message,
        })
        .unwrap();
        wait_event(
            &receiver,
            |e| matches!(e, Event::Saved { key } if key == "config.bluetooth"),
        )
        .await;
        let event = wait_event(
            &receiver,
            |e| matches!(e, Event::Setting { key, .. } if key == "config.bluetooth"),
        )
        .await;
        let Event::Setting { message, .. } = event else {
            unreachable!()
        };
        assert!(!p::flag(&message, "enabled"));
        tx.send(Command::Disconnect).unwrap();
        wait_event(&receiver, |e| matches!(e, Event::Disconnected(_))).await;
        drop(tx);
        actor.await.unwrap();
        assert_eq!(firmware.await.unwrap(), (true, true));
    }
}
