//! ASK Desktop: alles ohne Fenster. Jedes Modul entspricht einer Datei aus der früheren
//! Electron-App (src/main/<name>.js); die Begründungen stehen in docs/ENTSCHEIDE.md.
//! Das Fenster, die Webansichten und Windows-Dialoge liefert die Tauri-Hülle (src-tauri/src)
//! über die Schnittstelle in host.rs.

pub mod util;
pub mod host;
pub mod build_dir;
pub mod paths;
pub mod settings;
pub mod safe;
pub mod secrets;
pub mod doh;

// Welle 1: Module ohne Abhängigkeiten untereinander
pub mod approvals;
pub mod folders;
pub mod cleanup;
pub mod lowlabel;
pub mod portable;
pub mod python;
pub mod ooxml;
pub mod office;
pub mod attachments;
pub mod snapshot;
pub mod askapi;
pub mod review;
pub mod usage;
pub mod web;
pub mod memory;
pub mod skills;
pub mod history;
pub mod memorytools;
pub mod workspace;
pub mod github;
pub mod servers;
pub mod terminal;
pub mod updater;

// Welle 2
pub mod tools;
pub mod bild;
pub mod helpers;
pub mod knowledge;
pub mod visual;

// Welle 3
pub mod agent;
pub mod app;
