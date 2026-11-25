// Frage 4: Terminal mit niedriger Integritätsstufe in einem Job-Objekt.
//
// Heute macht das nicht Node, sondern der Python-Starter (python/ask_terminal/launcher.py, winproc.py):
// Node startet nur «python -E -s -B -X utf8 launcher.py», schreibt die Einstellungen als JSON auf stdin und
// liest JSON-Zeilen von stdout (src/main/backend/native-low.js). Genau das kann Rust mit std/tokio::process
// ebenso; start_launcher() zeigt das. Der Sidecar in main.rs nutzt dasselbe Muster und läuft im Spike.
//
// Zum Vergleich low_job_spawn(): dieselben Schritte wie winproc.py direkt in Rust (Token duplizieren, auf Low
// senken, angehalten starten, in ein Job-Objekt mit KILL_ON_JOB_CLOSE, fortsetzen). Nur für Windows,
// nur kompiliert (cargo check --target x86_64-pc-windows-msvc), nicht ausgeführt.

#![allow(dead_code)]

use std::path::Path;

/// Startet den bestehenden Python-Starter unverändert, wie native-low.js.
pub fn start_launcher(python: &Path, launcher: &Path, cfg_json: &str) -> std::io::Result<std::process::Child> {
  use std::io::Write;
  let mut cmd = std::process::Command::new(python);
  cmd.args(["-E", "-s", "-B", "-X", "utf8"]).arg(launcher);
  cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW, wie windowsHide
  }
  let mut child = cmd.spawn()?;
  child.stdin.as_mut().unwrap().write_all(format!("{cfg_json}\n").as_bytes())?;
  Ok(child)
}

pub fn describe() -> &'static str {
  "Python-Starter unverändert (start_launcher) oder Low+Job direkt in Rust (low_job_spawn, nur Windows)"
}

#[cfg(windows)]
pub fn low_job_spawn(command_line: &str) -> windows::core::Result<(windows::Win32::Foundation::HANDLE, u32)> {
  use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
      Foundation::{CloseHandle, HANDLE},
      Security::{
        Authorization::ConvertStringSidToSidW, DuplicateTokenEx, GetLengthSid, SecurityImpersonation, SetTokenInformation,
        TokenIntegrityLevel, TokenPrimary, PSID, SID_AND_ATTRIBUTES, TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT,
        TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
      },
      System::{
        JobObjects::{
          AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
          JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
          CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED,
          PROCESS_INFORMATION, STARTUPINFOW,
        },
      },
    },
  };
  unsafe {
    // Job mit «Kill on Job Close»: endet die App, endet alles darin
    let job = CreateJobObjectW(None, PCWSTR::null())?;
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    SetInformationJobObject(
      job,
      JobObjectExtendedLimitInformation,
      &info as *const _ as *const core::ffi::c_void,
      std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
    )?;

    // Eigenes Token duplizieren und auf Low (S-1-16-4096) senken
    let mut token = HANDLE::default();
    OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT, &mut token)?;
    let mut low = HANDLE::default();
    DuplicateTokenEx(token, TOKEN_ACCESS_MASK(0x02000000), None, SecurityImpersonation, TokenPrimary, &mut low)?; // MAXIMUM_ALLOWED
    let mut sid = PSID::default();
    let sid_str: Vec<u16> = "S-1-16-4096\0".encode_utf16().collect();
    ConvertStringSidToSidW(PCWSTR(sid_str.as_ptr()), &mut sid)?;
    let label = TOKEN_MANDATORY_LABEL { Label: SID_AND_ATTRIBUTES { Sid: sid, Attributes: 0x20 } }; // SE_GROUP_INTEGRITY
    SetTokenInformation(
      low,
      TokenIntegrityLevel,
      &label as *const _ as *const core::ffi::c_void,
      std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid),
    )?;

    // Angehalten starten, in den Job, dann fortsetzen (kein Fenster). stdin/stdout-Pipes fehlen hier noch:
    // dafür bräuchte es STARTF_USESTDHANDLES mit eigenen CreatePipe-Handles.
    let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut pi = PROCESS_INFORMATION::default();
    let mut cmd: Vec<u16> = command_line.encode_utf16().chain(std::iter::once(0)).collect();
    CreateProcessAsUserW(
      Some(low),
      PCWSTR::null(),
      Some(PWSTR(cmd.as_mut_ptr())),
      None,
      None,
      false,
      CREATE_SUSPENDED | CREATE_NO_WINDOW,
      None,
      PCWSTR::null(),
      &si,
      &mut pi,
    )?;
    AssignProcessToJobObject(job, pi.hProcess)?;
    ResumeThread(pi.hThread);
    let _ = CloseHandle(pi.hThread);
    let _ = CloseHandle(pi.hProcess);
    let _ = CloseHandle(low);
    let _ = CloseHandle(token);
    Ok((job, pi.dwProcessId))
  }
}
