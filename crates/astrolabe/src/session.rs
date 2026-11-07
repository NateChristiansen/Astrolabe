//! Owned session state. JSON is confined to reading and writing compatible records.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Older records accepted missing, null and unexpected values in optional fields. Keep
/// those values verbatim until a transition replaces the field, rather than rejecting or
/// silently deleting them. Missing and explicit null remain distinct on disk.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Field<T> {
    Known(T),
    Other(Value),
    #[default]
    Missing,
}

impl<T> Field<T> {
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            _ => None,
        }
    }
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
    pub fn set(&mut self, value: T) {
        *self = Self::Known(value);
    }
    pub fn clear(&mut self) {
        *self = Self::Other(Value::Null);
    }
}
impl Field<String> {
    pub fn as_deref(&self) -> Option<&str> {
        self.as_ref().map(String::as_str)
    }
}
impl Field<bool> {
    pub fn is_true(&self) -> bool {
        self.as_ref() == Some(&true)
    }
}
impl<T> From<T> for Field<T> {
    fn from(value: T) -> Self {
        Self::Known(value)
    }
}

/// A page and a numbered effort have distinct identities even when their keys coincide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Binding {
    Effort {
        key: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    Initiative {
        key: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
}

impl Binding {
    pub fn effort(key: String) -> Self {
        Self::Effort {
            key,
            extra: Map::new(),
        }
    }
    pub fn initiative(key: String) -> Self {
        Self::Initiative {
            key,
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub binding: Field<Binding>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub session: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub effort: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub provider: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub file: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub repo: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub cwd: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub token: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub delivered: Field<String>,
    /// Notes explicitly read by number in this session, separate from the bound record.
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub read_efforts: Field<std::collections::BTreeMap<String, String>>,
    /// When this exact note version was delivered; older replies cannot answer its wait.
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub delivered_ms: Field<i64>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub baseline: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub acknowledged: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub reload_failed: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub at_ms: Field<i64>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub last_save_ms: Field<i64>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub closed_ms: Field<i64>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub saves: Field<u64>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub mark: Field<bool>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub worker: Field<bool>,
    /// What the app should call this session: the effort's number and title.
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub name: Field<String>,
    /// The name Astrolabe last gave the session, so a rename by the owner after it is kept.
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub named: Field<String>,
    /// The name the session's agent was last told to rename it to, with the app's own tool.
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub told: Field<String>,
    /// Whether the session took up its effort's work, or only filed it for later.
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub taken: Field<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SessionState {
    /// Remember the version delivered by an explicit read or a checked write by number.
    pub fn deliver_effort(&mut self, number: u32, hash: String) {
        let mut reads = self.read_efforts.as_ref().cloned().unwrap_or_default();
        reads.insert(number.to_string(), hash);
        self.read_efforts.set(reads);
    }

    pub fn initiative_key(&self) -> Option<&str> {
        match self.binding.as_ref() {
            Some(Binding::Initiative { key, .. }) => Some(key),
            _ => None,
        }
    }
    pub fn bound_key(&self) -> Option<&str> {
        match self.binding.as_ref() {
            Some(Binding::Effort { key, .. } | Binding::Initiative { key, .. }) => Some(key),
            None if self.binding.is_missing() => self.effort.as_deref(),
            _ => None,
        }
    }
    pub fn record_kind(&self) -> &'static str {
        if self.initiative_key().is_some() {
            "initiative"
        } else {
            "effort"
        }
    }
    pub fn record_label(&self) -> String {
        format!(
            "{} {}",
            self.record_kind(),
            self.bound_key().unwrap_or_default()
        )
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("session fields are JSON values")
    }
    pub fn saved(&mut self, delivered: String, baseline: Option<String>, at_ms: i64) {
        self.delivered.set(delivered);
        self.delivered_ms.set(at_ms);
        self.baseline = baseline
            .map(Field::Known)
            .unwrap_or(Field::Other(Value::Null));
        self.last_save_ms.set(at_ms);
        self.saves
            .set(self.saves.as_ref().copied().unwrap_or(0) + 1);
        self.mark.set(false);
        self.acknowledged.clear();
    }
    pub fn acknowledge(&mut self, fingerprint: Option<&str>) {
        self.acknowledged = fingerprint
            .map(|p| Field::Known(p.to_owned()))
            .unwrap_or(Field::Other(Value::Null));
        self.mark.set(false);
    }
}
impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_value().fmt(f)
    }
}

pub(crate) fn state_path(dir: &Path, session: &str) -> PathBuf {
    let safe = |c: char| {
        if c.is_ascii_alphanumeric() || c == '-' {
            c
        } else {
            '_'
        }
    };
    dir.join(format!(
        "{}.json",
        session.chars().map(safe).collect::<String>()
    ))
}

/// None when this session is off. An error when it's on but the file can't be read.
pub fn read_state(dir: &Path, session: &str) -> Result<Option<SessionState>, String> {
    let path = state_path(dir, session);
    match std::fs::read(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
        Ok(bytes) => match serde_json::from_slice::<SessionState>(&bytes) {
            Ok(state)
                if state.session.as_deref() == Some(session) && state.bound_key().is_some() =>
            {
                Ok(Some(state))
            }
            _ => Err(format!(
                "{} isn't a state file this program wrote",
                path.display()
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_fields_and_unknown_data_survive_a_save() {
        let original = json!({"session":"s", "effort":"3", "worker":"legacy", "provider":null,
            "future":{"nested":[1,true]}, "closed_ms":"unknown"});
        let mut state: SessionState = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(state.to_value(), original);
        assert_eq!(state.bound_key(), Some("3"));
        assert_eq!(state.initiative_key(), None);
        assert!(!state.worker.is_true());
        assert!(state.token.is_missing());
        state.saved("hash".into(), None, 17);
        let saved = state.to_value();
        for key in [
            "session",
            "effort",
            "worker",
            "provider",
            "future",
            "closed_ms",
        ] {
            assert_eq!(saved[key], original[key]);
        }
        assert_eq!(saved["saves"], 1);
        assert_eq!(saved["baseline"], Value::Null);
        assert!(saved.get("token").is_none());
    }
}
