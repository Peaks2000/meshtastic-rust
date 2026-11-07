use crate::protocol::{self as p, put};
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use prost_reflect::{DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, Value};

#[derive(Debug, Clone)]
pub struct Category {
    pub key: String,
    pub title: String,
    pub group: &'static str,
    pub descriptor: MessageDescriptor,
    pub wrapper: Option<(String, String)>,
    pub request: (String, Value),
    pub setter: String,
}

pub fn title(name: &str) -> String {
    match name {
        "lora" => "LoRa".into(),
        "mqtt" => "MQTT".into(),
        "paxcounter" => "PAX Counter".into(),
        "canned_message" => "Canned Messages".into(),
        "store_forward" => "Store & Forward".into(),
        "device_ui" => "Device UI".into(),
        "tak" => "TAK".into(),
        _ => name
            .split('_')
            .map(|s| {
                let mut c = s.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub fn categories() -> Vec<Category> {
    let mut result = vec![Category {
        key: "user".into(),
        title: "User".into(),
        group: "Identity & channels",
        descriptor: p::descriptor("User"),
        wrapper: None,
        request: ("get_owner_request".into(), Value::Bool(true)),
        setter: "set_owner".into(),
    }];
    for index in 0..8 {
        result.push(Category {
            key: format!("channel.{index}"),
            title: format!("Channel {index}"),
            group: "Identity & channels",
            descriptor: p::descriptor("Channel"),
            wrapper: None,
            request: ("get_channel_request".into(), Value::U32(index + 1)),
            setter: "set_channel".into(),
        });
    }
    for (wrapper, enum_name, request, setter, group, prefix) in [
        (
            "Config",
            "AdminMessage.ConfigType",
            "get_config_request",
            "set_config",
            "Radio configuration",
            "config",
        ),
        (
            "ModuleConfig",
            "AdminMessage.ModuleConfigType",
            "get_module_config_request",
            "set_module_config",
            "Module configuration",
            "module",
        ),
    ] {
        let enumeration = p::pool()
            .get_enum_by_name(&format!("meshtastic.{enum_name}"))
            .unwrap();
        for field in p::descriptor(wrapper).fields() {
            let Kind::Message(desc) = field.kind() else {
                continue;
            };
            let normal = match field.name() {
                "external_notification" => "EXTNOTIF".into(),
                "canned_message" => "CANNEDMSG".into(),
                name => name.replace('_', "").to_uppercase(),
            };
            let request_enum = enumeration
                .values()
                .find(|v| v.name().trim_end_matches("_CONFIG").replace('_', "") == normal);
            if let Some(value) = request_enum {
                result.push(Category {
                    key: format!("{prefix}.{}", field.name()),
                    title: title(field.name()),
                    group,
                    descriptor: desc,
                    wrapper: Some((wrapper.into(), field.name().into())),
                    request: (request.into(), Value::EnumNumber(value.number())),
                    setter: setter.into(),
                });
            }
        }
    }
    for (key, label, get, set) in [
        (
            "canned_text",
            "Canned Message Text",
            "get_canned_message_module_messages_request",
            "set_canned_message_module_messages",
        ),
        (
            "ringtone",
            "Ringtone",
            "get_ringtone_request",
            "set_ringtone_message",
        ),
    ] {
        result.push(Category {
            key: key.into(),
            title: label.into(),
            group: "Module content",
            descriptor: p::descriptor("AdminMessage"),
            wrapper: None,
            request: (get.into(), Value::Bool(true)),
            setter: set.into(),
        });
    }
    result
}

pub fn category(key: &str) -> Option<Category> {
    categories().into_iter().find(|c| c.key == key)
}

pub fn admin_request(key: &str) -> Result<DynamicMessage> {
    let category = category(key).context("Unknown settings category")?;
    let mut msg = p::message("AdminMessage");
    put(&mut msg, &category.request.0, category.request.1);
    Ok(msg)
}

pub fn admin_set(key: &str, value: DynamicMessage, session: &[u8]) -> Result<DynamicMessage> {
    let category = category(key).context("Unknown settings category")?;
    let value = if key == "ringtone" || key == "canned_text" {
        Value::String(p::string(&value, &category.setter))
    } else if let Some((wrapper, field)) = category.wrapper {
        let mut wrapped = p::message(&wrapper);
        put(&mut wrapped, &field, Value::Message(value));
        Value::Message(wrapped)
    } else {
        Value::Message(value)
    };
    let mut msg = p::message("AdminMessage");
    put(&mut msg, &category.setter, value);
    put(
        &mut msg,
        "session_passkey",
        Value::Bytes(session.to_vec().into()),
    );
    Ok(msg)
}

pub fn unwrap_config(wrapper: &DynamicMessage, prefix: &str) -> Option<(String, DynamicMessage)> {
    wrapper.fields().find_map(|(field, value)| {
        if let Value::Message(value) = value {
            Some((format!("{prefix}.{}", field.name()), value.clone()))
        } else {
            None
        }
    })
}

pub fn editable_fields(category: &Category) -> Vec<FieldDescriptor> {
    category
        .descriptor
        .fields()
        .filter(|f| {
            if category.key == "user" {
                return ["long_name", "short_name", "is_licensed"].contains(&f.name());
            }
            if category.key.starts_with("channel.") {
                return ["settings", "role"].contains(&f.name());
            }
            if category.key == "ringtone" || category.key == "canned_text" {
                return f.name() == category.setter;
            }
            true
        })
        .collect()
}

/// Read the same first-party field metadata used by the Apple client.
pub fn metadata(field: &FieldDescriptor, name: &str) -> Option<Value> {
    let extension = p::pool().get_extension_by_name("meshtastic.field_metadata")?;
    let options = field.options();
    let value = options.get_extension(&extension);
    let Value::Message(meta) = value.as_ref() else {
        return None;
    };
    if meta.has_field_by_name(name) {
        meta.get_field_by_name(name).map(|v| v.into_owned())
    } else {
        None
    }
}
pub fn field_label(field: &FieldDescriptor) -> String {
    if let Some(Value::String(label)) = metadata(field, "label")
        && !label.is_empty()
    {
        return label;
    }
    title(field.name())
}
pub fn field_help(field: &FieldDescriptor) -> String {
    if let Some(Value::String(text)) = metadata(field, "description")
        && !text.is_empty()
    {
        return text;
    }
    let path = field.path();
    field
        .parent_file()
        .file_descriptor_proto()
        .source_code_info
        .as_ref()
        .and_then(|info| info.location.iter().find(|l| l.path == path))
        .and_then(|location| location.leading_comments.as_ref())
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|s| !s.contains("TODO: REPLACE"))
        .unwrap_or_default()
}

pub fn field_text(message: &DynamicMessage, field: &FieldDescriptor) -> String {
    let value = message.get_field(field);
    if field.is_list() || field.is_map() {
        let json = serde_json::to_value(message).unwrap_or_default();
        return serde_json::to_string(&json.get(field.json_name()).cloned().unwrap_or_else(|| {
            if field.is_map() {
                serde_json::json!({})
            } else {
                serde_json::json!([])
            }
        }))
        .unwrap_or_default();
    }
    match value.as_ref() {
        Value::Bool(v) => v.to_string(),
        Value::I32(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::U32(v) => v.to_string(),
        Value::U64(v) => v.to_string(),
        Value::F32(v) => v.to_string(),
        Value::F64(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Bytes(v) => STANDARD.encode(v),
        Value::EnumNumber(v) => v.to_string(),
        _ => String::new(),
    }
}

pub fn parse_field(field: &FieldDescriptor, text: &str) -> Result<Value> {
    if field.is_list() || field.is_map() {
        let json: serde_json::Value =
            serde_json::from_str(text).context("Use a JSON array (or object for maps)")?;
        let json = serde_json::json!({ field.json_name(): json }).to_string();
        let msg = DynamicMessage::deserialize(
            field.parent_message().clone(),
            &mut serde_json::Deserializer::from_str(&json),
        )?;
        return Ok(msg.get_field(field).into_owned());
    }
    let trimmed = text.trim();
    let value = match field.kind() {
        Kind::Bool => Value::Bool(trimmed.parse().context("Expected true or false")?),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => Value::I32(
            trimmed
                .parse()
                .context("Expected a signed 32-bit integer")?,
        ),
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => Value::I64(
            trimmed
                .parse()
                .context("Expected a signed 64-bit integer")?,
        ),
        Kind::Uint32 | Kind::Fixed32 => Value::U32(
            trimmed
                .parse()
                .context("Expected an unsigned 32-bit integer")?,
        ),
        Kind::Uint64 | Kind::Fixed64 => Value::U64(
            trimmed
                .parse()
                .context("Expected an unsigned 64-bit integer")?,
        ),
        Kind::Float => {
            let n: f32 = trimmed.parse()?;
            if !n.is_finite() {
                bail!("Must be finite");
            }
            Value::F32(n)
        }
        Kind::Double => {
            let n: f64 = trimmed.parse()?;
            if !n.is_finite() {
                bail!("Must be finite");
            }
            Value::F64(n)
        }
        Kind::String => Value::String(text.into()),
        Kind::Bytes => Value::Bytes(
            STANDARD
                .decode(trimmed)
                .context("Expected a Base64-encoded key")?
                .into(),
        ),
        Kind::Enum(_) => Value::EnumNumber(trimmed.parse().context("Choose an enum value")?),
        Kind::Message(_) => bail!("Edit the nested fields below"),
    };
    let number = match value {
        Value::U32(n) => Some(n as f64),
        Value::I32(n) => Some(n as f64),
        Value::F32(n) => Some(n as f64),
        Value::F64(n) => Some(n),
        _ => None,
    };
    if let Some(number) = number {
        for (bound, minimum) in [("min_value", true), ("max_value", false)] {
            if let Some(Value::F64(limit)) = metadata(field, bound)
                && ((minimum && number < limit) || (!minimum && number > limit))
            {
                bail!("Value is outside the allowed {bound}: {limit}");
            }
        }
    }
    if field.full_name().ends_with("User.short_name") && text.chars().count() > 4 {
        bail!("Short name must be at most 4 characters");
    }
    if field.full_name().ends_with("User.long_name") && text.len() > 39 {
        bail!("Long name must be at most 39 UTF-8 bytes");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_reflect::ReflectMessage;

    #[test]
    fn apple_settings_field_catalog_is_covered_by_the_native_schema() {
        let fields: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../assets/apple-settings-fields.json")).unwrap();
        assert!(fields.len() > 200);
        for reference in fields {
            let name = reference["message"].as_str().unwrap();
            let mut message = p::pool()
                .get_message_by_name(name)
                .unwrap_or_else(|| panic!("Missing Apple settings message {name}"));
            let tag = reference["tag"].as_u64().unwrap() as u32;
            let path = reference["field"]
                .as_str()
                .unwrap()
                .split('.')
                .collect::<Vec<_>>();
            for (index, part) in path.iter().enumerate() {
                let field = message
                    .get_field_by_name(part)
                    .unwrap_or_else(|| panic!("Missing {name} field {part}"));
                if index == path.len() - 1 {
                    assert_eq!(field.number(), tag);
                } else if let Kind::Message(nested) = field.kind() {
                    message = nested;
                } else {
                    panic!("Expected a nested message at {name}.{part}");
                }
            }
        }
    }
    #[test]
    fn every_radio_and_module_category_has_a_request_and_setter() {
        for wrapper in ["Config", "ModuleConfig"] {
            for field in p::descriptor(wrapper).fields() {
                let key = format!(
                    "{}.{}",
                    if wrapper == "Config" {
                        "config"
                    } else {
                        "module"
                    },
                    field.name()
                );
                assert!(category(&key).is_some(), "Missing {key}");
            }
        }
        for category in categories() {
            assert!(admin_request(&category.key).is_ok());
        }
    }
    #[test]
    fn invalid_unsigned_values_and_nonfinite_floats_are_rejected() {
        let desc = p::descriptor("Config.LoRaConfig");
        assert!(parse_field(&desc.get_field_by_name("hop_limit").unwrap(), "-1").is_err());
        assert!(
            parse_field(
                &desc.get_field_by_name("override_frequency").unwrap(),
                "NaN"
            )
            .is_err()
        );
    }
    #[test]
    fn editing_preserves_unknown_wire_fields() {
        use prost::Message;
        // Future firmware field 1000, varint 42.
        let mut msg = DynamicMessage::decode(
            p::descriptor("Config.BluetoothConfig"),
            &[0xc0, 0x3e, 42][..],
        )
        .unwrap();
        let field = msg.descriptor().get_field_by_name("enabled").unwrap();
        msg.set_field(&field, Value::Bool(true));
        assert!(
            msg.encode_to_vec()
                .windows(3)
                .any(|w| w == [0xc0, 0x3e, 42])
        );
    }
}
