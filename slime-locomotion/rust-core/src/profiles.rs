//! Local, bounded storage for locomotion profiles. Profiles contain settings only;
//! tracker identities and hardware credentials are deliberately not accepted.

use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SCHEMA_VERSION: u64 = 1;
const MAX_PROFILES: usize = 100;
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
const MAX_NAME_BYTES: usize = 64;
static TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub struct ProfileStore {
    config_parent: PathBuf,
}

impl ProfileStore {
    pub fn new(config_parent: PathBuf) -> Self {
        Self { config_parent }
    }

    fn directory(&self, create: bool) -> Result<PathBuf, String> {
        if create {
            fs::create_dir_all(&self.config_parent).map_err(|e| e.to_string())?;
        }
        let parent = fs::canonicalize(&self.config_parent).map_err(|e| e.to_string())?;
        let dir = parent.join("profiles");
        if create && !dir.exists() {
            fs::create_dir(&dir).map_err(|e| e.to_string())?;
        }
        let metadata = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("profiles path must be a real directory".into());
        }
        let canonical = fs::canonicalize(&dir).map_err(|e| e.to_string())?;
        if !canonical.starts_with(&parent) {
            return Err("profiles directory escapes the configuration directory".into());
        }
        Ok(canonical)
    }

    fn validate_name(name: &str) -> Result<(), String> {
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err("profile name must contain 1 to 64 ASCII characters".into());
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'_' || b == b'-')
        {
            return Err("profile name may use only ASCII letters, digits, spaces, _ and -".into());
        }
        Ok(())
    }

    fn file_name(name: &str) -> String {
        let mut encoded = String::with_capacity(name.len() * 2 + 5);
        for byte in name.bytes() {
            use std::fmt::Write as _;
            let _ = write!(encoded, "{byte:02x}");
        }
        format!("{encoded}.json")
    }

    fn validate_settings(settings: &Value) -> Result<(), String> {
        let object = settings
            .as_object()
            .ok_or_else(|| "profile settings must be a JSON object".to_string())?;
        const ALLOWED: &[&str] = &[
            "sensor_layout",
            "step_input",
            "locomotion",
            "foot_motion",
            "head_recenter",
            "actions",
            "gestures",
            "movement_engine",
            "detection",
            "outputmode",
            "output_mode",
        ];
        if object.is_empty() || object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
            return Err("settings may contain only locomotion, actions, movement_engine, detection and output mode".into());
        }
        if let Some(settings)=object.get("head_recenter") {
            crate::head_recenter::Settings::from_json(settings).map_err(str::to_owned)?;
        }
        if let Some(settings)=object.get("foot_motion") {
            crate::foot_motion::Settings::from_json(settings).map_err(str::to_owned)?;
        }
        if let Some(source)=object.get("step_input") {if !matches!(source.as_str(),Some("off"|"chest"|"feet")){return Err("invalid profile step_input".into());}}
        if let Some(layout)=object.get("sensor_layout") {
            let layout=layout.as_object().ok_or("sensor_layout must be an object")?;
            if layout.len()!=1 || !matches!(layout.get("count").and_then(Value::as_u64),Some(1|3|5)) {return Err("sensor_layout must contain only count 1, 3, or 5".into());}
        }
        Ok(())
    }

    #[cfg(test)]
    fn test_validate_settings(settings:&Value)->Result<(),String> {Self::validate_settings(settings)}

    fn decode_profile(bytes: &[u8]) -> Result<Value, String> {
        let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let obj = value
            .as_object()
            .ok_or_else(|| "profile must be a JSON object".to_string())?;
        if obj.get("schema_version").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
            return Err("unsupported profile schema_version".into());
        }
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "profile name is missing".to_string())?;
        Self::validate_name(name)?;
        if obj.get("description").and_then(Value::as_str).is_none() {
            return Err("profile description must be text".into());
        }
        Self::validate_settings(
            obj.get("settings")
                .ok_or_else(|| "profile settings are missing".to_string())?,
        )?;
        Ok(value)
    }

    fn read_file(path: &Path) -> Result<Value, String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("profile must be a regular file".into());
        }
        if metadata.len() > MAX_PROFILE_BYTES {
            return Err("profile exceeds 1 MiB".into());
        }
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_PROFILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_PROFILE_BYTES {
            return Err("profile exceeds 1 MiB".into());
        }
        Self::decode_profile(&bytes)
    }

    pub fn list(&self) -> Result<Vec<Value>, String> {
        let dir = self.directory(false)?;
        let mut profiles = Vec::new();
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if profiles.len() == MAX_PROFILES {
                return Err("profile directory exceeds the 100-profile limit".into());
            }
            profiles.push(Self::read_file(&entry.path())?);
        }
        profiles.sort_by(|a, b| {
            a.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase()
                .cmp(
                    &b.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                )
        });
        Ok(profiles)
    }

    pub fn save(&self, name: &str, description: &str, settings: Value) -> Result<(), String> {
        Self::validate_name(name)?;
        if description.len() > 4096 {
            return Err("profile description exceeds 4096 bytes".into());
        }
        Self::validate_settings(&settings)?;
        let dir = self.directory(true)?;
        let target = dir.join(Self::file_name(name));
        if !target.exists()
            && fs::read_dir(&dir).map_err(|e| e.to_string())?.count() >= MAX_PROFILES
        {
            return Err("profile directory already contains 100 profiles".into());
        }
        let profile = json!({
            "schema_version": SCHEMA_VERSION,
            "name": name,
            "description": description,
            "settings": settings,
        });
        let data = serde_json::to_vec_pretty(&profile).map_err(|e| e.to_string())?;
        if data.len() as u64 > MAX_PROFILE_BYTES {
            return Err("profile exceeds 1 MiB".into());
        }
        let temp = dir.join(format!(
            ".profile-{}-{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let write_result = file.write_all(&data).and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp);
            return Err(error.to_string());
        }
        if let Err(error) = fs::rename(&temp, &target) {
            let _ = fs::remove_file(&temp);
            return Err(error.to_string());
        }
        Ok(())
    }

    pub fn load(&self, name: &str) -> Result<Value, String> {
        Self::validate_name(name)?;
        let dir = self.directory(false)?;
        let value = Self::read_file(&dir.join(Self::file_name(name)))?;
        if value.get("name").and_then(Value::as_str) != Some(name) {
            return Err("profile filename and embedded name do not match".into());
        }
        Ok(value
            .get("settings")
            .expect("validated profile settings")
            .clone())
    }

    pub fn delete(&self, name: &str) -> Result<(), String> {
        Self::validate_name(name)?;
        let dir = self.directory(false)?;
        let path = dir.join(Self::file_name(name));
        let value = Self::read_file(&path)?;
        if value.get("name").and_then(Value::as_str) != Some(name) {
            return Err("profile filename and embedded name do not match".into());
        }
        fs::remove_file(path).map_err(|e| e.to_string())
    }

    pub fn import(&self, value: Value) -> Result<(), String> {
        let profile =
            Self::decode_profile(&serde_json::to_vec(&value).map_err(|e| e.to_string())?)?;
        let name = profile
            .get("name")
            .and_then(Value::as_str)
            .expect("validated name");
        let description = profile
            .get("description")
            .and_then(Value::as_str)
            .expect("validated description");
        let settings = profile.get("settings").expect("validated settings").clone();
        self.save(name, description, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "vrcraft-profiles-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn sample() -> Value {
        json!({ "locomotion": { "gain": 1.2 }, "actions": {}, "movement_engine": {}, "detection": {}, "outputmode": "preview" })
    }

    #[test]
    fn layout_contains_only_a_valid_count() {
        assert!(ProfileStore::test_validate_settings(&json!({"sensor_layout":{"count":5}})).is_ok());
        assert!(ProfileStore::test_validate_settings(&json!({"sensor_layout":{"count":2}})).is_err());
        assert!(ProfileStore::test_validate_settings(&json!({"sensor_layout":{"count":5,"thighs":["id"]}})).is_err());
    }
    #[test]
    fn save_load_replace_and_delete_roundtrip() {
        let root = temp_dir();
        let store = ProfileStore::new(root.clone());
        store.save("Gentle 1", "first", sample()).unwrap();
        assert_eq!(store.load("Gentle 1").unwrap(), sample());
        store
            .save("Gentle 1", "updated", json!({ "locomotion": {} }))
            .unwrap();
        assert_eq!(store.list().unwrap()[0]["description"], "updated");
        store.delete("Gentle 1").unwrap();
        assert!(store.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_bad_names_and_import_payloads() {
        let root = temp_dir();
        let store = ProfileStore::new(root.clone());
        let names = vec![
            String::new(),
            "../escape".into(),
            "a/b".into(),
            "☃".into(),
            "x".repeat(65),
        ];
        for name in names {
            assert!(store.save(&name, "", sample()).is_err());
        }
        assert!(store.import(json!({ "schema_version": 1, "name": "x", "description": "", "settings": { "tracker_password": "secret" } })).is_err());
        assert!(store
            .import(
                json!({ "schema_version": 9, "name": "x", "description": "", "settings": sample() })
            )
            .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_roundtrip_and_size_limit() {
        let root = temp_dir();
        let store = ProfileStore::new(root.clone());
        let document = json!({ "schema_version": 1, "name": "Imported", "description": "from file", "settings": sample() });
        store.import(document.clone()).unwrap();
        assert_eq!(store.list().unwrap(), vec![document]);
        let huge = json!({ "locomotion": "x".repeat(1024 * 1024) });
        assert!(store.save("Too big", "", huge).is_err());
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod foot_motion_tests {
    use super::*;
    #[test]
    fn foot_motion_profile_roundtrip_and_strict_identity_rejection() {
        let dir=std::env::temp_dir().join(format!("foot-profile-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&dir).unwrap(); let store=ProfileStore::new(dir.clone());
        let settings=json!({"foot_motion":{"enabled":false,"min_swing_degrees":1.,"pivot_ratio":3.}});
        store.save("Foot tuning","",settings.clone()).unwrap();
        assert_eq!(store.load("Foot tuning").unwrap(),settings);
        for invalid in [json!({"foot_motion":{"trackers":{"left":"private-id"}}}),json!({"foot_motion":{"enabled":true,"min_swing_degrees":9.}}),json!({"foot_motion":null}),json!({"foot_motion":{"credentials":"private"}})] {
            assert!(store.save("Rejected","",invalid).is_err());
        }
        assert_eq!(store.list().unwrap().len(),1);
        fs::remove_dir_all(dir).unwrap();
    }
}
