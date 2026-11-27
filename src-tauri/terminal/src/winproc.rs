//! Windows-Funktionen: Integritätsstufen, Job-Objekte, angehaltene Prozesse fortsetzen, Low-Kennzeichnung
//! von Ordnern (Port von python/ask_terminal/winproc.py).
//!
//! Alles funktioniert ohne Admin-Rechte: Ein Prozess darf das Token seiner eigenen Kinder absenken
//! (nie anheben), und Besitzer dürfen ihre Ordner mit «Low» kennzeichnen.
//!
//! Öffentliche Schnittstelle (nur Windows):
//! - `LOW_SID`, `CREATE_SUSPENDED`, `CREATE_NO_WINDOW`
//! - `lower_token_to_low(prozess)`: Primärtoken eines (angehaltenen) Kindprozesses auf Low
//! - `current_integrity_sid()`: Integritätsstufe des eigenen Prozesses
//! - `resume_process(pid)`: Threads eines mit `CREATE_SUSPENDED` gestarteten Prozesses fortsetzen
//! - `Job`: Job-Objekt mit «Kill on Job Close» (`new`, `assign`, `terminate`; beim Schliessen endet alles darin)
//! - `has_low_label(pfad)`, `ensure_low_label(pfad)`: vererbbares Low-Label `(OI)(CI)` auf einem Ordner

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, SetTokenInformation, ACL, LABEL_SECURITY_INFORMATION, SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

/// SID der Integritätsstufe «Low»
pub const LOW_SID: &str = "S-1-16-4096";
pub const CREATE_SUSPENDED: u32 = 0x0000_0004;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const SE_GROUP_INTEGRITY: u32 = 0x20;
const SYSTEM_MANDATORY_LABEL_ACE_TYPE: u8 = 0x11;
const OBJECT_INHERIT_ACE: u8 = 0x1;
const CONTAINER_INHERIT_ACE: u8 = 0x2;

fn last_error(what: &str) -> io::Error {
    let err = io::Error::last_os_error();
    io::Error::new(err.kind(), format!("{what}: {err}"))
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Setzt das Primärtoken eines (angehaltenen) Kindprozesses auf Low Integrity.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn lower_token_to_low(process: HANDLE) -> io::Result<()> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_ADJUST_DEFAULT | TOKEN_QUERY, &mut token) == 0 {
            return Err(last_error("OpenProcessToken"));
        }
        let mut sid: *mut c_void = std::ptr::null_mut();
        let result = (|| {
            if ConvertStringSidToSidW(wide(LOW_SID).as_ptr(), &mut sid) == 0 {
                return Err(last_error("ConvertStringSidToSid"));
            }
            let label = TOKEN_MANDATORY_LABEL { Label: SID_AND_ATTRIBUTES { Sid: sid, Attributes: SE_GROUP_INTEGRITY } };
            let size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid);
            if SetTokenInformation(token, TokenIntegrityLevel, &label as *const _ as *const c_void, size) == 0 {
                return Err(last_error("SetTokenInformation"));
            }
            Ok(())
        })();
        if !sid.is_null() {
            LocalFree(sid);
        }
        CloseHandle(token);
        result
    }
}

fn sid_to_string(sid: *mut c_void) -> io::Result<String> {
    unsafe {
        let mut out: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut out) == 0 {
            return Err(last_error("ConvertSidToStringSid"));
        }
        let mut len = 0;
        while *out.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(out, len));
        LocalFree(out as *mut c_void);
        Ok(text)
    }
}

/// Integritätsstufe des eigenen Prozesses als SID, z. B. S-1-16-4096 für Low.
pub fn current_integrity_sid() -> io::Result<String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(last_error("OpenProcessToken"));
        }
        let result = (|| {
            let mut size = 0u32;
            GetTokenInformation(token, TokenIntegrityLevel, std::ptr::null_mut(), 0, &mut size);
            if size == 0 {
                return Err(last_error("GetTokenInformation"));
            }
            // 8-Byte-ausgerichtet für den Zeiger in TOKEN_MANDATORY_LABEL
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            if GetTokenInformation(token, TokenIntegrityLevel, buf.as_mut_ptr() as *mut c_void, size, &mut size) == 0 {
                return Err(last_error("GetTokenInformation"));
            }
            let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            sid_to_string(label.Label.Sid)
        })();
        CloseHandle(token);
        result
    }
}

/// Setzt alle Threads eines mit `CREATE_SUSPENDED` gestarteten Prozesses fort.
pub fn resume_process(pid: u32) -> io::Result<()> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateToolhelp32Snapshot"));
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        let mut ok = Thread32First(snap, &mut entry);
        while ok != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    ResumeThread(thread);
                    CloseHandle(thread);
                    resumed = true;
                }
            }
            ok = Thread32Next(snap, &mut entry);
        }
        CloseHandle(snap);
        if resumed {
            Ok(())
        } else {
            Err(io::Error::other("Kein Thread zum Fortsetzen gefunden"))
        }
    }
}

/// Job-Objekt mit «Kill on Job Close»: Endet der Besitzer, endet alles im Job.
pub struct Job {
    handle: HANDLE,
}

// Das Handle gehört dem Job allein; die Aufrufe sind thread-sicher.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    pub fn new() -> io::Result<Job> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(last_error("CreateJobObject"));
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let err = last_error("SetInformationJobObject");
                CloseHandle(handle);
                return Err(err);
            }
            Ok(Job { handle })
        }
    }

    /// Legt einen Prozess (Handle) in den Job
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn assign(&self, process: HANDLE) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.handle, process) } == 0 {
            return Err(last_error("AssignProcessToJobObject"));
        }
        Ok(())
    }

    /// Anzahl der noch laufenden Prozesse im Job
    pub fn active_processes(&self) -> u32 {
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            let ok = QueryInformationJobObject(
                self.handle,
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                0
            } else {
                info.ActiveProcesses
            }
        }
    }

    /// Beendet alle Prozesse im Job sofort
    pub fn terminate(&self, code: u32) {
        unsafe {
            TerminateJobObject(self.handle, code);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

/// True, wenn der Ordner ein vererbbares Low-Label trägt (Objekte und Unterordner).
pub fn has_low_label(path: &Path) -> bool {
    unsafe {
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut sacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        let err = GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sacl,
            &mut descriptor,
        );
        if err != 0 {
            return false;
        }
        let mut found = false;
        if !sacl.is_null() {
            let count = (*sacl).AceCount;
            let mut offset = (sacl as *const u8).add(std::mem::size_of::<ACL>());
            for _ in 0..count {
                // ACE_HEADER: Typ (1 Byte), Flags (1 Byte), Grösse (2 Bytes)
                let (ace_type, flags, size) = (*offset, *offset.add(1), u16::from_le_bytes([*offset.add(2), *offset.add(3)]));
                if ace_type == SYSTEM_MANDATORY_LABEL_ACE_TYPE {
                    // SYSTEM_MANDATORY_LABEL_ACE: Header, Mask (DWORD), danach die SID
                    let sid = offset.add(4 + 4) as *mut c_void;
                    let inherit = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
                    if sid_to_string(sid).is_ok_and(|s| s == LOW_SID) && flags & inherit == inherit {
                        found = true;
                        break;
                    }
                }
                offset = offset.add(size as usize);
            }
        }
        LocalFree(descriptor);
        found
    }
}

/// Kennzeichnet einen Ordner mit Low, damit Low-Prozesse darin schreiben dürfen.
/// Wie in der Python-Fassung setzt `icacls` die Kennzeichnung; geprüft wird sie mit der Windows-API.
pub fn ensure_low_label(path: &Path) -> (bool, String) {
    use std::os::windows::process::CommandExt;
    if has_low_label(path) {
        return (true, "bereits gekennzeichnet".into());
    }
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let icacls = Path::new(&root).join("System32").join("icacls.exe");
    let result = std::process::Command::new(icacls).arg(path).args(["/setintegritylevel", "(OI)(CI)L"]).creation_flags(CREATE_NO_WINDOW).output();
    let output = match result {
        Ok(o) => o,
        Err(e) => return (false, e.to_string()),
    };
    if !output.status.success() {
        let oem = crate::streams::Oem::system();
        let out = crate::streams::decode_output(&output.stdout, &oem);
        let message = if out.trim().is_empty() { crate::streams::decode_output(&output.stderr, &oem) } else { out.trim().to_string() };
        return (false, message);
    }
    if !has_low_label(path) {
        return (false, "Kennzeichnung nach icacls nicht vorhanden".into());
    }
    (true, "gekennzeichnet".into())
}
