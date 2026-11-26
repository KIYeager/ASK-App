// Noch nicht portiert (src/main/python.js). Platzhalter mit der Schnittstelle, die paths.rs braucht.
use std::path::PathBuf;
pub struct PythonRuntime { root: PathBuf }
impl PythonRuntime { pub fn exe(&self) -> PathBuf { self.root.join("python.exe") } }
pub fn shared() -> &'static PythonRuntime {
    static R: std::sync::OnceLock<PythonRuntime> = std::sync::OnceLock::new();
    R.get_or_init(|| PythonRuntime { root: crate::paths::local_app_data().join("ASK-Desktop").join("python") })
}
