//! Aufrufe der Oberfläche für Mods und Kataloge (ENTSCHEIDE E71, E72): `mods:*`, `catalog:*`.
//! Die Bestätigung von Windows bei laufendem Mod-Code macht `App::handle` vorher (`guard_text`).

use crate::app::App;
use crate::catalog::{parse_source, Source};
use crate::host::ConfirmOptions;
use crate::mods::Mods;
use crate::skills::{SaveOptions, SkillInput, Skills};
use crate::{settings, util::{js_string, truthy}};
use serde_json::{json, Value};
use std::sync::Arc;

/// Kataloge, die immer da sind (app-config.json «kataloge»), sonst diese
const DEFAULT_CATALOGS: &[&str] = &["KIYeager/ASK-Erweiterungen", "anthropics/skills"];

fn fail(reason: impl Into<String>) -> Value {
    json!({ "ok": false, "reason": reason.into() })
}

fn s(a: &[Value], i: usize) -> String {
    a.get(i).map(js_string).unwrap_or_default()
}

/// Mitgelieferte und eigene Kataloge: [{ repo, eigen }]
pub fn catalogs() -> Vec<Value> {
    let fixed: Vec<String> = settings::app_config()["kataloge"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_else(|| DEFAULT_CATALOGS.iter().map(|x| x.to_string()).collect());
    let own: Vec<String> = settings::get("kataloge").as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let mut out: Vec<Value> = fixed.iter().map(|r| json!({ "repo": r, "eigen": false })).collect();
    for r in own {
        if !fixed.iter().any(|f| f.eq_ignore_ascii_case(&r)) {
            out.push(json!({ "repo": r, "eigen": true }));
        }
    }
    out
}

impl App {
    fn changed_mods(&self) {
        self.send_ui("mods:changed", json!({}));
        self.send_ui("memory:changed", json!({})); // Skills-Liste neu laden
        self.push_state();
    }

    /// Skill speichern: fragt vor dem Ersetzen eines eigenen gleichnamigen
    async fn install_skill(&self, data: &[u8], origin: &str) -> Result<Value, String> {
        let parsed = Skills::read_import(data).map_err(|e| e.to_string())?;
        if self.knowledge.skills.own(&parsed.name).is_some() {
            let ok = self.confirm(&format!("Skill «{}» ersetzen?", parsed.name), "Es gibt schon einen eigenen Skill mit diesem Namen.", ConfirmOptions::default()).await.ok;
            if !ok {
                return Ok(json!({ "ok": false, "canceled": true }));
            }
        }
        let body = format!("{}\n\n<!-- Quelle: {origin} -->", parsed.body);
        let skill = self.knowledge.skills.save(&SkillInput { name: parsed.name, description: parsed.description, body }, &SaveOptions { previous: String::new(), overwrite: true })
            .map_err(|e| e.to_string())?;
        self.changed_mods();
        Ok(json!({ "ok": true, "kind": "skill", "skill": skill }))
    }

    /// Mod speichern (aus): fragt vor dem Ersetzen
    async fn install_mod(&self, files: Vec<(String, Vec<u8>)>) -> Result<Value, String> {
        let m = Mods::check_files(&files).map_err(|e| e.to_string())?;
        if self.knowledge.mods.info(&m.id).is_some() {
            let ok = self.confirm(&format!("Mod «{}» ersetzen?", m.name), "Es gibt schon einen Mod mit dieser Kennung. Er wird ersetzt und ist danach aus.", ConfirmOptions::default()).await.ok;
            if !ok {
                return Ok(json!({ "ok": false, "canceled": true }));
            }
        }
        let info = self.knowledge.mods.save(&files, true).map_err(|e| e.to_string())?;
        self.changed_mods();
        Ok(json!({ "ok": true, "kind": "mod", "mod": info }))
    }

    async fn install_entry(&self, src: &Source, kind: &str, path: &str) -> Value {
        let files = match self.catalog.files(src, kind, path).await {
            Ok(f) => f,
            Err(e) => return fail(e.to_string()),
        };
        let origin = format!("https://github.com/{}{}", src.repo_name(), if path.is_empty() { String::new() } else { format!("/tree/HEAD/{path}") });
        let r = if kind == "skill" { self.install_skill(&files[0].1, &origin).await } else { self.install_mod(files).await };
        r.unwrap_or_else(fail)
    }

    /// `Some(Ergebnis)` für die Kanäle dieses Moduls, sonst `None`
    pub(crate) async fn handle_extensions(self: &Arc<Self>, channel: &str, a: &[Value]) -> Option<Result<Value, String>> {
        let mods = self.knowledge.mods.clone();
        Some(Ok(match channel {
            "mods:list" => json!({ "mods": mods.list(), "examples": mods.examples(), "code": mods.code_active(), "folder": mods.dir().to_string_lossy() }),
            "mods:get" => mods.get(&s(a, 0)).unwrap_or(Value::Null),
            // Für die Oberfläche beim Start: aktive Mods mit Dateien zum Laden
            "mods:active" => {
                let active = mods.active();
                if active.iter().any(|m| !m.manifest.script.is_empty()) {
                    self.mark_ui_mod_code();
                }
                json!(active.iter().map(|m| json!({ "id": m.manifest.id, "name": m.manifest.name, "hash": m.hash,
                "style": m.manifest.style, "script": m.manifest.script, "befehle": m.manifest.befehle })).collect::<Vec<_>>())
            }
            "mods:enable" => {
                let r = mods.enable(&s(a, 0), truthy(&a.get(1).cloned().unwrap_or(Value::Null)));
                self.changed_mods();
                match r { Ok(ok) => json!({ "ok": ok }), Err(e) => fail(e.to_string()) }
            }
            "mods:off-all" => {
                let n = mods.disable_all().unwrap_or(0);
                self.changed_mods();
                json!({ "ok": true, "count": n })
            }
            "mods:remove" => {
                let Some(info) = mods.info(&s(a, 0)) else { return Some(Ok(json!(false))) };
                if !self.confirm(&format!("Mod «{}» löschen?", info.manifest.name), "Der Mod wird von diesem Laptop entfernt.", ConfirmOptions { danger: true, ..Default::default() }).await.ok {
                    return Some(Ok(json!(false)));
                }
                let r = mods.remove(&info.manifest.id);
                self.changed_mods();
                json!(r.unwrap_or(false))
            }
            "mods:import" => {
                let Some(folder) = self.host_pick_folder("Mod-Ordner wählen (mit mod.json)").await else { return Some(Ok(json!({ "canceled": true }))) };
                match Mods::read_folder(std::path::Path::new(&folder)) {
                    Ok(files) => self.install_mod(files).await.unwrap_or_else(fail),
                    Err(e) => fail(e.to_string()),
                }
            }
            "mods:example" => match mods.example_files(&s(a, 0)) {
                Ok(files) => self.install_mod(files).await.unwrap_or_else(fail),
                Err(e) => fail(e.to_string()),
            },
            "mods:folder" => {
                let _ = std::fs::create_dir_all(mods.dir());
                self.host_open(&mods.dir().to_string_lossy()).await;
                Value::Null
            }
            // Eigene Kataloge setzen (Liste) oder nur lesen
            "catalog:sources" => {
                if let Some(list) = a.first().and_then(Value::as_array) {
                    let mut own: Vec<String> = Vec::new();
                    for r in list.iter().map(js_string) {
                        match parse_source(&r) {
                            Ok(src) if src.path.is_empty() => { if !own.iter().any(|o| o.eq_ignore_ascii_case(&src.repo_name())) { own.push(src.repo_name()); } }
                            Ok(_) => return Some(Ok(fail("Ein Katalog ist ein ganzes Repository, z. B. anthropics/skills."))),
                            Err(e) => return Some(Ok(fail(e.to_string()))),
                        }
                    }
                    settings::set(json!({ "kataloge": own.into_iter().take(20).collect::<Vec<_>>() }));
                }
                json!({ "ok": true, "catalogs": catalogs() })
            }
            "catalog:list" => match parse_source(&s(a, 0)) {
                Ok(src) => match self.catalog.list(&src).await {
                    Ok(entries) => {
                        let own: Vec<String> = self.knowledge.skills.list_own().into_iter().map(|x| x.slug).collect();
                        let ids: Vec<String> = mods.list().into_iter().map(|m| m.manifest.name).collect();
                        let entries: Vec<Value> = entries.into_iter().map(|e| {
                            let installed = if e.kind == "skill" { own.contains(&e.name) } else { ids.contains(&e.name) };
                            let mut v = json!(e);
                            v["installiert"] = json!(installed);
                            v
                        }).collect();
                        json!({ "ok": true, "entries": entries })
                    }
                    Err(e) => fail(e.to_string()),
                },
                Err(e) => fail(e.to_string()),
            },
            "catalog:install" => match parse_source(&s(a, 0)) {
                Ok(src) => {
                    let kind = s(a, 1);
                    if kind != "skill" && kind != "mod" {
                        return Some(Ok(fail("Unbekannte Art.")));
                    }
                    self.install_entry(&src, &kind, &s(a, 2)).await
                }
                Err(e) => fail(e.to_string()),
            },
            // Link auf einen Skill, einen Mod oder einen ganzen Katalog
            "catalog:link" => match parse_source(&s(a, 0)) {
                Ok(src) => match self.catalog.resolve(&src).await {
                    Ok(found) if found.is_empty() => fail("Unter diesem Link liegt kein Skill (SKILL.md) und kein Mod (mod.json)."),
                    Ok(found) if found.len() == 1 => self.install_entry(&src, &found[0].0, &found[0].1).await,
                    Ok(found) => json!({ "ok": true, "choose": found.iter().map(|(k, p)| json!({ "kind": k, "path": p })).collect::<Vec<_>>(), "repo": src.repo_name() }),
                    Err(e) => fail(e.to_string()),
                },
                Err(e) => fail(e.to_string()),
            },
            _ => return None,
        }))
    }
}
