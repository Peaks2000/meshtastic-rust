use super::*;
use crate::model::Connection;

pub fn page(ui: &Rc<Ui>) -> gtk::ScrolledWindow {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 22);
    margins(&page, 28, 30);
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let content = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::None)
        .transition_duration(0)
        .build();
    let mut group: Option<gtk::ToggleButton> = None;
    for (name, text) in [
        ("usb", "USB serial"),
        ("wifi", "Wi-Fi / TCP"),
        ("bluetooth", "Bluetooth LE"),
    ] {
        let button = gtk::ToggleButton::with_label(text);
        button.add_css_class("transport-tab");
        if let Some(first) = group.as_ref() {
            button.set_group(Some(first));
        } else {
            group = Some(button.clone());
            button.set_active(true);
        }
        let stack = content.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                stack.set_visible_child_name(name);
            }
        });
        tabs.append(&button);
    }
    page.append(&tabs);
    let usb = card();
    usb.append(&heading("USB"));
    let port = gtk::ComboBoxText::with_entry();
    port.set_hexpand(true);
    let refresh = button("Refresh ports", "");
    let ports = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    ports.append(&port);
    ports.append(&refresh);
    usb.append(&label("Serial port", "field-label"));
    usb.append(&ports);
    refresh_ports(&port);
    let baud = gtk::ComboBoxText::new();
    for value in [115200, 921600, 460800, 57600] {
        baud.append(Some(&value.to_string()), &value.to_string());
    }
    baud.set_active(Some(0));
    usb.append(&label("Baud rate", "field-label"));
    usb.append(&baud);
    let weak = Rc::downgrade(ui);
    let port_refresh = port.clone();
    refresh.connect_clicked(move |_| {
        refresh_ports(&port_refresh);
        if let Some(ui) = weak.upgrade() {
            ui.log("USB serial port list refreshed");
        }
    });
    content.add_named(&usb, Some("usb"));
    let wifi = card();
    wifi.append(&heading("Wi-Fi"));
    let host = gtk::Entry::builder()
        .placeholder_text("meshtastic.local or 192.168.1.50")
        .build();
    let tcp_port = gtk::Entry::builder()
        .text("4403")
        .input_purpose(gtk::InputPurpose::Digits)
        .build();
    wifi.append(&label("Hostname or IP address", "field-label"));
    wifi.append(&host);
    wifi.append(&label("TCP port", "field-label"));
    wifi.append(&tcp_port);
    content.add_named(&wifi, Some("wifi"));
    let ble = card();
    ble.append(&heading("Bluetooth"));
    ble.append(&ui.bluetooth_devices);
    ble.append(&ui.scan_button);
    let weak = Rc::downgrade(ui);
    ui.scan_button.connect_clicked(move |button| {
        if let Some(ui) = weak.upgrade() {
            button.set_sensitive(false);
            ui.log("Scanning for Meshtastic Bluetooth nodes…");
            ui.send(Command::ScanBluetooth);
        }
    });
    content.add_named(&ble, Some("bluetooth"));
    page.append(&content);
    // Restore the most recent endpoint without starting a connection unless enabled in app settings.
    if let Some(last) = &ui.state.borrow().prefs.last_connection {
        match last {
            Connection::Usb {
                port: value,
                baud: value_baud,
            } => {
                set_combo_entry(&port, value);
                baud.set_active_id(Some(&value_baud.to_string()));
            }
            Connection::Wifi { host: value, port } => {
                host.set_text(value);
                tcp_port.set_text(&port.to_string());
            }
            Connection::Bluetooth { device } => {
                set_combo_entry(&ui.bluetooth_devices, device);
            }
        }
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.append(&ui.connect_button);
    let disconnect = button("Disconnect", "");
    actions.append(&disconnect);
    page.append(&actions);
    let weak = Rc::downgrade(ui);
    disconnect.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.send(Command::Disconnect);
        }
    });
    let weak = Rc::downgrade(ui);
    ui.connect_button.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            if ui.flash_busy.get() {
                ui.notice("Wait for firmware flashing to finish first.");
                return;
            }
            let connection = match content.visible_child_name().as_deref() {
                Some("wifi") => {
                    let host = host.text().trim().to_string();
                    if host.is_empty() {
                        ui.notice("Enter the node’s hostname or IP address.");
                        return;
                    }
                    let Ok(port) = tcp_port.text().parse::<u16>() else {
                        ui.notice("Enter a TCP port from 1 to 65535.");
                        return;
                    };
                    if port == 0 {
                        ui.notice("TCP port 0 is not valid.");
                        return;
                    }
                    Connection::Wifi { host, port }
                }
                Some("bluetooth") => {
                    let device = ui
                        .bluetooth_devices
                        .active_id()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| combo_entry(&ui.bluetooth_devices));
                    if device.trim().is_empty() {
                        ui.notice("Scan and select a Bluetooth node, or enter its name/address.");
                        return;
                    }
                    Connection::Bluetooth { device }
                }
                _ => {
                    let port = port
                        .active_id()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| combo_entry(&port));
                    if port.trim().is_empty() {
                        ui.notice("Select a USB serial port.");
                        return;
                    }
                    Connection::Usb {
                        port,
                        baud: baud
                            .active_id()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(115200),
                    }
                }
            };
            ui.state.borrow_mut().prefs.last_connection = Some(connection.clone());
            ui.save_preferences();
            ui.connect_button.set_sensitive(false);
            ui.send(Command::Connect(connection));
        }
    });
    let diagnostics = gtk::Expander::builder()
        .label("Connection activity")
        .build();
    let text = gtk::TextView::builder()
        .buffer(&ui.diagnostics)
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .css_classes(["console"])
        .build();
    let logs = scroll(&text);
    logs.set_min_content_height(160);
    diagnostics.set_child(Some(&logs));
    page.append(&diagnostics);
    scroll(&page)
}

pub fn refresh_ports(combo: &gtk::ComboBoxText) {
    let previous = combo.active_id().map(|s| s.to_string());
    combo.remove_all();
    if let Ok(mut ports) = serialport::available_ports() {
        ports.sort_by_key(|p| match &p.port_type {
            serialport::SerialPortType::UsbPort(info) if info.vid != 0x04e8 => 0,
            _ => 1,
        });
        for port in ports {
            let label = match port.port_type {
                serialport::SerialPortType::UsbPort(info) => format!(
                    "{} · {}",
                    port.port_name,
                    info.product
                        .unwrap_or_else(|| format!("USB {:04x}:{:04x}", info.vid, info.pid))
                ),
                _ => port.port_name.clone(),
            };
            combo.append(Some(&port.port_name), &label);
        }
        if !previous.is_some_and(|id| combo.set_active_id(Some(&id))) {
            combo.set_active(Some(0));
        }
    }
}
pub fn combo_entry(combo: &gtk::ComboBoxText) -> String {
    combo
        .child()
        .and_then(|child| child.downcast::<gtk::Entry>().ok())
        .map(|entry| entry.text().to_string())
        .unwrap_or_default()
}
fn set_combo_entry(combo: &gtk::ComboBoxText, text: &str) {
    if !combo.set_active_id(Some(text))
        && let Some(entry) = combo.child().and_then(|c| c.downcast::<gtk::Entry>().ok())
    {
        entry.set_text(text);
    }
}
