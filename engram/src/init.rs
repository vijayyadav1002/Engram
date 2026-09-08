use crate::error::Error;
use crate::store::Store;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const DEFAULT_ENGRAMIGNORE: &str = "\
node_modules
.venv
dist
build
target
.next
*.min.js
";

const GROK_MCP_TOML: &str = "\
[mcp_servers.engram]
command = \"engram\"
args = [\"mcp\"]
";

const SKILL_MD: &str = "\
# Engram + MemPalace

Call Engram `get_context` first for repo questions; do not grep the tree until
the package is empty or `stale_index` is true.

Call MemPalace `mempalace_search` first for prior sessions, decisions, and
people. Quote drawers verbatim.

For \"why did we…\" call `get_context` then palace search. If they conflict,
current code wins.
";

const AGENTS_BLURB: &str = "\
## Engram + MemPalace

Prefer Engram `get_context` before searching the repo; do not grep until the
package is empty or `stale_index` is true.

Prefer MemPalace `mempalace_search` for prior decisions and sessions; quote
drawers verbatim.

For \"why\" questions use both (Engram first). If they conflict, current code
wins.
";

/// Create `.engram/`, empty DB, default `.engramignore`, and a gitignore entry.
/// Does not index. Uses `cwd` as root even without repo markers.
pub fn run_init(cwd: &Path) -> Result<PathBuf, Error> {
    let engram_dir = cwd.join(".engram");
    std::fs::create_dir_all(&engram_dir)?;

    let db = engram_dir.join("index.sqlite");
    if !db.exists() {
        let root = cwd.to_string_lossy();
        Store::create(&db, &root)?;
    }

    let ignore = cwd.join(".engramignore");
    if !ignore.exists() {
        std::fs::write(&ignore, DEFAULT_ENGRAMIGNORE)?;
    }

    let gitignore = cwd.join(".gitignore");
    if gitignore.is_file() {
        append_engram_gitignore(&gitignore)?;
    }

    Ok(cwd.to_path_buf())
}

/// Write project-scoped MCP harness snippets. `id`: grok|copilot|claude|cursor|all.
pub fn write_harness(root: &Path, id: &str) -> Result<(), Error> {
    match id {
        "grok" => write_grok_toml(root),
        "copilot" | "claude" => merge_mcp_json(&root.join(".mcp.json")),
        "cursor" => {
            let path = root.join(".cursor/mcp.json");
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            merge_mcp_json(&path)
        }
        "all" => {
            write_grok_toml(root)?;
            merge_mcp_json(&root.join(".mcp.json"))?;
            let cursor = root.join(".cursor/mcp.json");
            if let Some(parent) = cursor.parent() {
                std::fs::create_dir_all(parent)?;
            }
            merge_mcp_json(&cursor)?;
            Ok(())
        }
        other => Err(Error::Usage(format!(
            "unknown harness `{other}`; expected grok|copilot|claude|cursor|all"
        ))),
    }
}

/// Write `.grok/skills/engram/SKILL.md`; optionally Claude copy and AGENTS.md.
pub fn write_skill(root: &Path, also_claude: bool, write_agents: bool) -> Result<(), Error> {
    let grok_skill = root.join(".grok/skills/engram/SKILL.md");
    if let Some(parent) = grok_skill.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&grok_skill, SKILL_MD)?;

    if also_claude {
        let claude_skill = root.join(".claude/skills/engram/SKILL.md");
        if let Some(parent) = claude_skill.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&claude_skill, SKILL_MD)?;
    }

    let agents = root.join("AGENTS.md");
    if agents.is_file() {
        append_agents_blurb(&agents)?;
    } else if write_agents {
        std::fs::write(&agents, AGENTS_BLURB)?;
    }

    Ok(())
}

fn write_grok_toml(root: &Path) -> Result<(), Error> {
    let path = root.join(".grok/config.toml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.is_file() {
        let mut contents = std::fs::read_to_string(&path)?;
        if contents.contains("[mcp_servers.engram]") {
            return Ok(());
        }
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(GROK_MCP_TOML);
        std::fs::write(&path, contents)?;
    } else {
        std::fs::write(&path, GROK_MCP_TOML)?;
    }
    Ok(())
}

fn engram_mcp_entry() -> Value {
    json!({
        "type": "stdio",
        "command": "engram",
        "args": ["mcp"]
    })
}

fn merge_mcp_json(path: &Path) -> Result<(), Error> {
    let engram = engram_mcp_entry();
    let mut root_obj: Map<String, Value> = if path.is_file() {
        let text = std::fs::read_to_string(path)?;
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            Ok(_) | Err(_) => Map::new(),
        }
    } else {
        Map::new()
    };

    let key = if root_obj.contains_key("servers") && !root_obj.contains_key("mcpServers") {
        "servers"
    } else {
        "mcpServers"
    };

    let servers = root_obj.entry(key.to_string()).or_insert_with(|| json!({}));
    let map = match servers {
        Value::Object(m) => m,
        _ => {
            *servers = json!({});
            servers.as_object_mut().expect("object")
        }
    };
    map.insert("engram".to_string(), engram);

    let out = Value::Object(root_obj);
    let pretty = serde_json::to_string_pretty(&out).map_err(|e| Error::Usage(e.to_string()))?;
    std::fs::write(path, pretty + "\n")?;
    Ok(())
}

fn append_agents_blurb(path: &Path) -> Result<(), Error> {
    let mut contents = std::fs::read_to_string(path)?;
    if contents.contains("get_context") {
        return Ok(());
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    if !contents.is_empty() {
        contents.push('\n');
    }
    contents.push_str(AGENTS_BLURB);
    std::fs::write(path, contents)?;
    Ok(())
}

fn append_engram_gitignore(path: &Path) -> Result<(), Error> {
    let mut contents = std::fs::read_to_string(path)?;
    let has_entry = contents.lines().any(|line| {
        let t = line.trim();
        t == ".engram/" || t == ".engram"
    });
    if has_entry {
        return Ok(());
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(".engram/\n");
    std::fs::write(path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tempfile_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("engram-init-{}-{}", std::process::id(), n));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn init_creates_db_and_gitignore_entry() {
        let root = tempfile_dir();
        std::fs::write(root.join(".gitignore"), "node_modules\n").unwrap();
        crate::init::run_init(&root).unwrap();
        assert!(root.join(".engram/index.sqlite").is_file());
        assert!(root.join(".engramignore").is_file());
        let gi = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(gi.contains(".engram/"));
    }

    #[test]
    fn harness_grok_writes_toml() {
        let root = tempfile_dir();
        crate::init::run_init(&root).unwrap();
        crate::init::write_harness(&root, "grok").unwrap();
        let t = std::fs::read_to_string(root.join(".grok/config.toml")).unwrap();
        assert!(t.contains("[mcp_servers.engram]"));
        assert!(t.contains("command = \"engram\""));
        assert!(t.contains("mcp"));
    }

    #[test]
    fn harness_copilot_merges_mcp_json() {
        let root = tempfile_dir();
        std::fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"other":{"command":"x"}}}"#,
        )
        .unwrap();
        crate::init::run_init(&root).unwrap();
        crate::init::write_harness(&root, "copilot").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".mcp.json")).unwrap())
                .unwrap();
        assert!(v["mcpServers"]["other"].is_object() || v["servers"]["other"].is_object());
        let engram = v
            .pointer("/mcpServers/engram")
            .or_else(|| v.pointer("/servers/engram"));
        assert!(engram.is_some());
    }

    #[test]
    fn skill_does_not_create_agents_unless_asked() {
        let root = tempfile_dir();
        crate::init::run_init(&root).unwrap();
        crate::init::write_skill(&root, false, false).unwrap();
        assert!(root.join(".grok/skills/engram/SKILL.md").is_file());
        assert!(!root.join("AGENTS.md").exists());
        std::fs::write(root.join("AGENTS.md"), "# hi\n").unwrap();
        crate::init::write_skill(&root, false, false).unwrap();
        let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(agents.contains("get_context"));
    }

    #[test]
    fn skill_write_agents_creates_file() {
        let root = tempfile_dir();
        crate::init::run_init(&root).unwrap();
        crate::init::write_skill(&root, true, true).unwrap();
        assert!(root.join("AGENTS.md").is_file());
        assert!(root.join(".claude/skills/engram/SKILL.md").is_file());
    }
}
