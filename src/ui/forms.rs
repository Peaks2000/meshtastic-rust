use super::*;
use crate::{
    protocol::{self as p},
    settings::{self, Category},
};
use anyhow::{Context, Result};
use prost_reflect::{DynamicMessage, FieldDescriptor, Kind, Value};

struct Binding {
    path: Vec<FieldDescriptor>,
    read: Box<dyn Fn() -> Result<Value>>,
}
pub struct Form {
    pub widget: gtk::Box,
    pub dirty: Rc<Cell<bool>>,
    baseline: DynamicMessage,
    bindings: Vec<Binding>,
}

impl Form {
    pub fn build(category: &Category, baseline: DynamicMessage) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let dirty = Rc::new(Cell::new(false));
        let mut bindings = vec![];
        for field in settings::editable_fields(category) {
            add_field(&widget, &baseline, field, vec![], &dirty, &mut bindings, 0);
        }
        Self {
            widget,
            dirty,
            baseline,
            bindings,
        }
    }
    pub fn collect(&self) -> Result<DynamicMessage> {
        let mut result = self.baseline.clone();
        for binding in &self.bindings {
            let label = binding
                .path
                .iter()
                .map(settings::field_label)
                .collect::<Vec<_>>()
                .join(" → ");
            let value = (binding.read)().with_context(|| label.to_string())?;
            patch(&mut result, &binding.path, value);
        }
        Ok(result)
    }
}

fn patch(message: &mut DynamicMessage, path: &[FieldDescriptor], value: Value) {
    let field = &path[0];
    if path.len() == 1 {
        if message.get_field(field).as_ref() != &value {
            message.set_field(field, value);
        }
    } else {
        let Value::Message(mut child) = message.get_field(field).into_owned() else {
            return;
        };
        let before = child.clone();
        patch(&mut child, &path[1..], value);
        if child != before {
            message.set_field(field, Value::Message(child));
        }
    }
}

fn add_field(
    container: &gtk::Box,
    message: &DynamicMessage,
    field: FieldDescriptor,
    mut path: Vec<FieldDescriptor>,
    dirty: &Rc<Cell<bool>>,
    bindings: &mut Vec<Binding>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    path.push(field.clone());
    if let Kind::Message(desc) = field.kind()
        && !field.is_list()
        && !field.is_map()
    {
        let nested =
            p::nested(message, field.name()).unwrap_or_else(|| DynamicMessage::new(desc.clone()));
        let expander = gtk::Expander::builder()
            .label(settings::field_label(&field))
            .expanded(true)
            .build();
        expander.set_margin_top(16);
        let children = gtk::Box::new(gtk::Orientation::Vertical, 0);
        children.set_margin_start(14);
        for nested_field in desc.fields() {
            add_field(
                &children,
                &nested,
                nested_field,
                path.clone(),
                dirty,
                bindings,
                depth + 1,
            );
        }
        expander.set_child(Some(&children));
        container.append(&expander);
        return;
    }
    let row = gtk::Box::new(gtk::Orientation::Vertical, 7);
    row.add_css_class("field-row");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let name = label(&settings::field_label(&field), "field-label");
    name.set_hexpand(true);
    name.set_wrap(true);
    name.set_width_chars(16);
    name.set_max_width_chars(35);
    top.append(&name);
    let value = message.get_field(&field);
    let reader: Box<dyn Fn() -> Result<Value>>;
    if field.kind() == Kind::Bool && !field.is_list() {
        let switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .active(matches!(value.as_ref(), Value::Bool(true)))
            .build();
        let changed = dirty.clone();
        switch.connect_active_notify(move |_| changed.set(true));
        top.append(&switch);
        reader = Box::new(move || Ok(Value::Bool(switch.is_active())));
    } else if let Kind::Enum(enumeration) = field.kind() {
        if !field.is_list() {
            let combo = gtk::ComboBoxText::new();
            combo.set_width_request(210);
            for value in enumeration.values() {
                combo.append(
                    Some(&value.number().to_string()),
                    &settings::title(&value.name().to_lowercase()),
                );
            }
            let id = settings::field_text(message, &field);
            if !combo.set_active_id(Some(&id)) {
                combo.append(Some(&id), &format!("Unknown value ({id})"));
                combo.set_active_id(Some(&id));
            }
            let changed = dirty.clone();
            combo.connect_changed(move |_| changed.set(true));
            top.append(&combo);
            let field = field.clone();
            reader = Box::new(move || {
                settings::parse_field(&field, combo.active_id().as_deref().unwrap_or("0"))
            });
        } else {
            reader = add_entry(&top, message, &field, dirty);
        }
    } else {
        reader = add_entry(&top, message, &field, dirty);
    }
    row.append(&top);
    let mut help = settings::field_help(&field);
    if let Some(Value::String(unit)) = settings::metadata(&field, "unit")
        && !unit.is_empty()
    {
        help.push_str(&format!("  Unit: {unit}."));
    }
    if let Some(Value::String(version)) = settings::metadata(&field, "since_firmware") {
        help.push_str(&format!("  Requires firmware {version}+."));
    }
    if field
        .field_descriptor_proto()
        .options
        .as_ref()
        .is_some_and(|o| o.deprecated())
    {
        help.push_str("  Deprecated field; retained for older nodes.");
    }
    if field.is_list() || field.is_map() {
        help.push_str("  Enter a JSON array (Base64 strings for keys); maps use a JSON object.");
    }
    if matches!(field.kind(), Kind::Bytes) {
        help.push_str("  Base64 encoded.");
    }
    if !help.is_empty() {
        row.set_tooltip_text(Some(&help));
    }
    container.append(&row);
    bindings.push(Binding { path, read: reader });
}

fn add_entry(
    top: &gtk::Box,
    message: &DynamicMessage,
    field: &FieldDescriptor,
    dirty: &Rc<Cell<bool>>,
) -> Box<dyn Fn() -> Result<Value>> {
    let original = settings::field_text(message, field);
    let original_value = message.get_field(field).into_owned();
    let entry = gtk::Entry::builder()
        .width_chars(20)
        .max_width_chars(32)
        .text(&original)
        .build();
    let sensitive = [
        "password",
        "private_key",
        "admin_key",
        "psk",
        "fixed_pin",
        "session_key",
    ]
    .iter()
    .any(|name| field.name().contains(name));
    if sensitive {
        entry.set_visibility(false);
        entry.set_secondary_icon_name(Some("view-reveal-symbolic"));
        entry.connect_icon_release(|entry, _| {
            entry.set_visibility(!gtk::prelude::EntryExt::is_visible(entry));
        });
    }
    let changed = dirty.clone();
    entry.connect_changed(move |_| changed.set(true));
    top.append(&entry);
    let field = field.clone();
    Box::new(move || {
        if entry.text().as_str() == original {
            Ok(original_value.clone())
        } else {
            settings::parse_field(&field, entry.text().as_str())
        }
    })
}

pub fn settings_page(ui: &Rc<Ui>) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::None);
    stack.set_transition_duration(0);
    let home = gtk::Box::new(gtk::Orientation::Vertical, 20);
    margins(&home, 28, 30);
    for (target, title, icon) in [
        ("node", "Node settings", "network-workgroup-symbolic"),
        ("app", "App settings", "preferences-system-symbolic"),
    ] {
        let card = gtk::Button::new();
        card.add_css_class("card");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(32);
        row.append(&image);
        let text = heading(title);
        text.set_hexpand(true);
        row.append(&text);
        row.append(&label("›", "section-title"));
        card.set_child(Some(&row));
        home.append(&card);
        let stack = stack.clone();
        card.connect_clicked(move |_| {
            stack.set_visible_child_name(target);
        });
    }
    stack.add_named(&scroll(&home), Some("home"));
    let node = gtk::Box::new(gtk::Orientation::Vertical, 0);
    node.append(&back_button(&stack));
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_position(220);
    split.set_resize_start_child(false);
    split.set_shrink_start_child(false);
    let categories = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["settings-categories"])
        .build();
    let mut last_group = "";
    let mut category_keys = vec![];
    for category in settings::categories() {
        if category.group != last_group {
            let row = gtk::ListBoxRow::builder()
                .selectable(false)
                .activatable(false)
                .build();
            let text = label(category.group, "eyebrow");
            text.set_wrap(true);
            text.set_margin_top(12);
            row.set_child(Some(&text));
            categories.append(&row);
            category_keys.push(None);
            last_group = category.group;
        }
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label(&category.title, "")));
        categories.append(&row);
        category_keys.push(Some(category.key));
    }
    split.set_start_child(Some(&scroll(&categories)));
    let right = gtk::Box::new(gtk::Orientation::Vertical, 16);
    margins(&right, 20, 24);
    right.append(&scroll(&ui.settings_content));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let reload = button("Reload from node", "");
    actions.append(&reload);
    actions.append(&ui.setting_save);
    ui.setting_save.set_sensitive(false);
    right.append(&actions);
    ui.setting_status.set_wrap(true);
    ui.setting_status.set_selectable(true);
    right.append(&ui.setting_status);
    ui.settings_content
        .append(&heading("Select a settings category"));
    split.set_end_child(Some(&right));
    split.set_vexpand(true);
    node.append(&split);
    stack.add_named(&node, Some("node"));
    let weak = Rc::downgrade(ui);
    categories.connect_row_selected(move |_, row| {
        if let Some(ui) = weak.upgrade()
            && let Some(key) = row
                .and_then(|row| category_keys.get(row.index() as usize))
                .and_then(|key| key.as_ref())
        {
            ui.select_category(key);
        }
    });
    let weak = Rc::downgrade(ui);
    reload.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let key = ui.selected_category.borrow().clone();
            if let Some(key) = key {
                if !ui.state.borrow().connected {
                    ui.notice("Connect to a node first.");
                    return;
                }
                let dirty = ui
                    .editor
                    .borrow()
                    .as_ref()
                    .is_some_and(|form| form.dirty.get());
                if dirty {
                    confirm_discard(&ui, move |ui| {
                        ui.editor.borrow_mut().take();
                        ui.state.borrow_mut().settings.remove(&key);
                        ui.show_category(&key);
                    });
                } else {
                    ui.editor.borrow_mut().take();
                    ui.state.borrow_mut().settings.remove(&key);
                    ui.show_category(&key);
                }
            }
        }
    });
    let weak = Rc::downgrade(ui);
    ui.setting_save.connect_clicked(move |_| if let Some(ui) = weak.upgrade() {
        if !ui.state.borrow().connected { ui.notice("Connect to a node first."); return; }
        let Some(key) = ui.selected_category.borrow().clone() else { return; };
        let result = ui.editor.borrow().as_ref().map(Form::collect);
        match result {
            Some(Ok(message)) => {
                let category = settings::category(&key).unwrap();
                let dialog = gtk::MessageDialog::builder().transient_for(&ui.window).modal(true).message_type(gtk::MessageType::Question)
                    .buttons(gtk::ButtonsType::OkCancel).text(format!("Save {} to the node?", category.title))
                    .secondary_text("The radio may restart or disconnect when these settings change. Values are only marked saved after the radio acknowledges the request.").build();
                let weak = Rc::downgrade(&ui); dialog.connect_response(move |dialog, response| {
                    if response == gtk::ResponseType::Ok && let Some(ui) = weak.upgrade() {
                        ui.setting_save.set_sensitive(false); ui.setting_status.set_text("Waiting for the radio to acknowledge the change…");
                        if let Some(form) = ui.editor.borrow().as_ref() { form.widget.set_sensitive(false); }
                        ui.send(Command::SaveSetting { key: key.clone(), message: message.clone() });
                    } dialog.close();
                }); dialog.present();
            }
            Some(Err(error)) => ui.notice(&format!("Check this field: {error:#}")),
            None => ui.notice("Load the settings from your node before saving."),
        }
    });
    let app = gtk::Box::new(gtk::Orientation::Vertical, 0);
    app.append(&back_button(&stack));
    app.append(&super::preferences::page(ui));
    stack.add_named(&app, Some("app"));
    stack
}

fn back_button(stack: &gtk::Stack) -> gtk::Button {
    let back = button("‹  All settings", "back");
    margins(&back, 12, 18);
    back.set_halign(gtk::Align::Start);
    let stack = stack.clone();
    back.connect_clicked(move |_| stack.set_visible_child_name("home"));
    back
}

impl Ui {
    pub fn select_category(self: &Rc<Self>, key: &str) {
        let dirty = self
            .editor
            .borrow()
            .as_ref()
            .is_some_and(|form| form.dirty.get());
        if dirty && self.selected_category.borrow().as_deref() != Some(key) {
            let key = key.to_string();
            confirm_discard(self, move |ui| ui.show_category(&key));
        } else {
            self.show_category(key);
        }
    }
    pub fn show_category(&self, key: &str) {
        let Some(category) = settings::category(key) else {
            return;
        };
        *self.selected_category.borrow_mut() = Some(key.to_string());
        clear_box(&self.settings_content);
        self.editor.borrow_mut().take();
        self.setting_save.set_sensitive(false);
        self.settings_content.append(&heading(&category.title));
        let baseline = self.state.borrow().settings.get(key).cloned();
        if let Some(baseline) = baseline {
            let form = Form::build(&category, baseline);
            self.settings_content.append(&form.widget);
            *self.editor.borrow_mut() = Some(form);
            self.setting_save
                .set_sensitive(self.state.borrow().connected);
            self.setting_status.set_text("Loaded from node");
        } else if self.state.borrow().connected {
            self.settings_content
                .append(&label("Reading this category from your radio…", "muted"));
            self.setting_status
                .set_text("Waiting for the node’s response. Unsupported modules may not respond.");
            self.send(Command::ReadSetting(key.into()));
        } else {
            self.settings_content.append(&label(
                "Connect to a node to view and edit its settings.",
                "muted",
            ));
            self.setting_status.set_text("No node connected");
        }
    }
}

fn confirm_discard(ui: &Rc<Ui>, action: impl Fn(&Rc<Ui>) + 'static) {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(&ui.window)
        .modal(true)
        .message_type(gtk::MessageType::Question)
        .buttons(gtk::ButtonsType::OkCancel)
        .text("Discard unsaved changes?")
        .secondary_text("These edits have not been sent to the node.")
        .build();
    let weak = Rc::downgrade(ui);
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Ok
            && let Some(ui) = weak.upgrade()
        {
            action(&ui);
        }
        dialog.close();
    });
    dialog.present();
}
