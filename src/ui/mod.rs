mod connection;
mod forms;
mod preferences;
mod views;

use crate::{
    backend::Backend,
    model::{ChatMessage, Command, Event, Node},
    storage::{self, Preferences},
};
use gtk::{glib, prelude::*};
use prost_reflect::DynamicMessage;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};

pub struct State {
    pub connected: bool,
    pub active_connection: Option<crate::model::Connection>,
    pub own: u32,
    pub nodes: BTreeMap<u32, Node>,
    pub settings: HashMap<String, DynamicMessage>,
    pub messages: Vec<ChatMessage>,
    pub prefs: Preferences,
}

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    pub commands: tokio::sync::mpsc::UnboundedSender<Command>,
    pub events: mpsc::Sender<Event>,
    pub state: RefCell<State>,
    pub stack: gtk::Stack,
    pub title: gtk::Label,
    pub status: gtk::Label,
    pub banner: gtk::Revealer,
    pub banner_text: gtk::Label,
    pub nodes_list: gtk::ListBox,
    pub nodes_search: gtk::SearchEntry,
    pub chat_list: gtk::Box,
    pub recipient: gtk::ComboBoxText,
    pub channel: gtk::ComboBoxText,
    pub diagnostics: gtk::TextBuffer,
    pub settings_content: gtk::Box,
    pub setting_status: gtk::Label,
    pub setting_save: gtk::Button,
    pub editor: RefCell<Option<forms::Form>>,
    pub selected_category: RefCell<Option<String>>,
    pub bluetooth_devices: gtk::ComboBoxText,
    pub connect_button: gtk::Button,
    pub scan_button: gtk::Button,
    pub flash_busy: Cell<bool>,
    pub flash_button: gtk::Button,
    pub flash_log: gtk::TextBuffer,
    pub flash_status: gtk::Label,
    pub pending_flash: RefCell<Option<crate::flashing::FlashPlan>>,
}

pub fn build(app: &gtk::Application) {
    let display = gtk::gdk::Display::default().expect("GTK display");
    gtk::IconTheme::for_display(&display)
        .add_resource_path("/io/github/meshtasticgtk/Client/icons");
    gtk::Window::set_default_icon_name("io.github.meshtasticgtk.Client");
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("../../assets/style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let prefs: Preferences = storage::read("preferences.json");
    apply_theme(&prefs.theme);
    let messages = storage::load_history(&prefs);
    let (events, receiver) = mpsc::channel();
    let backend = Backend::start(events.clone());
    let ui = Rc::new(Ui {
        window: gtk::ApplicationWindow::builder()
            .application(app)
            .icon_name("io.github.meshtasticgtk.Client")
            .title("Meshtastic")
            .default_width(1180)
            .default_height(820)
            .build(),
        commands: backend.commands,
        events,
        state: RefCell::new(State {
            connected: false,
            active_connection: None,
            own: 0,
            nodes: BTreeMap::new(),
            settings: HashMap::new(),
            messages,
            prefs,
        }),
        stack: gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::None)
            .transition_duration(0)
            .build(),
        title: label("Connect", "page-title"),
        status: label("Disconnected", "status"),
        banner: gtk::Revealer::new(),
        banner_text: gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .selectable(true)
            .build(),
        nodes_list: gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build(),
        nodes_search: gtk::SearchEntry::builder()
            .placeholder_text("Search names or node IDs…")
            .build(),
        chat_list: gtk::Box::new(gtk::Orientation::Vertical, 12),
        recipient: gtk::ComboBoxText::new(),
        channel: gtk::ComboBoxText::new(),
        diagnostics: gtk::TextBuffer::new(None),
        settings_content: gtk::Box::new(gtk::Orientation::Vertical, 16),
        setting_status: label("", "muted"),
        setting_save: button("Save to node", "suggested-action"),
        editor: RefCell::new(None),
        selected_category: RefCell::new(None),
        bluetooth_devices: gtk::ComboBoxText::with_entry(),
        connect_button: button("Connect to node", "suggested-action"),
        scan_button: button("Scan nearby", ""),
        flash_busy: Cell::new(false),
        flash_button: button("Flash firmware…", "suggested-action"),
        flash_log: gtk::TextBuffer::new(None),
        flash_status: label("", "muted"),
        pending_flash: RefCell::new(None),
    });
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.add_css_class("workspace");
    root.append(&sidebar(&ui));
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.set_hexpand(true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    margins(&header, 22, 28);
    header.add_css_class("page-header");
    let heading = gtk::Box::new(gtk::Orientation::Vertical, 4);
    heading.set_hexpand(true);
    heading.append(&ui.title);
    header.append(&heading);
    header.append(&ui.status);
    let controls = gtk::WindowControls::new(gtk::PackType::End);
    header.append(&controls);
    main.append(&header);
    let banner_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    margins(&banner_box, 12, 24);
    banner_box.add_css_class("notice");
    ui.banner_text.set_hexpand(true);
    banner_box.append(&ui.banner_text);
    let dismiss = gtk::Button::from_icon_name("window-close-symbolic");
    dismiss.add_css_class("flat");
    banner_box.append(&dismiss);
    ui.banner.set_child(Some(&banner_box));
    main.append(&ui.banner);
    let weak = Rc::downgrade(&ui);
    dismiss.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.banner.set_reveal_child(false);
        }
    });
    ui.stack.add_named(&connection::page(&ui), Some("connect"));
    ui.stack.add_named(&views::nodes_page(&ui), Some("nodes"));
    ui.stack
        .add_named(&views::messages_page(&ui), Some("messages"));
    ui.stack.add_named(&views::flash_page(&ui), Some("flash"));
    ui.stack
        .add_named(&forms::settings_page(&ui), Some("settings"));
    main.append(&ui.stack);
    root.append(&main);
    ui.window.set_child(Some(&root));
    ui.navigate("connect");
    let weak = Rc::downgrade(&ui);
    ui.window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade() {
            if ui.flash_busy.get() {
                ui.notice("Firmware is still being written. Wait for it to finish before closing.");
                return glib::Propagation::Stop;
            }
            ui.send(Command::Disconnect);
            ui.save_preferences();
            ui.save_history();
        }
        glib::Propagation::Proceed
    });
    // Keep the UI alive until its window closes, while callbacks hold weak references.
    let keep_alive = ui.clone();
    glib::timeout_add_local(Duration::from_millis(80), move || {
        for event in receiver.try_iter().take(250) {
            keep_alive.on_event(event);
        }
        if keep_alive.window.is_visible() {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
    ui.window.present();
    ui.refresh_nodes();
    ui.refresh_messages();
    let prefs = ui.state.borrow().prefs.clone();
    if prefs.auto_connect
        && std::env::var_os("MESHTASTIC_UI_SMOKE").is_none()
        && let Some(connection) = prefs.last_connection
    {
        ui.send(Command::Connect(connection));
    }
    if std::env::var_os("MESHTASTIC_UI_SMOKE").is_some() {
        views::smoke_test(&ui);
    } else if let Ok(port) = std::env::var("MESHTASTIC_CONNECT_USB") {
        ui.send(Command::Connect(crate::model::Connection::Usb {
            port,
            baud: 115200,
        }));
    }
}

fn sidebar(ui: &Rc<Ui>) -> gtk::Box {
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.add_css_class("sidebar");
    sidebar.set_width_request(220);
    let brand = gtk::Box::new(gtk::Orientation::Vertical, 5);
    margins(&brand, 30, 22);
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_resource_at_scale(
        "/io/github/meshtasticgtk/Client/meshtastic-wordmark.svg",
        170,
        22,
        true,
    )
    .expect("Bundled Meshtastic wordmark");
    let pixbuf = pixbuf
        .scale_simple(170, 18, gtk::gdk_pixbuf::InterpType::Bilinear)
        .expect("Compact wordmark dimensions");
    let logo = gtk::Picture::for_paintable(&gtk::gdk::Texture::for_pixbuf(&pixbuf));
    logo.set_content_fit(gtk::ContentFit::Contain);
    logo.set_size_request(170, 18);
    logo.set_halign(gtk::Align::Start);
    logo.set_tooltip_text(Some("Meshtastic"));
    brand.append(&logo);
    sidebar.append(&brand);
    let top = gtk::Box::new(gtk::Orientation::Vertical, 5);
    margins(&top, 8, 12);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 5);
    margins(&bottom, 12, 12);
    for (key, name, icon, parent) in [
        ("connect", "Connect", "network-wired-symbolic", &top),
        ("nodes", "Nodes", "network-workgroup-symbolic", &top),
        ("messages", "Messages", "mail-unread-symbolic", &top),
        (
            "flash",
            "Flashing tool",
            "drive-removable-media-symbolic",
            &bottom,
        ),
        ("settings", "Settings", "emblem-system-symbolic", &bottom),
    ] {
        let button = gtk::ToggleButton::new();
        button.add_css_class("nav-button");
        button.set_widget_name(key);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&gtk::Image::from_icon_name(icon));
        row.append(&label(name, ""));
        button.set_child(Some(&row));
        parent.append(&button);
        let weak = Rc::downgrade(ui);
        button.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.navigate(key);
            }
        });
    }
    sidebar.append(&top);
    let space = gtk::Box::new(gtk::Orientation::Vertical, 0);
    space.set_vexpand(true);
    sidebar.append(&space);
    sidebar.append(&bottom);
    sidebar
}

impl Ui {
    pub fn send(&self, command: Command) {
        if let Command::Connect(connection) = &command {
            self.state.borrow_mut().active_connection = Some(connection.clone());
        }
        if self.commands.send(command).is_err() {
            self.notice("The radio worker has stopped. Restart the app.");
        }
    }
    pub fn notice(&self, text: &str) {
        self.banner_text.set_text(text);
        self.banner.set_reveal_child(true);
        self.log(text);
    }
    pub fn log(&self, text: &str) {
        let text = format!("{}  {}\n", chrono::Local::now().format("%H:%M:%S"), text);
        append_log(&self.diagnostics, &text);
    }
    pub fn navigate(&self, name: &str) {
        self.stack.set_visible_child_name(name);
        let title = match name {
            "nodes" => "Nodes",
            "messages" => "Messages",
            "flash" => "Flashing tool",
            "settings" => "Settings",
            _ => "Connect",
        };
        self.title.set_text(title);
        if let Some(root) = self.window.child() {
            update_nav(&root, name);
        }
    }
    fn on_event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Connecting(endpoint_label) => {
                let mut state = self.state.borrow_mut();
                state.connected = false;
                state.own = 0;
                state.nodes.clear();
                state.settings.clear();
                drop(state);
                self.editor.borrow_mut().take();
                self.setting_save.set_sensitive(false);
                clear_box(&self.settings_content);
                self.settings_content
                    .append(&label("Reading configuration from the node…", "muted"));
                self.status.set_text("Connecting…");
                self.connect_button.set_sensitive(false);
                self.log(&format!("Connecting: {endpoint_label}"));
                self.refresh_nodes();
            }
            Event::Ready { own, label } => {
                let mut state = self.state.borrow_mut();
                state.connected = true;
                state.own = own;
                drop(state);
                self.status.set_text("● Connected");
                self.status.add_css_class("connected");
                self.status
                    .set_tooltip_text(Some(&format!("!{own:08x} · {label}")));
                self.connect_button.set_sensitive(false);
                self.log(&format!("Configuration loaded · !{own:08x}"));
                self.refresh_nodes();
                self.refresh_channels();
                let selected = self.selected_category.borrow().clone();
                if let Some(key) = selected {
                    self.show_category(&key);
                }
                eprintln!(
                    "Connected to !{own:08x}; {} nodes; {} settings categories loaded",
                    self.state.borrow().nodes.len(),
                    self.state.borrow().settings.len()
                );
                if let Some(path) = std::env::var_os("MESHTASTIC_LIVE_CAPTURE") {
                    self.navigate("nodes");
                    let weak = Rc::downgrade(self);
                    glib::timeout_add_local_once(Duration::from_millis(600), move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.capture(path);
                        }
                    });
                }
            }
            Event::Disconnected(error) => {
                self.state.borrow_mut().connected = false;
                self.state.borrow_mut().settings.clear();
                self.editor.borrow_mut().take();
                self.setting_save.set_sensitive(false);
                self.status.set_text("Disconnected");
                self.status.remove_css_class("connected");
                self.status.set_tooltip_text(None);
                self.connect_button.set_sensitive(true);
                self.setting_status
                    .set_text("Disconnected — reconnect to edit settings");
                self.log(&error);
                if error != "Disconnected" {
                    self.notice(&error);
                }
                if let Some(plan) = self.pending_flash.borrow_mut().take() {
                    self.connect_button.set_sensitive(false);
                    crate::flashing::start(plan, self.events.clone());
                }
            }
            Event::Node(node) => {
                self.state.borrow_mut().nodes.insert(node.num, node);
                self.refresh_nodes();
            }
            Event::Setting { key, message } => {
                self.state
                    .borrow_mut()
                    .settings
                    .insert(key.clone(), message);
                if self.selected_category.borrow().as_deref() == Some(&key) {
                    let dirty = self
                        .editor
                        .borrow()
                        .as_ref()
                        .is_some_and(|editor| editor.dirty.get());
                    if !dirty {
                        self.show_category(&key);
                    }
                }
                if key.starts_with("channel.") {
                    self.refresh_channels();
                }
            }
            Event::Message(message) => {
                let mut state = self.state.borrow_mut();
                if state
                    .messages
                    .iter()
                    .any(|existing| existing.id == message.id && existing.from == message.from)
                {
                    return;
                }
                if !message.outgoing
                    && state.prefs.notifications
                    && let Some(app) = self.window.application()
                {
                    let notification = gtk::gio::Notification::new("New mesh message");
                    notification.set_body(Some(&message.text));
                    app.send_notification(None, &notification);
                }
                state.messages.push(message);
                let prefs = state.prefs.clone();
                storage::prune_history(&mut state.messages, &prefs);
                drop(state);
                self.refresh_messages();
                self.save_history();
            }
            Event::Delivery { id, status } => {
                for msg in &mut self.state.borrow_mut().messages {
                    if msg.id == id && msg.outgoing {
                        msg.status = status.clone();
                    }
                }
                self.refresh_messages();
                self.save_history();
            }
            Event::BluetoothDevices(devices) => {
                self.bluetooth_devices.remove_all();
                for (id, name) in &devices {
                    self.bluetooth_devices
                        .append(Some(id), &format!("{name} · {id}"));
                }
                if !devices.is_empty() {
                    self.bluetooth_devices.set_active(Some(0));
                }
                self.scan_button.set_sensitive(true);
                self.log(&format!(
                    "Found {} Meshtastic Bluetooth devices",
                    devices.len()
                ));
                if devices.is_empty() {
                    self.notice("No Meshtastic Bluetooth nodes found. Check that Bluetooth is enabled on the node.");
                }
            }
            Event::Saved { key } => {
                self.log(&format!("Radio acknowledged settings: {key}"));
                if self.selected_category.borrow().as_deref() == Some(&key) {
                    if let Some(form) = self.editor.borrow().as_ref() {
                        form.dirty.set(false);
                        form.widget.set_sensitive(true);
                    }
                    self.setting_status
                        .set_text("Acknowledged by radio · reloading values…");
                    self.setting_save.set_sensitive(true);
                }
            }
            Event::SaveFailed { key, error } => {
                self.notice(&error);
                if self.selected_category.borrow().as_deref() == Some(&key) {
                    self.setting_status.set_text(&error);
                    self.setting_save.set_sensitive(true);
                    if let Some(form) = self.editor.borrow().as_ref() {
                        form.widget.set_sensitive(true);
                    }
                }
            }
            Event::ReadFailed { key } => {
                let text = "The node did not return this category. It may be unsupported by this firmware or disabled in this build. You can try Reload from node.";
                if self.selected_category.borrow().as_deref() == Some(&key)
                    && !self.state.borrow().settings.contains_key(&key)
                {
                    self.setting_status.set_text(text);
                }
                self.log(&format!("Settings read timed out: {key}"));
            }
            Event::Error(error) => {
                self.notice(&error);
                self.scan_button.set_sensitive(true);
                self.setting_save
                    .set_sensitive(self.state.borrow().connected && self.editor.borrow().is_some());
                if let Some(form) = self.editor.borrow().as_ref() {
                    form.widget.set_sensitive(true);
                }
            }
            Event::Log(text) => self.log(&text),
            Event::Firmware(text) => append_log(&self.flash_log, &format!("{text}\n")),
            Event::FlashFinished(success) => {
                self.flash_busy.set(false);
                self.flash_button.set_sensitive(true);
                self.connect_button.set_sensitive(true);
                self.flash_status.set_text(if success {
                    "Transfer complete. Reconnect to check the firmware version."
                } else {
                    "Flash failed — see the output below"
                });
            }
        }
    }
    pub fn save_preferences(&self) {
        if let Err(error) = storage::write("preferences.json", &self.state.borrow().prefs) {
            self.notice(&error.to_string());
        }
    }
    pub fn save_history(&self) {
        let state = self.state.borrow();
        let history = if state.prefs.remember_messages {
            state.messages.as_slice()
        } else {
            &[]
        };
        if let Err(error) = storage::write("messages.json", &history) {
            drop(state);
            self.notice(&error.to_string());
        }
    }
    pub fn capture(&self, path: impl AsRef<std::path::Path>) {
        let root = self.window.child().expect("Application root");
        let paintable = gtk::WidgetPaintable::new(Some(&root));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, root.width() as f64, root.height() as f64);
        if let Some(node) = snapshot.to_node()
            && let Some(renderer) = self.window.renderer()
        {
            let path = path.as_ref();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = renderer.render_texture(&node, None).save_to_png(path) {
                eprintln!("UI rendering capture failed: {error}");
            }
        }
    }
}

fn update_nav(widget: &gtk::Widget, selected: &str) {
    if let Some(button) = widget.downcast_ref::<gtk::ToggleButton>()
        && button.has_css_class("nav-button")
    {
        button.set_active(button.widget_name() == selected);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        update_nav(&widget, selected);
        child = widget.next_sibling();
    }
}
pub fn apply_theme(theme: &str) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(theme != "light");
    }
}
pub fn append_log(buffer: &gtk::TextBuffer, text: &str) {
    buffer.insert(&mut buffer.end_iter(), text);
    if buffer.line_count() > 1000 {
        buffer.delete(
            &mut buffer.start_iter(),
            &mut buffer.iter_at_line(buffer.line_count() - 800).unwrap(),
        );
    }
}
pub fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}
pub fn button(text: &str, class: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    if !class.is_empty() {
        button.add_css_class(class);
    }
    button
}
pub fn margins(widget: &impl IsA<gtk::Widget>, vertical: i32, horizontal: i32) {
    widget.set_margin_top(vertical);
    widget.set_margin_bottom(vertical);
    widget.set_margin_start(horizontal);
    widget.set_margin_end(horizontal);
}
pub fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
pub fn scroll(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(child)
        .build()
}
pub fn heading(title: &str) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 7);
    group.append(&label(title, "section-title"));
    group
}
pub fn card() -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 16);
    card.add_css_class("card");
    margins(&card, 0, 0);
    card
}
