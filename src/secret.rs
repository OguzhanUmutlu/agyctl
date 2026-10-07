use base64::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn get_gemini_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home).join(".gemini")
}


pub fn get_app_storage_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home).join(".config").join("Antigravity").join("app_storage.json")
}

pub fn get_standalone_token_path() -> PathBuf {
    get_gemini_dir().join("jetski-standalone-oauth-token")
}

pub fn parse_jwt_payload(jwt: &str) -> Option<Value> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = parts[1];
    let rem = payload.len() % 4;
    let padded = if rem > 0 {
        format!("{}{}", payload, "=".repeat(4 - rem))
    } else {
        payload.to_string()
    };
    let decoded = BASE64_URL_SAFE.decode(padded.as_bytes()).ok()?;
    serde_json::from_slice(&decoded).ok()
}

pub fn get_active_token() -> Option<Value> {
    if let Ok(token) = get_keyring_token_dbus() {
        return Some(token);
    }
    let path = get_standalone_token_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(val) = serde_json::from_str(&content) {
                return Some(val);
            }
        }
    }
    None
}

fn get_keyring_token_dbus() -> Result<Value, Box<dyn std::error::Error>> {
    let conn = zbus::blocking::Connection::session()?;
    let open_reply = conn.call_method(
        Some("org.freedesktop.secrets"),
        "/org/freedesktop/secrets",
        Some("org.freedesktop.Secret.Service"),
        "OpenSession",
        &("plain", zbus::zvariant::Value::from("")),
    )?;
    let (output, session_path): (zbus::zvariant::OwnedValue, zbus::zvariant::OwnedObjectPath) =
        open_reply.body().deserialize()?;
    let _ = output;

    let mut attributes = HashMap::new();
    attributes.insert("service", "gemini");
    attributes.insert("username", "antigravity");

    let search_reply = conn.call_method(
        Some("org.freedesktop.secrets"),
        "/org/freedesktop/secrets",
        Some("org.freedesktop.Secret.Service"),
        "SearchItems",
        &(attributes,),
    )?;
    let (unlocked, _locked): (Vec<zbus::zvariant::OwnedObjectPath>, Vec<zbus::zvariant::OwnedObjectPath>) =
        search_reply.body().deserialize()?;

    if unlocked.is_empty() {
        return Err("No item in keyring".into());
    }

    let item_path = &unlocked[0];
    let secret_reply = conn.call_method(
        Some("org.freedesktop.secrets"),
        item_path.as_str(),
        Some("org.freedesktop.Secret.Item"),
        "GetSecret",
        &(session_path,),
    )?;

    let (_session, _params, secret_bytes, _content_type): (
        zbus::zvariant::OwnedObjectPath,
        Vec<u8>,
        Vec<u8>,
        String,
    ) = secret_reply.body().deserialize()?;

    let parsed: Value = serde_json::from_slice(&secret_bytes)?;
    Ok(parsed)
}

pub fn set_active_token(token_data: &Value) -> bool {
    let secret_bytes = serde_json::to_vec(token_data).unwrap_or_default();
    let dbus_ok = set_keyring_token_dbus(&secret_bytes).is_ok();

    let standalone_path = get_standalone_token_path();
    let _ = fs::write(&standalone_path, &secret_bytes);

    if let Some(id_token) = token_data.get("id_token").and_then(|v| v.as_str()) {
        if let Some(jwt) = parse_jwt_payload(id_token) {
            if let Some(email) = jwt.get("email").and_then(|e| e.as_str()) {
                let storage_path = get_app_storage_path();
                if storage_path.exists() {
                    if let Ok(content) = fs::read_to_string(&storage_path) {
                        if let Ok(mut storage) = serde_json::from_str::<Value>(&content) {
                            if let Some(obj) = storage.as_object_mut() {
                                obj.insert(
                                    "jetski.onboarding.lastLoginUsername".to_string(),
                                    Value::String(email.to_string()),
                                );
                                let _ = fs::write(
                                    &storage_path,
                                    serde_json::to_string_pretty(&storage).unwrap_or_default(),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    dbus_ok || standalone_path.exists()
}

fn set_keyring_token_dbus(secret_bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let conn = zbus::blocking::Connection::session()?;
    let open_reply = conn.call_method(
        Some("org.freedesktop.secrets"),
        "/org/freedesktop/secrets",
        Some("org.freedesktop.Secret.Service"),
        "OpenSession",
        &("plain", zbus::zvariant::Value::from("")),
    )?;
    let (_output, session_path): (zbus::zvariant::OwnedValue, zbus::zvariant::OwnedObjectPath) =
        open_reply.body().deserialize()?;

    let mut attributes = HashMap::new();
    attributes.insert("service", "gemini");
    attributes.insert("username", "antigravity");

    let search_reply = conn.call_method(
        Some("org.freedesktop.secrets"),
        "/org/freedesktop/secrets",
        Some("org.freedesktop.Secret.Service"),
        "SearchItems",
        &(&attributes,),
    )?;
    let (unlocked, _locked): (Vec<zbus::zvariant::OwnedObjectPath>, Vec<zbus::zvariant::OwnedObjectPath>) =
        search_reply.body().deserialize()?;

    let secret_tuple = (
        session_path,
        Vec::<u8>::new(),
        secret_bytes.to_vec(),
        "text/plain",
    );

    if !unlocked.is_empty() {
        let item_path = &unlocked[0];
        conn.call_method(
            Some("org.freedesktop.secrets"),
            item_path.as_str(),
            Some("org.freedesktop.Secret.Item"),
            "SetSecret",
            &(secret_tuple,),
        )?;
    } else {
        let mut props = HashMap::new();
        props.insert(
            "org.freedesktop.Secret.Item.Label",
            zbus::zvariant::Value::from("Password for 'antigravity' on 'gemini'"),
        );
        let mut attrs_val = HashMap::new();
        attrs_val.insert("service", "gemini");
        attrs_val.insert("username", "antigravity");
        attrs_val.insert("xdg:schema", "org.freedesktop.Secret.Generic");
        props.insert(
            "org.freedesktop.Secret.Item.Attributes",
            zbus::zvariant::Value::from(attrs_val),
        );

        conn.call_method(
            Some("org.freedesktop.secrets"),
            "/org/freedesktop/secrets/aliases/default",
            Some("org.freedesktop.Secret.Collection"),
            "CreateItem",
            &(props, secret_tuple, true),
        )?;
    }

    Ok(())
}
