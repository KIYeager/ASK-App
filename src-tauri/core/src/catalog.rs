//! Kataloge für Skills und Mods (ENTSCHEIDE E71): öffentliche GitHub-Repositories, in denen Ordner mit
//! `SKILL.md` (Skill) oder `mod.json` (Mod) liegen, z. B. anthropics/skills oder das Repository der Schule.
//! Gelesen wird ohne Anmeldung: einmal die Liste der Dateien (GitHub-API, `git/trees`), dann die Dateien selbst
//! über raw.githubusercontent.com. Installiert wird nichts ohne Klick der Person; ein Mod ist danach aus.
//!
//! Öffentliche Schnittstelle: `parse_source`, `entries_from_tree`, `Catalog::new(client)`, `list`, `files`.

use anyhow::{anyhow, bail, Result};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const API: &str = "https://api.github.com";
const RAW: &str = "https://raw.githubusercontent.com";
const AGENT: &str = "ASK-Desktop";
const CACHE: Duration = Duration::from_secs(10 * 60);
/// Höchstens so viele Einträge pro Katalog (Beschreibungen werden einzeln geladen)
pub const MAX_ENTRIES: usize = 80;

/// Ort in einem Repository: owner/repo, Branch (leer: Standard) und Ordner (leer: ganzes Repository)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Source {
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub path: String,
}

impl Source {
    pub fn repo_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

fn valid_part(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// «owner/repo», «github.com/owner/repo», «https://github.com/owner/repo/tree/main/skills/x» oder ein Link auf
/// eine Datei (…/blob/main/skills/x/SKILL.md, dann gilt ihr Ordner)
pub fn parse_source(text: &str) -> Result<Source> {
    let t = text.trim().trim_end_matches('/');
    let rest = t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")).unwrap_or(t);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_prefix("github.com/").unwrap_or(rest);
    if rest.contains("://") || rest.starts_with("raw.githubusercontent.com") {
        bail!("Nur Links auf github.com gehen, z. B. https://github.com/anthropics/skills.");
    }
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 || !valid_part(parts[0]) || !valid_part(parts[1]) {
        bail!("Das ist kein GitHub-Repository. Beispiel: anthropics/skills");
    }
    let repo = parts[1].trim_end_matches(".git").to_string();
    let (mut branch, mut path) = (String::new(), String::new());
    if parts.len() >= 4 && (parts[2] == "tree" || parts[2] == "blob") {
        branch = parts[3].to_string();
        let mut p: Vec<&str> = parts[4..].to_vec();
        if parts[2] == "blob" && p.last().is_some_and(|l| l.contains('.')) {
            p.pop(); // Datei → ihr Ordner
        }
        if p.iter().any(|x| *x == ".." || *x == ".") {
            bail!("Ungültiger Pfad im Link.");
        }
        path = p.join("/");
    } else if parts.len() > 2 {
        bail!("Diesen Link verstehe ich nicht. Nimm den Link auf das Repository oder auf einen Ordner (…/tree/main/…).");
    }
    if !branch.is_empty() && !crate::github::valid_branch(&branch) {
        bail!("Ungültiger Branch im Link.");
    }
    Ok(Source { owner: parts[0].to_string(), repo, branch, path })
}

/// Eintrag eines Katalogs
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    /// «skill» oder «mod»
    pub kind: String,
    /// Ordner im Repository
    pub path: String,
    pub name: String,
    pub description: String,
}

/// Ordner mit SKILL.md oder mod.json aus der Antwort von git/trees, unter `under` (leer: alle)
pub fn entries_from_tree(tree: &Value, under: &str) -> Vec<(String, String)> {
    let prefix = if under.is_empty() { String::new() } else { format!("{}/", under.trim_end_matches('/')) };
    let mut out: Vec<(String, String)> = tree.get("tree").and_then(Value::as_array).cloned().unwrap_or_default().iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("blob"))
        .filter_map(|e| e.get("path").and_then(Value::as_str).map(str::to_string))
        .filter(|p| p.starts_with(&prefix) || format!("{p}/") == prefix)
        .filter_map(|p| {
            let (dir, file) = p.rsplit_once('/').unwrap_or(("", p.as_str()));
            let kind = match file { "SKILL.md" => "skill", "mod.json" => "mod", _ => return None };
            // Vorlagen und Beispiele für Autoren nicht anbieten
            if dir.split('/').any(|d| d.starts_with('.') || d == "template" || d == "template-skill") {
                return None;
            }
            Some((kind.to_string(), dir.to_string()))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Dateien eines Ordners aus der Antwort von git/trees (relativ zum Ordner)
pub fn files_under(tree: &Value, dir: &str) -> Vec<(String, u64)> {
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    tree.get("tree").and_then(Value::as_array).cloned().unwrap_or_default().iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("blob"))
        .filter_map(|e| {
            let p = e.get("path").and_then(Value::as_str)?;
            let rel = p.strip_prefix(&prefix)?;
            Some((rel.to_string(), e.get("size").and_then(Value::as_u64).unwrap_or(0)))
        })
        .collect()
}

struct Cached {
    at: Instant,
    branch: String,
    tree: Value,
    entries: Vec<Entry>,
}

pub struct Catalog {
    client: reqwest::Client,
    cache: Mutex<HashMap<String, Cached>>,
}

impl Catalog {
    pub fn new(client: reqwest::Client) -> Self {
        Catalog { client, cache: Mutex::new(HashMap::new()) }
    }

    async fn get_json(&self, url: &str) -> Result<Value> {
        let r = self.client.get(url).header("User-Agent", AGENT).header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(20)).send().await
            .map_err(|_| anyhow!("GitHub ist nicht erreichbar. Prüfe die Internetverbindung."))?;
        match r.status().as_u16() {
            200 => Ok(r.json().await?),
            404 => bail!("Das Repository gibt es nicht oder es ist nicht öffentlich."),
            403 | 429 => bail!("GitHub lässt gerade keine weiteren Abfragen zu. Versuch es in einer Stunde wieder."),
            s => bail!("GitHub antwortet mit Fehler {s}."),
        }
    }

    pub async fn raw(&self, src: &Source, branch: &str, path: &str, max: usize) -> Result<Vec<u8>> {
        let enc = |p: &str| p.split('/').map(crate::github::encode_uri_component).collect::<Vec<_>>().join("/");
        let url = format!("{RAW}/{}/{}/{}/{}", src.owner, src.repo, enc(branch), enc(path));
        let r = self.client.get(&url).header("User-Agent", AGENT).timeout(Duration::from_secs(30)).send().await
            .map_err(|_| anyhow!("GitHub ist nicht erreichbar."))?;
        if !r.status().is_success() {
            bail!("«{path}» liess sich nicht laden ({}).", r.status().as_u16());
        }
        let bytes = r.bytes().await?;
        if bytes.len() > max {
            bail!("«{path}» ist zu gross.");
        }
        Ok(bytes.to_vec())
    }

    /// Branch und Dateiliste (10 Minuten zwischengespeichert)
    async fn tree(&self, src: &Source) -> Result<(String, Value)> {
        let key = format!("{}@{}", src.repo_name().to_lowercase(), src.branch);
        if let Some(c) = self.cache.lock().get(&key).filter(|c| c.at.elapsed() < CACHE) {
            return Ok((c.branch.clone(), c.tree.clone()));
        }
        let branch = if src.branch.is_empty() {
            let info = self.get_json(&format!("{API}/repos/{}/{}", src.owner, src.repo)).await?;
            info.get("default_branch").and_then(Value::as_str).unwrap_or("main").to_string()
        } else {
            src.branch.clone()
        };
        let tree = self.get_json(&format!("{API}/repos/{}/{}/git/trees/{}?recursive=1", src.owner, src.repo, crate::github::encode_uri_component(&branch))).await?;
        self.cache.lock().insert(key, Cached { at: Instant::now(), branch: branch.clone(), tree: tree.clone(), entries: Vec::new() });
        Ok((branch, tree))
    }

    /// Skills und Mods eines Katalogs mit Name und Beschreibung
    pub async fn list(&self, src: &Source) -> Result<Vec<Entry>> {
        let key = format!("{}@{}", src.repo_name().to_lowercase(), src.branch);
        if let Some(c) = self.cache.lock().get(&key).filter(|c| c.at.elapsed() < CACHE && !c.entries.is_empty()) {
            return Ok(c.entries.clone());
        }
        let (branch, tree) = self.tree(src).await?;
        let found: Vec<(String, String)> = entries_from_tree(&tree, &src.path).into_iter().take(MAX_ENTRIES).collect();
        let loads = found.iter().map(|(kind, dir)| {
            let file = if dir.is_empty() { (if kind == "skill" { "SKILL.md" } else { "mod.json" }).to_string() } else { format!("{dir}/{}", if kind == "skill" { "SKILL.md" } else { "mod.json" }) };
            let branch = branch.clone();
            async move {
                let data = self.raw(src, &branch, &file, 200 * 1024).await.ok()?;
                let text = String::from_utf8_lossy(&data).to_string();
                let (name, description) = if kind == "skill" {
                    let p = crate::skills::parse(&text).ok()?;
                    (crate::skills::slugify(&p.name), p.description)
                } else {
                    let m = crate::mods::parse_manifest(&text).ok()?;
                    (m.name, m.beschreibung)
                };
                Some(Entry { kind: kind.clone(), path: dir.clone(), name, description })
            }
        });
        let entries: Vec<Entry> = futures::future::join_all(loads).await.into_iter().flatten().collect();
        if let Some(c) = self.cache.lock().get_mut(&key) {
            c.entries = entries.clone();
        }
        Ok(entries)
    }

    /// Dateien eines Eintrags: Skill nur SKILL.md, Mod alle Dateien des Ordners (geprüft wie beim Speichern)
    pub async fn files(&self, src: &Source, kind: &str, dir: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let (branch, tree) = self.tree(src).await?;
        let join = |f: &str| if dir.is_empty() { f.to_string() } else { format!("{dir}/{f}") };
        if kind == "skill" {
            return Ok(vec![("SKILL.md".into(), self.raw(src, &branch, &join("SKILL.md"), crate::skills::MAX_FILE).await?)]);
        }
        let list = files_under(&tree, dir);
        if list.len() > crate::mods::MAX_FILES {
            bail!("Der Mod hat zu viele Dateien.");
        }
        if list.iter().map(|(_, s)| *s as usize).sum::<usize>() > crate::mods::MAX_TOTAL {
            bail!("Der Mod ist zu gross.");
        }
        let mut out = Vec::new();
        for (rel, _) in list {
            crate::mods::check_path(&rel)?;
            out.push((rel.clone(), self.raw(src, &branch, &join(&rel), crate::mods::MAX_TOTAL).await?));
        }
        Ok(out)
    }

    /// Was liegt unter einem Link? Ein Skill, ein Mod oder ein ganzer Katalog
    pub async fn resolve(&self, src: &Source) -> Result<Vec<(String, String)>> {
        let (_, tree) = self.tree(src).await?;
        let exact: Vec<(String, String)> = entries_from_tree(&tree, &src.path).into_iter().filter(|(_, d)| *d == src.path).collect();
        if !exact.is_empty() {
            return Ok(exact);
        }
        Ok(entries_from_tree(&tree, &src.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn links_und_kurzformen() {
        let s = parse_source("anthropics/skills").unwrap();
        assert_eq!((s.owner.as_str(), s.repo.as_str(), s.branch.as_str(), s.path.as_str()), ("anthropics", "skills", "", ""));
        let s = parse_source("https://github.com/anthropics/skills/tree/main/skills/frontend-design").unwrap();
        assert_eq!((s.branch.as_str(), s.path.as_str()), ("main", "skills/frontend-design"));
        let s = parse_source("github.com/a/b/blob/dev/x/SKILL.md").unwrap();
        assert_eq!((s.branch.as_str(), s.path.as_str()), ("dev", "x"));
        assert_eq!(parse_source("https://github.com/a/b.git/").unwrap().repo, "b");
        for bad in ["", "abc", "https://example.com/a/b", "a/b/c", "https://github.com/a/b/tree/main/../x", "a b/c"] {
            assert!(parse_source(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn eintraege_aus_der_dateiliste() {
        let tree = json!({ "tree": [
            { "path": "skills/pdf/SKILL.md", "type": "blob", "size": 10 },
            { "path": "skills/pdf/scripts/a.py", "type": "blob", "size": 5 },
            { "path": "template/SKILL.md", "type": "blob" },
            { "path": "mods/robo/mod.json", "type": "blob", "size": 3 },
            { "path": "mods/robo/mod.js", "type": "blob", "size": 4 },
            { "path": "mods", "type": "tree" },
            { "path": "README.md", "type": "blob" },
        ] });
        assert_eq!(entries_from_tree(&tree, ""), vec![("mod".to_string(), "mods/robo".to_string()), ("skill".to_string(), "skills/pdf".to_string())]);
        assert_eq!(entries_from_tree(&tree, "skills"), vec![("skill".to_string(), "skills/pdf".to_string())]);
        assert_eq!(entries_from_tree(&tree, "skills/pdf"), vec![("skill".to_string(), "skills/pdf".to_string())]);
        assert_eq!(files_under(&tree, "mods/robo"), vec![("mod.json".to_string(), 3), ("mod.js".to_string(), 4)]);
    }
}
