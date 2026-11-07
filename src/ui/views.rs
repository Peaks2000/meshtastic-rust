use super::*;
use crate::{
    flashing,
    protocol::{self as p},
    settings,
};
use std::path::PathBuf;

pub fn nodes_page(ui: &Rc<Ui>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    margins(&page, 24, 28);
    page.append(&ui.nodes_search);
    page.append(&scroll(&ui.nodes_list));
    let weak = Rc::downgrade(ui);
    ui.nodes_search.connect_search_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.refresh_nodes();
        }
    });
    page
}

pub fn messages_page(ui: &Rc<Ui>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    margins(&page, 24, 28);
    let filters = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    ui.recipient.set_hexpand(true);
    ui.recipient.append(Some("broadcast"), "Channel messages");
    ui.recipient.set_active(Some(0));
    ui.channel.set_hexpand(true);
    ui.refresh_channels();
    filters.append(&ui.recipient);
    filters.append(&ui.channel);
    let add = button("Add", "");
    let add_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    add_content.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    add_content.append(&label("Add", ""));
    add.set_child(Some(&add_content));
    add.set_tooltip_text(Some("Start a direct conversation with a node"));
    filters.append(&add);
    let weak = Rc::downgrade(ui);
    add.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            node_picker(&ui).present();
        }
    });
    page.append(&filters);
    let messages = scroll(&ui.chat_list);
    page.append(&messages);
    let compose = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let text = gtk::Entry::builder()
        .placeholder_text("Write a message to the mesh…")
        .hexpand(true)
        .max_length(228)
        .build();
    let send = button("Send", "suggested-action");
    compose.append(&text);
    compose.append(&send);
    page.append(&compose);
    let count = label("0 / 228 bytes", "muted");
    page.append(&count);
    text.connect_changed(move |entry| {
        count.set_text(&format!("{} / 228 UTF-8 bytes", entry.text().len()));
    });
    let weak = Rc::downgrade(ui);
    let entry = text.clone();
    send.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            send_message(&ui, &entry);
        }
    });
    let weak = Rc::downgrade(ui);
    text.connect_activate(move |entry| {
        if let Some(ui) = weak.upgrade() {
            send_message(&ui, entry);
        }
    });
    let weak = Rc::downgrade(ui);
    ui.recipient.connect_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.refresh_messages();
        }
    });
    let weak = Rc::downgrade(ui);
    ui.channel.connect_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.refresh_messages();
        }
    });
    // Stay at the latest message as the history grows.
    let adjustment = messages.vadjustment();
    let scroll_to = adjustment.clone();
    adjustment.connect_upper_notify(move |_| {
        scroll_to.set_value((scroll_to.upper() - scroll_to.page_size()).max(0.0));
    });
    page
}

fn node_picker(ui: &Rc<Ui>) -> gtk::Dialog {
    let dialog = gtk::Dialog::builder()
        .transient_for(&ui.window)
        .modal(true)
        .title("New conversation")
        .default_width(500)
        .default_height(480)
        .build();
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    let content = dialog.content_area();
    content.set_spacing(16);
    margins(&content, 20, 20);
    content.append(&heading("Choose a node"));
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search names or node IDs…")
        .build();
    content.append(&search);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .css_classes(["boxed-list"])
        .build();
    let empty = label("", "muted");
    empty.set_wrap(true);
    content.append(&empty);
    let scroller = scroll(&list);
    scroller.set_min_content_height(240);
    content.append(&scroller);
    let nodes = {
        let state = ui.state.borrow();
        let mut nodes = state
            .nodes
            .values()
            .filter(|node| node.num != state.own)
            .cloned()
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| {
            (
                std::cmp::Reverse(node.last_heard),
                node.name().to_lowercase(),
            )
        });
        nodes
    };
    let populate = {
        let list = list.clone();
        let empty = empty.clone();
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let query = query.trim().to_lowercase();
            let mut count = 0;
            for node in &nodes {
                if !format!("{} {} !{:08x}", node.name(), node.short_name, node.num)
                    .to_lowercase()
                    .contains(&query)
                {
                    continue;
                }
                let row = gtk::ListBoxRow::new();
                row.set_widget_name(&node.num.to_string());
                let detail = heading(&node.name());
                detail.append(&label(
                    &format!("{} · !{:08x}", node.short_name, node.num),
                    "muted",
                ));
                margins(&detail, 12, 14);
                row.set_child(Some(&detail));
                list.append(&row);
                count += 1;
            }
            empty.set_visible(count == 0);
            empty.set_text(if nodes.is_empty() {
                "No other nodes yet. Connect to a radio to load its node list."
            } else {
                "No matching nodes. Try another name or node ID."
            });
        }
    };
    populate("");
    search.connect_search_changed(move |entry| populate(entry.text().as_str()));
    let weak = Rc::downgrade(ui);
    let picker = dialog.downgrade();
    list.connect_row_activated(move |_, row| {
        if let Some(ui) = weak.upgrade() {
            // Refresh first in case new node information arrived while choosing.
            ui.refresh_nodes();
            if ui.recipient.set_active_id(Some(row.widget_name().as_str())) {
                ui.navigate("messages");
                ui.refresh_messages();
                if let Some(dialog) = picker.upgrade() {
                    dialog.close();
                }
            }
        }
    });
    dialog.connect_response(|dialog, _| dialog.close());
    dialog
}

fn send_message(ui: &Ui, entry: &gtk::Entry) {
    if !ui.state.borrow().connected {
        ui.notice("Connect to a node before sending messages.");
        return;
    }
    let text = entry.text().to_string();
    if text.trim().is_empty() {
        return;
    }
    if text.len() > 228 {
        ui.notice("The message is longer than 228 UTF-8 bytes. Shorten it before sending.");
        return;
    }
    let to = ui
        .recipient
        .active_id()
        .and_then(|id| id.parse().ok())
        .unwrap_or(p::BROADCAST);
    let channel = ui
        .channel
        .active_id()
        .and_then(|id| id.parse().ok())
        .unwrap_or(0);
    ui.send(Command::SendText { text, to, channel });
    entry.set_text("");
}

impl Ui {
    pub fn refresh_nodes(&self) {
        while let Some(child) = self.nodes_list.first_child() {
            self.nodes_list.remove(&child);
        }
        let state = self.state.borrow();
        let filter = self.nodes_search.text().to_lowercase();
        let mut nodes = state.nodes.values().collect::<Vec<_>>();
        nodes.sort_by_key(|node| std::cmp::Reverse(node.last_heard));
        let selected = self.recipient.active_id().map(|s| s.to_string());
        // Keep recipient IDs stable when node information changes.
        self.recipient.remove_all();
        self.recipient.append(Some("broadcast"), "Channel messages");
        for node in &nodes {
            if node.num != state.own {
                self.recipient.append(
                    Some(&node.num.to_string()),
                    &format!("Direct · {} (!{:08x})", node.name(), node.num),
                );
            }
        }
        if !selected.is_some_and(|id| self.recipient.set_active_id(Some(&id))) {
            self.recipient.set_active(Some(0));
        }
        let mut shown = 0;
        for node in nodes {
            if !format!("{} {} !{:08x}", node.long_name, node.short_name, node.num)
                .to_lowercase()
                .contains(&filter)
            {
                continue;
            }
            shown += 1;
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            row.add_css_class("node-row");
            let initials = if node.short_name.is_empty() {
                format!("{:04x}", node.num & 0xffff)
            } else {
                node.short_name.clone()
            };
            let avatar = label(&initials, "node-avatar");
            avatar.set_xalign(0.5);
            row.append(&avatar);
            let detail = gtk::Box::new(gtk::Orientation::Vertical, 6);
            detail.set_hexpand(true);
            detail.append(&label(
                &format!(
                    "{}{}",
                    node.name(),
                    if node.num == state.own {
                        " · This node"
                    } else {
                        ""
                    }
                ),
                "field-label",
            ));
            let mut bits = vec![format!("!{:08x}", node.num)];
            if state.prefs.show_role && !node.role.is_empty() {
                bits.push(settings::title(&node.role.to_lowercase()));
            }
            if state.prefs.show_signal {
                bits.push(format!("{:.1} dB SNR", node.snr));
            }
            if state.prefs.show_hops
                && let Some(hops) = node.hops
            {
                bits.push(format!("{hops} hops"));
            }
            let metadata = label(&bits.join("  ·  "), "muted");
            metadata.set_wrap(true);
            detail.append(&metadata);
            if !state.prefs.compact_nodes
                && state.prefs.show_position
                && let Some((lat, lon)) = node.position
            {
                detail.append(&label(&format!("{lat:.5}, {lon:.5}"), "muted"));
            }
            row.append(&detail);
            let right = gtk::Box::new(gtk::Orientation::Vertical, 6);
            if state.prefs.show_power
                && let Some(battery) = node.battery
            {
                right.append(&label(
                    &if battery > 100 {
                        "USB power".into()
                    } else {
                        format!("{battery}% battery")
                    },
                    "muted",
                ));
            }
            if state.prefs.show_last_heard && node.last_heard != 0 {
                let time = if state.prefs.relative_time {
                    relative_time(node.last_heard as i64)
                } else {
                    format_time(node.last_heard as i64)
                };
                right.append(&label(&time, "muted"));
            }
            row.append(&right);
            self.nodes_list.append(&row);
        }
        if shown == 0 {
            let empty = heading(if state.nodes.is_empty() {
                "No nodes"
            } else {
                "No matching nodes"
            });
            margins(&empty, 40, 24);
            self.nodes_list.append(&empty);
        }
    }
    pub fn refresh_channels(&self) {
        let selected = self.channel.active_id().map(|s| s.to_string());
        self.channel.remove_all();
        let state = self.state.borrow();
        let mut count = 0;
        for index in 0..8 {
            if let Some(channel) = state.settings.get(&format!("channel.{index}")) {
                if p::number(channel, "role") == 0 {
                    continue;
                }
                let name = p::nested(channel, "settings")
                    .map(|s| p::string(&s, "name"))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        if index == 0 {
                            "Primary".into()
                        } else {
                            "Secondary".into()
                        }
                    });
                self.channel.append(
                    Some(&index.to_string()),
                    &format!("{name} · channel {index}"),
                );
                count += 1;
            }
        }
        if count == 0 {
            self.channel.append(Some("0"), "Primary · channel 0");
        }
        drop(state);
        if !selected.is_some_and(|id| self.channel.set_active_id(Some(&id))) {
            self.channel.set_active(Some(0));
        }
    }
    pub fn refresh_messages(&self) {
        clear_box(&self.chat_list);
        let state = self.state.borrow();
        let peer = self
            .recipient
            .active_id()
            .and_then(|id| id.parse::<u32>().ok());
        let channel = self
            .channel
            .active_id()
            .and_then(|id| id.parse::<u32>().ok())
            .unwrap_or(0);
        let mut shown = 0;
        for message in &state.messages {
            let visible = if let Some(peer) = peer {
                message.to == peer || (message.from == peer && message.to != p::BROADCAST)
            } else {
                message.to == p::BROADCAST && message.channel == channel
            };
            if !visible {
                continue;
            }
            shown += 1;
            let bubble = gtk::Box::new(gtk::Orientation::Vertical, 7);
            bubble.add_css_class("message");
            if message.outgoing {
                bubble.add_css_class("outgoing");
                bubble.set_margin_start(36);
            } else {
                bubble.set_margin_end(36);
            }
            let sender = if message.outgoing {
                "You".into()
            } else {
                state
                    .nodes
                    .get(&message.from)
                    .map(Node::name)
                    .unwrap_or_else(|| format!("!{:08x}", message.from))
            };
            bubble.append(&label(
                &format!("{sender}   ·   {}", format_time(message.timestamp)),
                "muted",
            ));
            let text = label(&message.text, "");
            text.set_selectable(true);
            text.set_wrap(true);
            text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            bubble.append(&text);
            if message.outgoing {
                bubble.append(&label(&message.status, "muted"));
            }
            self.chat_list.append(&bubble);
        }
        if shown == 0 {
            let empty = heading("No messages");
            margins(&empty, 50, 20);
            self.chat_list.append(&empty);
        }
    }
}
fn relative_time(time: i64) -> String {
    let seconds = (chrono::Utc::now().timestamp() - time).max(0);
    if seconds < 60 {
        "Just heard".into()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}
fn format_time(time: i64) -> String {
    chrono::DateTime::from_timestamp(time, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%d %b · %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Unknown time".into())
}

pub fn flash_page(ui: &Rc<Ui>) -> gtk::ScrolledWindow {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    margins(&page, 28, 30);
    let options = card();
    let mode = gtk::ComboBoxText::new();
    mode.append(Some("update"), "Update — keep settings");
    mode.append(Some("factory"), "Factory reset + flash (ESP32 USB)");
    mode.set_tooltip_text(Some("Both modes use an application firmware file. Factory mode clears settings after firmware verification."));
    mode.set_active(Some(0));
    options.append(&mode);
    ui.recovery_picker
        .append(Some("normal"), "Connected node / automatic UF2 drive");
    ui.recovery_picker.set_active(Some(0));
    options.append(&ui.recovery_picker);
    options.append(&ui.recovery_scan);
    let weak = Rc::downgrade(ui);
    ui.recovery_scan.connect_clicked(move |button| {
        if let Some(ui) = weak.upgrade() {
            if ui.flash_busy.get() {
                return;
            }
            button.set_sensitive(false);
            ui.flash_button.set_sensitive(false);
            ui.flash_status.set_text("Scanning recovery devices…");
            let mounts = gtk::gio::VolumeMonitor::get()
                .mounts()
                .into_iter()
                .filter_map(|mount| mount.root().path())
                .collect();
            let skip = {
                let state = ui.state.borrow();
                match &state.active_connection {
                    Some(crate::model::Connection::Usb { port, .. }) if state.connected => {
                        Some(port.clone())
                    }
                    _ => None,
                }
            };
            let events = ui.events.clone();
            std::thread::spawn(move || {
                let _ = events.send(Event::RecoveryDevices(
                    crate::recovery::scan(mounts, skip).map_err(|error| format!("{error:#}")),
                ));
            });
        }
    });
    let file = Rc::new(RefCell::new(None::<PathBuf>));
    let file_label = label("No firmware selected", "muted");
    file_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let choose_file = button("Choose firmware file…", "");
    options.append(&choose_file);
    options.append(&file_label);
    let weak = Rc::downgrade(ui);
    let chosen = file.clone();
    choose_file.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let window = ui.window.clone();
            let chosen = chosen.clone();
            let file_label = file_label.clone();
            glib::MainContext::default().spawn_local(async move {
                let dialog = gtk::FileDialog::builder()
                    .title("Choose the firmware for your exact board")
                    .build();
                let filter = gtk::FileFilter::new();
                filter.set_name(Some("Meshtastic firmware (.bin / .uf2)"));
                filter.add_pattern("*.bin");
                filter.add_pattern("*.uf2");
                dialog.set_default_filter(Some(&filter));
                if let Ok(file) = dialog.open_future(Some(&window)).await
                    && let Some(path) = file.path()
                {
                    file_label.set_text(&path.display().to_string());
                    *chosen.borrow_mut() = Some(path);
                }
            });
        }
    });
    page.append(&options);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.append(&ui.flash_button);
    let releases = button("Official firmware releases ↗", "");
    actions.append(&releases);
    page.append(&actions);
    let weak = Rc::downgrade(ui);
    releases.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade()
            && let Err(error) = gtk::gio::AppInfo::launch_default_for_uri(
                "https://github.com/meshtastic/firmware/releases",
                None::<&gtk::gio::AppLaunchContext>,
            )
        {
            ui.notice(&error.to_string());
        }
    });
    page.append(&ui.flash_status);
    let console = gtk::TextView::builder()
        .buffer(&ui.flash_log)
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .css_classes(["console"])
        .build();
    let logs = scroll(&console);
    logs.set_min_content_height(220);
    page.append(&logs);
    let weak = Rc::downgrade(ui);
    ui.flash_button.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let Some(firmware) = file.borrow().clone() else {
                ui.notice("Choose a firmware file first.");
                return;
            };
            let target = {
                let state = ui.state.borrow();
                match (&state.active_connection, state.nodes.get(&state.own)) {
                    (Some(crate::model::Connection::Usb { port, .. }), Some(node))
                        if state.connected =>
                    {
                        Some(flashing::UsbTarget {
                            board: node.hardware.clone(),
                            node: node.num,
                            port: port.clone(),
                            recovery_chip: None,
                        })
                    }
                    _ => None,
                }
            };
            let mounts = gtk::gio::VolumeMonitor::get()
                .mounts()
                .into_iter()
                .filter_map(|mount| mount.root().path())
                .collect::<Vec<_>>();
            let selected_mode = if mode.active_id().as_deref() == Some("factory") { flashing::FlashMode::Factory } else { flashing::FlashMode::Update };
            let recovery = ui.recovery_picker.active_id().and_then(|id| id.parse::<usize>().ok()).and_then(|index| ui.recovery_devices.borrow().get(index).cloned());
            let result = if let Some(device) = recovery.as_ref() { flashing::prepare_recovery(&firmware, device, selected_mode) } else { flashing::prepare(&firmware, target, &mounts, selected_mode) };
            let plan = match result {
                Ok(plan) => plan,
                Err(error) => {
                    ui.notice(&format!("{error:#}"));
                    return;
                }
            };
            let dialog = gtk::MessageDialog::builder()
                .transient_for(&ui.window)
                .modal(true)
                .message_type(gtk::MessageType::Question)
                .buttons(gtk::ButtonsType::OkCancel)
                .text(if plan.is_factory() { "Factory reset and flash?" } else { "Update firmware?" })
                .secondary_text(format!(
                    "{}\n\n{}",
                    plan.summary(),
                    if recovery.is_some() { "Recovery: the bootloader may only identify the chip, not the board model. Confirm this firmware is for your exact board. Factory mode clears settings. Keep USB connected." } else if plan.is_factory() { "This clears settings, channels, keys, and saved node data after the firmware is verified. Keep USB connected." } else { "Settings will be kept. Keep USB connected until the update finishes." }
                ))
                .build();
            let weak = Rc::downgrade(&ui);
            dialog.connect_response(move |dialog, response| {
                if response == gtk::ResponseType::Ok
                    && let Some(ui) = weak.upgrade()
                {
                    ui.flash_log.set_text("");
                    ui.flash_busy.set(true);
                    ui.flash_button.set_sensitive(false);
                    ui.connect_button.set_sensitive(false);
                    ui.flash_status.set_text("Releasing the radio connection…");
                    *ui.pending_flash.borrow_mut() = Some(plan.clone());
                    ui.send(Command::Disconnect);
                }
                dialog.close();
            });
            dialog.present();
        }
    });
    scroll(&page)
}

pub fn smoke_test(ui: &Rc<Ui>) {
    let weak = Rc::downgrade(ui);
    glib::idle_add_local_once(move || {
        if let Some(ui) = weak.upgrade() {
            for page in ["connect", "nodes", "messages", "flash", "settings"] {
                ui.navigate(page);
                assert_eq!(ui.stack.visible_child_name().as_deref(), Some(page));
            }
            // Exercise every schema-driven form without a radio and without writing settings.
            for category in settings::categories() {
                let baseline = prost_reflect::DynamicMessage::new(category.descriptor.clone());
                let form = forms::Form::build(&category, baseline);
                assert!(form.collect().is_ok(), "Form failed: {}", category.key);
            }
            let picker = node_picker(&ui);
            picker.close();
            eprintln!(
                "GTK smoke test passed: all pages and {} settings categories",
                settings::categories().len()
            );
            ui.navigate(&std::env::var("MESHTASTIC_UI_PAGE").unwrap_or_else(|_| "connect".into()));
            let weak = Rc::downgrade(&ui);
            glib::timeout_add_local_once(Duration::from_millis(500), move || {
                if let Some(ui) = weak.upgrade() {
                    if let Some(path) = std::env::var_os("MESHTASTIC_UI_CAPTURE") {
                        ui.capture(path);
                    }
                    ui.window.close();
                }
            });
        }
    });
}
