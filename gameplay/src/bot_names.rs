//! Synthetic defaults and explicitly loaded, per-engine operator bot identities.
//! No game catalog or real-person list is embedded.
use std::sync::OnceLock;
pub const BOT_NAME_COUNT: usize = 256;
/// No real-person identities are distributed in the public package.
pub const ZIPPER_NAME_COUNT: usize = 0;
pub const MAX_NAME_BYTES: usize = 32;
fn names() -> &'static Vec<String> {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        (0..BOT_NAME_COUNT)
            .map(|i| {
                format!(
                    "Synthetic Bot {}{}",
                    (b'A' + (i / 26) as u8) as char,
                    (b'A' + (i % 26) as u8) as char
                )
            })
            .collect()
    })
}
/// Stable slot identity; unsupported slots never wrap.
pub fn name_for(index: u16) -> Option<&'static str> {
    names().get(usize::from(index)).map(String::as_str)
}
/// An immutable per-engine identity list; operator metadata is not used at runtime.
#[derive(Clone, Debug)]
pub struct Catalog {
    names: Vec<String>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            names: names().clone(),
        }
    }
}
impl Catalog {
    pub fn from_json(bytes: &[u8]) -> std::io::Result<Self> {
        use serde::Deserialize;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Document {
            entries: Vec<Entry>,
        }
        #[derive(Deserialize)]
        struct Entry {
            name: String,
        }
        let invalid = || {
            std::io::Error::other(
                "invalid bot names: require 1..256 unique printable ASCII names of 1..32 bytes, without surrounding whitespace",
            )
        };
        if bytes.len() > 131_072 {
            return Err(invalid());
        }
        let doc: Document = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if doc.entries.is_empty() || doc.entries.len() > BOT_NAME_COUNT {
            return Err(invalid());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut names = Vec::with_capacity(doc.entries.len());
        for entry in doc.entries {
            let name = entry.name;
            if name.is_empty()
                || name.len() > MAX_NAME_BYTES
                || name.trim() != name
                || !name.bytes().all(|b| (b' '..=b'~').contains(&b))
                || !seen.insert(name.to_ascii_lowercase())
            {
                return Err(invalid());
            }
            names.push(name);
        }
        Ok(Self { names })
    }
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(131_073)
            .read_to_end(&mut bytes)?;
        Self::from_json(&bytes)
    }
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
    pub fn name_for(&self, index: u16) -> Option<&str> {
        self.names.get(usize::from(index)).map(String::as_str)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_names_are_unique_stable_and_bounded() {
        let names: std::collections::BTreeSet<_> = (0..BOT_NAME_COUNT as u16)
            .map(|i| name_for(i).unwrap())
            .collect();
        assert_eq!(names.len(), BOT_NAME_COUNT);
        for i in 0..=u16::MAX {
            assert_eq!(name_for(i).is_some(), usize::from(i) < BOT_NAME_COUNT);
            assert_eq!(name_for(i), name_for(i));
        }
        assert!(names.iter().all(|n| n.len() <= MAX_NAME_BYTES
            && n.bytes().all(|b| b.is_ascii_alphabetic() || b == b' ')));
    }
}

#[cfg(test)]
mod external_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn external_names_preserve_order_and_are_instance_local() {
        let a = Catalog::from_json(&serde_json::to_vec(&json!({"entries":[
            {"name":"Synthetic Pilot Alpha","provenance":"synthetic","source":"https://example.invalid"},
            {"name":"Synthetic Pilot Bravo"}]})).unwrap()).unwrap();
        let b = Catalog::default();
        assert_eq!(a.name_for(0), Some("Synthetic Pilot Alpha"));
        assert_eq!(a.name_for(1), Some("Synthetic Pilot Bravo"));
        assert_eq!(a.name_for(2), None);
        assert_eq!(a.len(), 2);
        assert_eq!(b.name_for(0), Some("Synthetic Bot AA"));
    }
    #[test]
    fn external_names_accept_wire_and_capacity_boundaries() {
        let mut entries: Vec<_> = (0..256)
            .map(|i| json!({"name":format!("Synthetic {i}")}))
            .collect();
        entries[0]["name"] = json!("X".repeat(32));
        let catalog =
            Catalog::from_json(&serde_json::to_vec(&json!({"entries":entries})).unwrap()).unwrap();
        assert_eq!(catalog.len(), 256);
        assert_eq!(catalog.name_for(0).unwrap().len(), 32);
        assert_eq!(catalog.name_for(255), Some("Synthetic 255"));
        assert_eq!(catalog.name_for(256), None);
    }
    #[test]
    fn external_names_reject_invalid_identity_lists() {
        for value in [
            json!({"entries":[]}),
            json!({"entries":[{"name":"Alpha"},{"name":"alpha"}]}),
            json!({"entries":[{"name":""}]}),
            json!({"entries":[{"name":" Alpha"}]}),
            json!({"entries":[{"name":"Alpha\nBravo"}]}),
            json!({"entries":[{"name":"x".repeat(33)}]}),
            json!({"entries":[{"name":"Pilót"}]}),
            json!({"entries":[{}]}),
            json!({"entries":(0..257).map(|i|json!({"name":format!("Synthetic {i}")})).collect::<Vec<_>>()}),
        ] {
            assert!(
                Catalog::from_json(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{value}"
            );
        }
        assert!(Catalog::from_json(b"not json").is_err());
        assert!(Catalog::from_json(&vec![b' '; 131_073]).is_err());
        assert!(
            Catalog::load(std::path::Path::new(
                "/nonexistent-synthetic-bot-names.json"
            ))
            .is_err()
        );
    }
}
