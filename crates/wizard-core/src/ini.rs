//! A tiny KConfig-style INI reader shared by installer.ini and the markers.

use std::collections::HashMap;

/// Parsed INI: `(group, key) -> value`, values trimmed. Later duplicates win.
/// Lines that are not `[group]` or `key=value` are skipped. Keys with a
/// `[locale]` suffix (KConfig translations) are skipped.
#[derive(Debug, Default)]
pub(crate) struct Ini {
    values: HashMap<(String, String), String>,
    groups: Vec<String>,
}

impl Ini {
    pub(crate) fn parse(text: &str) -> Ini {
        let mut ini = Ini::default();
        let mut group = String::new();
        for raw in text.lines() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(name) = rest.strip_suffix(']') {
                    group = name.trim().to_string();
                    if !ini.groups.contains(&group) {
                        ini.groups.push(group.clone());
                    }
                }
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                if key.is_empty() || key.contains('[') {
                    continue;
                }
                ini.values
                    .insert((group.clone(), key.to_string()), value.trim().to_string());
            }
        }
        ini
    }

    pub(crate) fn has_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g == group)
    }

    /// The value, or `None` when missing or empty.
    pub(crate) fn get(&self, group: &str, key: &str) -> Option<&str> {
        self.values
            .get(&(group.to_string(), key.to_string()))
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}
