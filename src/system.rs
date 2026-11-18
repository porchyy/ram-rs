use std::{
    io,
    mem::{size_of, zeroed},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{ProcessStatus::*, SystemInformation::*, Threading::*},
};

pub const MIB: f64 = 1_048_576.0;

#[derive(Clone, Copy, Debug)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub commit: u64,
    pub limit: u64,
}

impl Memory {
    pub fn used_percent(self) -> f64 {
        (self.total - self.available) as f64 * 100.0 / self.total as f64
    }
}

pub fn memory() -> io::Result<Memory> {
    // SAFETY: API output buffers are initialized and correctly sized.
    unsafe {
        let mut ram: MEMORYSTATUSEX = zeroed();
        ram.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        let mut perf: PERFORMANCE_INFORMATION = zeroed();
        perf.cb = size_of::<PERFORMANCE_INFORMATION>() as u32;
        if GlobalMemoryStatusEx(&mut ram) == 0 || K32GetPerformanceInfo(&mut perf, perf.cb) == 0 {
            return Err(io::Error::last_os_error());
        }
        if ram.ullTotalPhys == 0 || ram.ullAvailPhys > ram.ullTotalPhys || perf.PageSize == 0 {
            return Err(io::Error::other("Invalid memory sample"));
        }
        Ok(Memory {
            total: ram.ullTotalPhys,
            available: ram.ullAvailPhys,
            commit: perf.CommitTotal as u64 * perf.PageSize as u64,
            limit: perf.CommitLimit as u64 * perf.PageSize as u64,
        })
    }
}

pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns exactly one successfully opened handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn open(pid: u32, rights: u32) -> io::Result<Handle> {
    // SAFETY: no borrowed pointers; a null result is never wrapped.
    let handle = unsafe { OpenProcess(rights, 0, pid) };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(handle))
    }
}

pub fn counters(handle: HANDLE) -> io::Result<PROCESS_MEMORY_COUNTERS> {
    // SAFETY: handle is only used during this call, output matches the API layout.
    unsafe {
        let mut data: PROCESS_MEMORY_COUNTERS = zeroed();
        data.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(
            handle,
            &mut data,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ) == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(data)
        }
    }
}

pub struct Process {
    pub pid: u32,
    pub name: String,
    pub working_set: usize,
}

pub fn image_path(handle: HANDLE) -> io::Result<String> {
    let mut path = vec![0u16; 32768];
    let mut len = path.len() as u32;
    // SAFETY: path is a writable UTF-16 buffer; the caller retains the handle.
    if unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf16_lossy(&path[..len as usize]))
}

pub fn processes() -> io::Result<(Vec<Process>, usize)> {
    let mut ids = vec![0u32; 1024];
    loop {
        let mut bytes = 0;
        // SAFETY: writable vector buffer with its actual capacity in bytes.
        if unsafe {
            K32EnumProcesses(
                ids.as_mut_ptr(),
                (ids.len() * size_of::<u32>()) as u32,
                &mut bytes,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if (bytes as usize) < ids.len() * size_of::<u32>() {
            ids.truncate(bytes as usize / size_of::<u32>());
            break;
        }
        if ids.len() >= 65536 {
            return Err(io::Error::other("Process enumeration exceeded bound"));
        }
        ids.resize(ids.len() * 2, 0);
    }
    let mut rows = Vec::new();
    let mut skipped = 0;
    for pid in ids {
        let row: io::Result<Process> = (|| {
            let handle = open(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)?;
            let usage = counters(handle.0)?;
            let path = image_path(handle.0)?;
            let name = path.rsplit('\\').next().unwrap_or(&path).to_owned();
            Ok(Process {
                pid,
                name,
                working_set: usage.WorkingSetSize,
            })
        })();
        match row {
            Ok(row) => rows.push(row),
            Err(_) => skipped += 1,
        }
    }
    rows.sort_unstable_by_key(|row| std::cmp::Reverse(row.working_set));
    Ok((rows, skipped))
}
