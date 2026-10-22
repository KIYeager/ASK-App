"""Windows-Funktionen: Integritätsstufen, Job-Objekte, angehaltene Prozesse fortsetzen.

Alles funktioniert ohne Admin-Rechte: Ein Prozess darf das Token seiner eigenen Kinder
absenken (nie anheben), und Besitzer dürfen ihre Ordner mit «Low» kennzeichnen.
"""
import ctypes
import ctypes.wintypes as wt
import os
import subprocess

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
adv = ctypes.WinDLL("advapi32", use_last_error=True)

LOW_SID = "S-1-16-4096"
SE_GROUP_INTEGRITY = 0x20
TOKEN_QUERY = 0x0008
TOKEN_ADJUST_DEFAULT = 0x0080
TOKEN_INTEGRITY_LEVEL = 25
JOB_EXTENDED_LIMIT_INFORMATION = 9
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000
TH32CS_SNAPTHREAD = 0x4
THREAD_SUSPEND_RESUME = 0x0002
SE_FILE_OBJECT = 1
LABEL_SECURITY_INFORMATION = 0x10
SYSTEM_MANDATORY_LABEL_ACE_TYPE = 0x11
OBJECT_INHERIT_ACE = 0x1
CONTAINER_INHERIT_ACE = 0x2
INVALID_HANDLE_VALUE = wt.HANDLE(-1).value

CREATE_SUSPENDED = 0x00000004
CREATE_NO_WINDOW = 0x08000000


class SID_AND_ATTRIBUTES(ctypes.Structure):
    _fields_ = [("Sid", ctypes.c_void_p), ("Attributes", wt.DWORD)]


class TOKEN_MANDATORY_LABEL(ctypes.Structure):
    _fields_ = [("Label", SID_AND_ATTRIBUTES)]


class IO_COUNTERS(ctypes.Structure):
    _fields_ = [(n, ctypes.c_ulonglong) for n in ("r", "w", "o", "rb", "wb", "ob")]


class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [("PerProcessUserTimeLimit", ctypes.c_longlong), ("PerJobUserTimeLimit", ctypes.c_longlong),
                ("LimitFlags", wt.DWORD), ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", wt.DWORD),
                ("Affinity", ctypes.c_size_t), ("PriorityClass", wt.DWORD), ("SchedulingClass", wt.DWORD)]


class JOBOBJECT_EXTENDED_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [("BasicLimitInformation", JOBOBJECT_BASIC_LIMIT_INFORMATION), ("IoInfo", IO_COUNTERS),
                ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t)]


class THREADENTRY32(ctypes.Structure):
    _fields_ = [("dwSize", wt.DWORD), ("cntUsage", wt.DWORD), ("th32ThreadID", wt.DWORD),
                ("th32OwnerProcessID", wt.DWORD), ("tpBasePri", wt.LONG), ("tpDeltaPri", wt.LONG),
                ("dwFlags", wt.DWORD)]


class ACL(ctypes.Structure):
    _fields_ = [("AclRevision", ctypes.c_ubyte), ("Sbz1", ctypes.c_ubyte), ("AclSize", wt.WORD),
                ("AceCount", wt.WORD), ("Sbz2", wt.WORD)]


class ACE_HEADER(ctypes.Structure):
    _fields_ = [("AceType", ctypes.c_ubyte), ("AceFlags", ctypes.c_ubyte), ("AceSize", wt.WORD)]


k32.GetCurrentProcess.restype = wt.HANDLE
k32.CreateJobObjectW.restype = wt.HANDLE
k32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wt.LPCWSTR]
k32.SetInformationJobObject.argtypes = [wt.HANDLE, ctypes.c_int, ctypes.c_void_p, wt.DWORD]
k32.AssignProcessToJobObject.argtypes = [wt.HANDLE, wt.HANDLE]
k32.TerminateJobObject.argtypes = [wt.HANDLE, wt.UINT]
k32.CloseHandle.argtypes = [wt.HANDLE]
k32.CreateToolhelp32Snapshot.restype = wt.HANDLE
k32.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
k32.Thread32First.argtypes = [wt.HANDLE, ctypes.POINTER(THREADENTRY32)]
k32.Thread32Next.argtypes = [wt.HANDLE, ctypes.POINTER(THREADENTRY32)]
k32.OpenThread.restype = wt.HANDLE
k32.OpenThread.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
k32.ResumeThread.argtypes = [wt.HANDLE]
k32.LocalFree.argtypes = [ctypes.c_void_p]
adv.OpenProcessToken.argtypes = [wt.HANDLE, wt.DWORD, ctypes.POINTER(wt.HANDLE)]
adv.SetTokenInformation.argtypes = [wt.HANDLE, ctypes.c_int, ctypes.c_void_p, wt.DWORD]
adv.GetTokenInformation.argtypes = [wt.HANDLE, ctypes.c_int, ctypes.c_void_p, wt.DWORD, ctypes.POINTER(wt.DWORD)]
adv.ConvertStringSidToSidW.argtypes = [wt.LPCWSTR, ctypes.POINTER(ctypes.c_void_p)]
adv.ConvertSidToStringSidW.argtypes = [ctypes.c_void_p, ctypes.POINTER(wt.LPWSTR)]
adv.GetLengthSid.argtypes = [ctypes.c_void_p]
adv.GetNamedSecurityInfoW.argtypes = [wt.LPCWSTR, ctypes.c_int, wt.DWORD, ctypes.c_void_p, ctypes.c_void_p,
                                      ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p),
                                      ctypes.POINTER(ctypes.c_void_p)]
adv.GetNamedSecurityInfoW.restype = wt.DWORD


def _check(ok, what):
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error(), what)


def _sid_to_str(sid) -> str:
    out = wt.LPWSTR()
    _check(adv.ConvertSidToStringSidW(sid, ctypes.byref(out)), "ConvertSidToStringSid")
    try:
        return out.value
    finally:
        k32.LocalFree(out)


def lower_token_to_low(process_handle: int) -> None:
    """Setzt das Primärtoken eines (angehaltenen) Kindprozesses auf Low Integrity."""
    token = wt.HANDLE()
    _check(adv.OpenProcessToken(process_handle, TOKEN_ADJUST_DEFAULT | TOKEN_QUERY, ctypes.byref(token)),
           "OpenProcessToken")
    sid = ctypes.c_void_p()
    try:
        _check(adv.ConvertStringSidToSidW(LOW_SID, ctypes.byref(sid)), "ConvertStringSidToSid")
        label = TOKEN_MANDATORY_LABEL(SID_AND_ATTRIBUTES(sid, SE_GROUP_INTEGRITY))
        size = ctypes.sizeof(label) + adv.GetLengthSid(sid)
        _check(adv.SetTokenInformation(token, TOKEN_INTEGRITY_LEVEL, ctypes.byref(label), size),
               "SetTokenInformation")
    finally:
        if sid:
            k32.LocalFree(sid)
        k32.CloseHandle(token)


def current_integrity_sid() -> str:
    """Integritätsstufe des eigenen Prozesses als SID, z. B. S-1-16-4096 für Low."""
    token = wt.HANDLE()
    _check(adv.OpenProcessToken(k32.GetCurrentProcess(), TOKEN_QUERY, ctypes.byref(token)), "OpenProcessToken")
    try:
        size = wt.DWORD()
        adv.GetTokenInformation(token, TOKEN_INTEGRITY_LEVEL, None, 0, ctypes.byref(size))
        buf = ctypes.create_string_buffer(size.value)
        _check(adv.GetTokenInformation(token, TOKEN_INTEGRITY_LEVEL, buf, size, ctypes.byref(size)),
               "GetTokenInformation")
        return _sid_to_str(TOKEN_MANDATORY_LABEL.from_buffer(buf).Label.Sid)
    finally:
        k32.CloseHandle(token)


def resume_process(pid: int) -> None:
    """Setzt alle Threads eines mit CREATE_SUSPENDED gestarteten Prozesses fort."""
    snap = k32.CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
    if snap == INVALID_HANDLE_VALUE:
        raise ctypes.WinError(ctypes.get_last_error(), "CreateToolhelp32Snapshot")
    try:
        entry = THREADENTRY32()
        entry.dwSize = ctypes.sizeof(entry)
        ok = k32.Thread32First(snap, ctypes.byref(entry))
        while ok:
            if entry.th32OwnerProcessID == pid:
                th = k32.OpenThread(THREAD_SUSPEND_RESUME, False, entry.th32ThreadID)
                if th:
                    k32.ResumeThread(th)
                    k32.CloseHandle(th)
            ok = k32.Thread32Next(snap, ctypes.byref(entry))
    finally:
        k32.CloseHandle(snap)


class Job:
    """Job-Objekt mit «Kill on Job Close»: Endet dieser Prozess, endet alles im Job."""

    def __init__(self):
        self.handle = k32.CreateJobObjectW(None, None)
        _check(self.handle, "CreateJobObject")
        info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        _check(k32.SetInformationJobObject(self.handle, JOB_EXTENDED_LIMIT_INFORMATION, ctypes.byref(info),
                                           ctypes.sizeof(info)), "SetInformationJobObject")

    def assign(self, process_handle: int) -> None:
        _check(k32.AssignProcessToJobObject(self.handle, process_handle), "AssignProcessToJobObject")

    def terminate(self, code: int = 1) -> None:
        k32.TerminateJobObject(self.handle, code)


def has_low_label(path: str) -> bool:
    """True, wenn der Ordner ein vererbbares Low-Label trägt (Objekte und Unterordner)."""
    sacl = ctypes.c_void_p()
    descriptor = ctypes.c_void_p()
    err = adv.GetNamedSecurityInfoW(path, SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, None, None, None,
                                    ctypes.byref(sacl), ctypes.byref(descriptor))
    if err != 0:
        return False
    try:
        if not sacl.value:
            return False
        acl = ACL.from_address(sacl.value)
        offset = sacl.value + ctypes.sizeof(ACL)
        for _ in range(acl.AceCount):
            header = ACE_HEADER.from_address(offset)
            if header.AceType == SYSTEM_MANDATORY_LABEL_ACE_TYPE:
                # SYSTEM_MANDATORY_LABEL_ACE: Header, Mask (DWORD), danach die SID.
                sid = offset + ctypes.sizeof(ACE_HEADER) + 4
                inherit = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
                if _sid_to_str(ctypes.c_void_p(sid)) == LOW_SID and header.AceFlags & inherit == inherit:
                    return True
            offset += header.AceSize
        return False
    finally:
        k32.LocalFree(descriptor)


def ensure_low_label(path: str) -> tuple[bool, str]:
    """Kennzeichnet einen Ordner mit Low, damit Low-Prozesse darin schreiben dürfen."""
    if has_low_label(path):
        return True, "bereits gekennzeichnet"
    icacls = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "icacls.exe")
    result = subprocess.run([icacls, path, "/setintegritylevel", "(OI)(CI)L"], capture_output=True,
                            creationflags=CREATE_NO_WINDOW)
    if result.returncode != 0:
        message = result.stdout.decode("cp850", "replace").strip() or result.stderr.decode("cp850", "replace")
        return False, message
    if not has_low_label(path):
        return False, "Kennzeichnung nach icacls nicht vorhanden"
    return True, "gekennzeichnet"
