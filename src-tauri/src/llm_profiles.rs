use crate::util::{is_safe_path_segment, read_json, validate_path_segment, write_json, write_text};
use crate::CommandResult;
use keyring::Entry;
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn profiles_path(root: &Path) -> PathBuf {
    root.join("config").join("llm-profiles.json")
}

fn secret_path(root: &Path, profile_id: &str) -> PathBuf {
    let segment = if is_safe_path_segment(profile_id) {
        profile_id
    } else {
        "__invalid_profile_id__"
    };
    root.join("config")
        .join("secrets")
        .join(format!("{}.key", segment))
}

fn os_secret_service() -> &'static str {
    "com.ielts.author.studio.llm"
}

pub(crate) fn os_secret_backend() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows-credential-manager"
    }
    #[cfg(target_os = "macos")]
    {
        "macos-keychain"
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        "os-keyring"
    }
}

pub(crate) fn os_secret_ref(profile_id: &str) -> String {
    format!("os-secret:{}:{}", os_secret_service(), profile_id)
}

pub(crate) fn file_secret_ref(profile_id: &str) -> String {
    format!("profile-secret:{}", profile_id)
}

pub(crate) fn plaintext_secret_fallback_allowed() -> bool {
    env::var("EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn os_secret_entry(profile_id: &str) -> CommandResult<Entry> {
    validate_path_segment("profile_id", profile_id)?;
    Entry::new(os_secret_service(), profile_id)
        .map_err(|error| format!("os_secret_entry_failed:{}", error))
}

fn os_secret_save_secret(profile_id: &str, api_key: &str) -> CommandResult<()> {
    os_secret_entry(profile_id)?
        .set_password(api_key)
        .map_err(|error| format!("os_secret_save_failed:{}:{}", os_secret_backend(), error))
}

fn os_secret_load_secret(profile_id: &str) -> CommandResult<Option<String>> {
    match os_secret_entry(profile_id)?.get_password() {
        Ok(secret) => Ok((!secret.trim().is_empty()).then_some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!(
            "os_secret_load_failed:{}:{}",
            os_secret_backend(),
            error
        )),
    }
}

fn os_secret_delete_secret(profile_id: &str) -> CommandResult<()> {
    match os_secret_entry(profile_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "os_secret_delete_failed:{}:{}",
            os_secret_backend(),
            error
        )),
    }
}

pub(crate) fn file_save_secret(root: &Path, profile_id: &str, api_key: &str) -> CommandResult<()> {
    validate_path_segment("profile_id", profile_id)?;
    write_text(&secret_path(root, profile_id), api_key)
}

pub(crate) fn file_load_secret(root: &Path, profile_id: &str) -> Option<String> {
    if !is_safe_path_segment(profile_id) {
        return None;
    }
    fs::read_to_string(secret_path(root, profile_id))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn file_delete_secret(root: &Path, profile_id: &str) -> CommandResult<()> {
    validate_path_segment("profile_id", profile_id)?;
    let path = secret_path(root, profile_id);
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| format!("delete_secret_file:{}:{}", path.display(), error))?;
    }
    Ok(())
}

pub(crate) fn delete_profile_secret(root: &Path, profile_id: &str) -> CommandResult<()> {
    let _ = os_secret_delete_secret(profile_id);
    file_delete_secret(root, profile_id)
}

pub(crate) fn redact_profile_for_ui(root: &Path, mut profile: Value) -> Value {
    let Some(profile_id) = profile
        .get("profileId")
        .and_then(Value::as_str)
        .map(ToString::to_string)
    else {
        return profile;
    };
    let os_has_secret = matches!(os_secret_load_secret(&profile_id), Ok(Some(_)));
    let file_has_secret =
        plaintext_secret_fallback_allowed() && file_load_secret(root, &profile_id).is_some();
    let (backend, secret_ref, message) = if os_has_secret {
        (
            "os",
            os_secret_ref(&profile_id),
            format!(
                "API Key is stored in OS secure storage ({}).",
                os_secret_backend()
            ),
        )
    } else if file_has_secret {
        (
            "file",
            file_secret_ref(&profile_id),
            "API Key is stored in plaintext app data file fallback; this is enabled only by EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK.".to_string(),
        )
    } else {
        ("none", String::new(), "No API key is stored.".to_string())
    };
    if let Some(obj) = profile.as_object_mut() {
        obj.remove("apiKey");
        obj.insert("hasApiKey".to_string(), json!(backend != "none"));
        if backend == "none" {
            obj.remove("apiKeySecretRef");
        } else {
            obj.insert("apiKeySecretRef".to_string(), json!(secret_ref));
        }
        obj.insert("secretStorageBackend".to_string(), json!(backend));
        obj.insert("secretStorageMessage".to_string(), json!(message));
    }
    profile
}

pub(crate) fn save_profile_secret(
    root: &Path,
    profile_id: &str,
    api_key: Option<&str>,
) -> CommandResult<(bool, String, String)> {
    validate_path_segment("profile_id", profile_id)?;
    let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty()) else {
        let _ = os_secret_delete_secret(profile_id);
        file_delete_secret(root, profile_id)?;
        return Ok((
            false,
            "none".to_string(),
            "No API key is stored.".to_string(),
        ));
    };
    match os_secret_save_secret(profile_id, api_key) {
        Ok(()) => {
            let _ = file_delete_secret(root, profile_id);
            Ok((
                true,
                "os".to_string(),
                format!(
                    "API Key saved to OS secure storage ({}).",
                    os_secret_backend()
                ),
            ))
        }
        Err(error) => {
            if !plaintext_secret_fallback_allowed() {
                let _ = file_delete_secret(root, profile_id);
                return Err(format!(
                    "os_secret_unavailable_plaintext_fallback_disabled:{}; set EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1 only for dev/emergency use",
                    error
                ));
            }
            file_save_secret(root, profile_id, api_key)?;
            Ok((
                true,
                "file".to_string(),
                format!(
                    "OS secure storage unavailable; API Key saved to plaintext app data file fallback because EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK is enabled: {}",
                    error
                ),
            ))
        }
    }
}

pub(crate) fn load_profile_secret(root: &Path, profile_id: &str) -> Option<String> {
    if !is_safe_path_segment(profile_id) {
        return None;
    }
    match os_secret_load_secret(profile_id) {
        Ok(Some(secret)) => Some(secret),
        _ if plaintext_secret_fallback_allowed() => file_load_secret(root, profile_id),
        _ => None,
    }
}

pub(crate) fn load_llm_api_key(root: &Path, profile_id: &str) -> Option<String> {
    load_profile_secret(root, profile_id).filter(|value| !value.trim().is_empty())
}

pub(crate) fn load_profiles(root: &Path) -> CommandResult<Vec<Value>> {
    let path = profiles_path(root);
    if !path.exists() {
        return Ok(vec![redact_profile_for_ui(
            root,
            json!({"profileId":"profile-local-placeholder","name":"Local JSON Gateway","provider":"OpenAiCompatible","baseUrl":"http://localhost:11434/v1","model":"local-structurer","temperature":0,"timeoutMs":60000,"forceJson":true,"enabled":true}),
        )]);
    }
    Ok(read_json::<Vec<Value>>(&path)?
        .into_iter()
        .map(|profile| redact_profile_for_ui(root, profile))
        .collect())
}

pub(crate) fn save_profiles(root: &Path, profiles: &[Value]) -> CommandResult<()> {
    write_json(&profiles_path(root), profiles)
}

pub(crate) fn find_profile(root: &Path, profile_id: &str) -> CommandResult<Value> {
    load_profiles(root)?
        .into_iter()
        .find(|profile| profile.get("profileId").and_then(Value::as_str) == Some(profile_id))
        .ok_or_else(|| format!("profile_not_found:{}", profile_id))
}
