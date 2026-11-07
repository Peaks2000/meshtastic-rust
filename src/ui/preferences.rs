use super::*;

pub fn page(ui: &Rc<Ui>) -> gtk::ScrolledWindow {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    margins(&page, 20, 30);
    let appearance = card();
    appearance.append(&heading("Appearance"));
    let theme = gtk::ComboBoxText::new();
    theme.append(Some("dark"), "Dark");
    theme.append(Some("light"), "Light");
    theme.set_active_id(Some(&ui.state.borrow().prefs.theme));
    appearance.append(&label("Color theme", "field-label"));
    appearance.append(&theme);
    let weak = Rc::downgrade(ui);
    theme.connect_changed(move |combo| {
        if let Some(ui) = weak.upgrade()
            && let Some(theme) = combo.active_id()
        {
            apply_theme(&theme);
            ui.state.borrow_mut().prefs.theme = theme.to_string();
            ui.save_preferences();
        }
    });
    page.append(&appearance);
    let nodes = card();
    nodes.append(&heading("Node layout"));
    for (key, label) in [
        ("compact_nodes", "Compact node list"),
        ("show_power", "Power and battery"),
        ("show_last_heard", "Last heard time"),
        ("relative_time", "Relative last heard time"),
        ("show_position", "Position coordinates"),
        ("show_hops", "Hops away"),
        ("show_signal", "Signal strength"),
        ("show_role", "Device role"),
    ] {
        nodes.append(&preference_switch(ui, key, label));
    }
    page.append(&nodes);
    let connection = card();
    connection.append(&heading("Connections & notifications"));
    connection.append(&preference_switch(
        ui,
        "auto_connect",
        "Connect to the last node on startup",
    ));
    connection.append(&preference_switch(
        ui,
        "notifications",
        "Desktop notifications for incoming messages",
    ));
    page.append(&connection);
    let data = card();
    data.append(&heading("Messages & local data"));
    data.append(&preference_switch(
        ui,
        "remember_messages",
        "Remember message history",
    ));
    let retention = gtk::SpinButton::with_range(0.0, 365.0, 1.0);
    retention.set_value(ui.state.borrow().prefs.retention_days as f64);
    data.append(&label(
        "History retention in days (0 keeps all, up to 5,000 messages)",
        "field-label",
    ));
    data.append(&retention);
    let weak = Rc::downgrade(ui);
    retention.connect_value_changed(move |spin| {
        if let Some(ui) = weak.upgrade() {
            let mut state = ui.state.borrow_mut();
            state.prefs.retention_days = spin.value_as_int() as u32;
            let prefs = state.prefs.clone();
            storage::prune_history(&mut state.messages, &prefs);
            drop(state);
            ui.save_preferences();
            ui.save_history();
            ui.refresh_messages();
        }
    });
    let clear = button("Clear local message history…", "");
    data.append(&clear);
    let weak = Rc::downgrade(ui);
    clear.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let dialog = gtk::MessageDialog::builder()
                .transient_for(&ui.window)
                .modal(true)
                .message_type(gtk::MessageType::Question)
                .buttons(gtk::ButtonsType::OkCancel)
                .text("Clear local message history?")
                .secondary_text(
                    "This removes saved messages from this computer. It does not change the radio.",
                )
                .build();
            let weak = Rc::downgrade(&ui);
            dialog.connect_response(move |dialog, response| {
                if response == gtk::ResponseType::Ok
                    && let Some(ui) = weak.upgrade()
                {
                    ui.state.borrow_mut().messages.clear();
                    ui.save_history();
                    ui.refresh_messages();
                }
                dialog.close();
            });
            dialog.present();
        }
    });
    page.append(&data);
    scroll(&page)
}

fn preference_switch(ui: &Rc<Ui>, key: &'static str, name: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("field-row");
    let name = label(name, "field-label");
    name.set_hexpand(true);
    name.set_wrap(true);
    row.append(&name);
    let prefs = &ui.state.borrow().prefs;
    let value = match key {
        "compact_nodes" => prefs.compact_nodes,
        "show_power" => prefs.show_power,
        "show_last_heard" => prefs.show_last_heard,
        "relative_time" => prefs.relative_time,
        "show_position" => prefs.show_position,
        "show_hops" => prefs.show_hops,
        "show_signal" => prefs.show_signal,
        "show_role" => prefs.show_role,
        "auto_connect" => prefs.auto_connect,
        "notifications" => prefs.notifications,
        "remember_messages" => prefs.remember_messages,
        _ => false,
    };
    let switch = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(value)
        .build();
    row.append(&switch);
    let weak = Rc::downgrade(ui);
    switch.connect_active_notify(move |switch| {
        if let Some(ui) = weak.upgrade() {
            let value = switch.is_active();
            let mut state = ui.state.borrow_mut();
            let prefs = &mut state.prefs;
            match key {
                "compact_nodes" => prefs.compact_nodes = value,
                "show_power" => prefs.show_power = value,
                "show_last_heard" => prefs.show_last_heard = value,
                "relative_time" => prefs.relative_time = value,
                "show_position" => prefs.show_position = value,
                "show_hops" => prefs.show_hops = value,
                "show_signal" => prefs.show_signal = value,
                "show_role" => prefs.show_role = value,
                "auto_connect" => prefs.auto_connect = value,
                "notifications" => prefs.notifications = value,
                "remember_messages" => prefs.remember_messages = value,
                _ => {}
            }
            drop(state);
            ui.save_preferences();
            ui.save_history();
            ui.refresh_nodes();
        }
    });
    row
}
