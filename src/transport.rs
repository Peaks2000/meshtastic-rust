use crate::{
    model::Connection,
    protocol::{FrameDecoder, encode_frame},
};
use anyhow::{Context, Result, bail};
use btleplug::{
    api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType},
    platform::{Adapter, Manager, Peripheral},
};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_serial::SerialPortBuilderExt;
use uuid::Uuid;

const SERVICE: Uuid = Uuid::from_u128(0x6ba1b218_15a8_461f_9fa8_5dcae273eafd);
const FROMRADIO: Uuid = Uuid::from_u128(0x2c55e69e_4993_11ed_b878_0242ac120002);
const TORADIO: Uuid = Uuid::from_u128(0xf75c76d2_129e_4dad_a1dd_7866124401e7);
const FROMNUM: Uuid = Uuid::from_u128(0xed9da18c_a800_4f66_a670_aa7547e34453);

pub struct Link {
    pub outgoing: mpsc::Sender<Vec<u8>>,
    pub incoming: mpsc::Receiver<Result<Vec<u8>>>,
    task: JoinHandle<()>,
    ble: Option<Peripheral>,
}
impl Link {
    pub async fn close(self) {
        self.task.abort();
        let _ = self.task.await;
        if let Some(ble) = self.ble {
            let _ = ble.disconnect().await;
        }
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        self.outgoing
            .try_send(bytes)
            .context("Radio connection is busy or closed")
    }
}

pub async fn connect(config: &Connection) -> Result<Link> {
    match config {
        Connection::Usb { port, baud } => {
            let mut stream = tokio_serial::new(port, *baud)
                .open_native_async()
                .with_context(|| {
                    format!("Cannot open {port}. Close other clients and check serial permissions")
                })?;
            #[cfg(unix)]
            stream
                .set_exclusive(true)
                .context("Serial port is already in use")?;
            // Do not toggle DTR/RTS: opening the client must not put an ESP32 into bootloader mode.
            Ok(stream_link(stream))
        }
        Connection::Wifi { host, port } => {
            let stream = tokio::net::TcpStream::connect((host.as_str(), *port))
                .await
                .with_context(|| {
                    format!("Cannot reach {host}:{port}. Enable TCP in the node's Network settings")
                })?;
            stream.set_nodelay(true)?;
            Ok(stream_link(stream))
        }
        Connection::Bluetooth { device } => ble_link(device).await,
    }
}

pub fn stream_link<S>(stream: S) -> Link
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut outgoing) = mpsc::channel::<Vec<u8>>(32);
    let (incoming, rx) = mpsc::channel(128);
    let task = tokio::spawn(async move {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut decoder = FrameDecoder::default();
        let mut buffer = [0; 2048];
        let result: Result<()> = async {
            loop {
                tokio::select! {
                    read = reader.read(&mut buffer) => {
                        let len = read.context("Reading from radio")?;
                        if len == 0 { bail!("Radio disconnected"); }
                        for frame in decoder.feed(&buffer[..len]) {
                            if incoming.send(Ok(frame)).await.is_err() { return Ok(()); }
                        }
                    }
                    bytes = outgoing.recv() => {
                        let Some(bytes) = bytes else { return Ok(()); };
                        writer.write_all(&encode_frame(&bytes)?).await.context("Writing to radio")?;
                        writer.flush().await?;
                    }
                }
            }
        }
        .await;
        if let Err(error) = result {
            let _ = incoming.send(Err(error)).await;
        }
    });
    Link {
        outgoing: tx,
        incoming: rx,
        task,
        ble: None,
    }
}

async fn adapters() -> Result<&'static Vec<Adapter>> {
    static ADAPTERS: tokio::sync::OnceCell<Vec<Adapter>> = tokio::sync::OnceCell::const_new();
    ADAPTERS
        .get_or_try_init(|| async {
            let manager = Manager::new()
                .await
                .context("Bluetooth is unavailable. Start Bluetooth in system settings")?;
            let adapters = manager.adapters().await?;
            if adapters.is_empty() {
                bail!("No Bluetooth adapter found");
            }
            Ok(adapters)
        })
        .await
}

pub async fn scan_bluetooth() -> Result<Vec<(String, String)>> {
    let mut devices = vec![];
    for adapter in adapters().await? {
        adapter
            .start_scan(ScanFilter {
                services: vec![SERVICE],
            })
            .await?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        let found = adapter.peripherals().await;
        let _ = adapter.stop_scan().await;
        for peripheral in found? {
            if let Some(props) = peripheral.properties().await?
                && props.services.contains(&SERVICE)
            {
                devices.push((
                    peripheral.id().to_string(),
                    props
                        .local_name
                        .unwrap_or_else(|| peripheral.address().to_string()),
                ));
            }
        }
    }
    devices.sort();
    devices.dedup();
    Ok(devices)
}

async fn ble_link(device: &str) -> Result<Link> {
    let mut selected = None;
    for adapter in adapters().await? {
        adapter
            .start_scan(ScanFilter {
                services: vec![SERVICE],
            })
            .await?;
        tokio::time::sleep(Duration::from_secs(3)).await;
        let found = adapter.peripherals().await;
        let _ = adapter.stop_scan().await;
        for peripheral in found? {
            let name = peripheral
                .properties()
                .await?
                .and_then(|p| p.local_name)
                .unwrap_or_default();
            if peripheral.id().to_string() == device
                || peripheral
                    .address()
                    .to_string()
                    .eq_ignore_ascii_case(device)
                || name == device
            {
                selected = Some(peripheral);
                break;
            }
        }
        if selected.is_some() {
            break;
        }
    }
    let radio = selected
        .context("Bluetooth node not found. Scan again, or enter its exact name/address")?;
    let setup = async {
        radio.connect().await.context(
            "Cannot connect over Bluetooth. Pair the node in system Bluetooth settings first",
        )?;
        radio.discover_services().await?;
        let chars = radio.characteristics();
        let to = chars
            .iter()
            .find(|c| c.uuid == TORADIO)
            .context("Missing Meshtastic ToRadio characteristic")?
            .clone();
        let from = chars
            .iter()
            .find(|c| c.uuid == FROMRADIO)
            .context("Missing Meshtastic FromRadio characteristic")?
            .clone();
        let num = chars
            .iter()
            .find(|c| c.uuid == FROMNUM)
            .context("Missing Meshtastic FromNum characteristic")?
            .clone();
        radio
            .subscribe(&num)
            .await
            .context("Bluetooth subscription failed; check pairing/PIN")?;
        let notifications = radio.notifications().await?;
        Ok::<_, anyhow::Error>((to, from, notifications))
    }
    .await;
    let (to, from, mut notifications) = match setup {
        Ok(value) => value,
        Err(error) => {
            let _ = radio.disconnect().await;
            return Err(error);
        }
    };
    let (tx, mut outgoing) = mpsc::channel::<Vec<u8>>(32);
    let (incoming, rx) = mpsc::channel(128);
    let device = radio.clone();
    let task = tokio::spawn(async move {
        let mut poll = tokio::time::interval(Duration::from_millis(750));
        let result: Result<()> = async {
            loop {
                tokio::select! {
                    bytes = outgoing.recv() => {
                        let Some(bytes) = bytes else { return Ok(()); };
                        device.write(&to, &bytes, WriteType::WithResponse).await?;
                    }
                    notification = notifications.next() => {
                        if notification.is_none() { bail!("Bluetooth disconnected"); }
                        drain_ble(&device, &from, &incoming).await?;
                    }
                    _ = poll.tick() => {
                        if !device.is_connected().await? { bail!("Bluetooth disconnected"); }
                        drain_ble(&device, &from, &incoming).await?;
                    }
                }
            }
        }
        .await;
        if let Err(error) = result {
            let _ = incoming.send(Err(error)).await;
        }
        let _ = device.disconnect().await;
    });
    Ok(Link {
        outgoing: tx,
        incoming: rx,
        task,
        ble: Some(radio),
    })
}

async fn drain_ble(
    radio: &Peripheral,
    characteristic: &btleplug::api::Characteristic,
    tx: &mpsc::Sender<Result<Vec<u8>>>,
) -> Result<()> {
    for _ in 0..64 {
        let data = radio.read(characteristic).await?;
        if data.is_empty() {
            break;
        }
        if tx.send(Ok(data)).await.is_err() {
            break;
        }
    }
    Ok(())
}
