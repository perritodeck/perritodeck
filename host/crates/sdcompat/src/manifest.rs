//! A Stream Deck plugin's `manifest.json` (the parts PerritoDeck uses).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Manifest {
    /// Missing in older manifests; then it's the folder name without `.sdPlugin`.
    #[serde(rename = "UUID", default)]
    pub uuid: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub code_path: Option<String>,
    #[serde(default)]
    pub code_path_win: Option<String>,
    #[serde(default)]
    pub code_path_mac: Option<String>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub nodejs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Action {
    #[serde(rename = "UUID")]
    pub uuid: String,
    pub name: String,
    #[serde(default)]
    pub tooltip: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub states: Vec<State>,
    /// `Keypad`, `Encoder`; missing means `Keypad`.
    #[serde(default)]
    pub controllers: Option<Vec<String>>,
    #[serde(default)]
    pub property_inspector_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct State {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

impl Action {
    /// Whether it works on a key (rather than only on a dial).
    pub fn on_keys(&self) -> bool {
        self.controllers
            .as_ref()
            .map_or(true, |c| c.is_empty() || c.iter().any(|x| x == "Keypad"))
    }
}

/// How a plugin's code runs here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Code {
    /// `node <file>`
    Node(PathBuf),
    /// The program itself.
    Native(PathBuf),
    /// Can't run here, and why.
    Unsupported(String),
}

impl Manifest {
    pub fn read(folder: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(folder.join("manifest.json"))
            .map_err(|_| "it has no manifest.json; that isn't a Stream Deck plugin".to_owned())?;
        // Some manifests start with a byte-order mark.
        let mut m: Manifest = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("its manifest.json can't be read: {e}"))?;
        if m.uuid.is_empty() {
            let name = folder
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            m.uuid = name.trim_end_matches(".sdPlugin").to_owned();
        }
        if m.uuid.is_empty()
            || !m
                .uuid
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(format!("its UUID {:?} isn't valid", m.uuid));
        }
        Ok(m)
    }

    /// What to run on this computer.
    pub fn code(&self, folder: &Path) -> Code {
        let os_specific = if cfg!(windows) {
            self.code_path_win.as_ref()
        } else if cfg!(target_os = "macos") {
            self.code_path_mac.as_ref()
        } else {
            None
        };
        let Some(rel) = os_specific.or(self.code_path.as_ref()) else {
            return Code::Unsupported("it doesn't say how to start it".into());
        };
        let Some(path) = inside(folder, rel) else {
            return Code::Unsupported(format!("its code path {rel:?} leaves the plugin folder"));
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "js" | "mjs" | "cjs" => Code::Node(path),
            "html" | "htm" => Code::Unsupported(
                "it's an older HTML plugin, which needs a browser engine to run".into(),
            ),
            "exe" if !cfg!(windows) => Code::Unsupported("it only runs on Windows".into()),
            _ if !path.is_file() => Code::Unsupported(format!("{rel} is missing")),
            _ if !cfg!(windows) && !cfg!(target_os = "macos") => {
                Code::Unsupported("it's a Windows or macOS program".into())
            }
            _ => Code::Native(path),
        }
    }

    pub fn action(&self, uuid: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.uuid == uuid)
    }
}

/// `rel` under `folder`, refusing anything that climbs out (`..`, absolute paths).
pub fn inside(folder: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    if rel.is_absolute()
        || rel.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return None;
    }
    Some(folder.join(rel))
}

/// An image named without its extension, as manifests do: `imgs/key` → `imgs/key@2x.png`,
/// `imgs/key.png`, `imgs/key.svg` (first found).
pub fn image_file(folder: &Path, base: &str) -> Option<PathBuf> {
    let base = inside(folder, base)?;
    let s = base.to_string_lossy().into_owned();
    [
        format!("{s}@2x.png"),
        format!("{s}.png"),
        format!("{s}.svg"),
        format!("{s}.jpg"),
        s.clone(),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("com.example.counter.sdPlugin");
        std::fs::create_dir_all(folder.join("bin")).unwrap();
        std::fs::write(
            folder.join("manifest.json"),
            "\u{feff}{\"Name\":\"Counter\",\"Version\":\"1.0.0.0\",\"CodePath\":\"bin/plugin.js\",\
             \"Actions\":[{\"UUID\":\"com.example.counter.up\",\"Name\":\"Up\",\"States\":[{\"Image\":\"imgs/key\"}]},\
             {\"UUID\":\"com.example.counter.dial\",\"Name\":\"Dial\",\"Controllers\":[\"Encoder\"]}],\
             \"SDKVersion\":2,\"Nodejs\":{\"Version\":\"20\"}}",
        )
        .unwrap();
        let m = Manifest::read(&folder).unwrap();
        assert_eq!(m.uuid, "com.example.counter", "from the folder name");
        assert!(m.actions[0].on_keys() && !m.actions[1].on_keys());
        assert_eq!(m.code(&folder), Code::Node(folder.join("bin/plugin.js")));
        assert!(inside(&folder, "../x").is_none() && inside(&folder, "/etc/passwd").is_none());
        std::fs::create_dir_all(folder.join("imgs")).unwrap();
        std::fs::write(folder.join("imgs/key.png"), b"x").unwrap();
        assert_eq!(
            image_file(&folder, "imgs/key"),
            Some(folder.join("imgs/key.png"))
        );
    }
}
