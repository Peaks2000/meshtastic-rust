use prost_reflect::DynamicMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Connection {
    Usb { port: String, baud: u32 },
    Wifi { host: String, port: u16 },
    Bluetooth { device: String },
}
impl Connection {
    pub fn label(&self) -> String {
        match self {
            Self::Usb { port, .. } => format!("USB · {port}"),
            Self::Wifi { host, port } => format!("Wi-Fi · {host}:{port}"),
            Self::Bluetooth { device } => format!("Bluetooth · {device}"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub num: u32,
    pub long_name: String,
    pub short_name: String,
    pub last_heard: u32,
    pub snr: f64,
    pub battery: Option<u32>,
    pub voltage: Option<f64>,
    pub position: Option<(f64, f64)>,
    pub hops: Option<u32>,
    pub role: String,
    pub hardware: String,
}
impl Node {
    pub fn name(&self) -> String {
        if self.long_name.is_empty() {
            format!("!{:08x}", self.num)
        } else {
            self.long_name.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: u32,
    pub from: u32,
    pub to: u32,
    pub channel: u32,
    pub text: String,
    pub timestamp: i64,
    pub outgoing: bool,
    pub status: String,
}

#[derive(Debug)]
pub enum Command {
    Connect(Connection),
    Disconnect,
    ScanBluetooth,
    SendText {
        text: String,
        to: u32,
        channel: u32,
    },
    ReadSetting(String),
    SaveSetting {
        key: String,
        message: DynamicMessage,
    },
}

#[derive(Debug)]
pub enum Event {
    Connecting(String),
    Ready {
        own: u32,
        label: String,
    },
    Disconnected(String),
    Node(Node),
    Setting {
        key: String,
        message: DynamicMessage,
    },
    Message(ChatMessage),
    Delivery {
        id: u32,
        status: String,
    },
    BluetoothDevices(Vec<(String, String)>),
    Saved {
        key: String,
    },
    ReadFailed {
        key: String,
    },
    SaveFailed {
        key: String,
        error: String,
    },
    Error(String),
    Log(String),
    Firmware(String),
    FlashFinished(bool),
}
