// v0.0.1 - Recover abandoned or retained unowned supervisor locks without allowing a second owner.
use std::ffi::c_void;
use std::io;
use std::marker::PhantomData;
use std::rc::Rc;

type Handle = *mut c_void;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_ABANDONED: u32 = 0x80;
const WAIT_TIMEOUT: u32 = 0x102;
const WAIT_FAILED: u32 = u32::MAX;
const SYNCHRONIZE: u32 = 0x0010_0000;
const MUTEX_MODIFY_STATE: u32 = 1;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(attributes: *mut c_void, owner: i32, name: *const u16) -> Handle;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn ReleaseMutex(handle: Handle) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

struct KernelHandle(Handle);
impl Drop for KernelHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn wide_name(name: &str) -> io::Result<Vec<u16>> {
    if name.is_empty() || name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid mutex name",
        ));
    }
    Ok(name.encode_utf16().chain(Some(0)).collect())
}

/// Ownership is bound to the acquiring thread, including when it is abandoned.
/// The guard must be dropped on that same thread before its handle is closed.
pub struct NamedMutexGuard {
    handle: KernelHandle,
    _thread_bound: PhantomData<Rc<()>>,
}

impl NamedMutexGuard {
    pub fn try_acquire(name: &str) -> io::Result<Option<Self>> {
        let name = wide_name(name)?;
        let raw = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let handle = KernelHandle(raw);
        match unsafe { WaitForSingleObject(handle.0, 0) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(Self {
                handle,
                _thread_bound: PhantomData,
            })),
            WAIT_TIMEOUT => Ok(None),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            result => Err(io::Error::other(format!(
                "unexpected mutex wait result: {result:#x}"
            ))),
        }
    }
}

impl Drop for NamedMutexGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle.0);
        }
    }
}

/// Checks ownership by another thread without keeping the lock acquired.
/// A retained object with no owner must not suppress supervisor recovery.
pub fn held_by_other_thread(name: &str) -> io::Result<bool> {
    let name = wide_name(name)?;
    let raw = unsafe { OpenMutexW(SYNCHRONIZE | MUTEX_MODIFY_STATE, 0, name.as_ptr()) };
    if raw.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(2) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    let handle = KernelHandle(raw);
    match unsafe { WaitForSingleObject(handle.0, 0) } {
        WAIT_TIMEOUT => Ok(true),
        WAIT_OBJECT_0 | WAIT_ABANDONED => {
            if unsafe { ReleaseMutex(handle.0) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(false)
            }
        }
        WAIT_FAILED => Err(io::Error::last_os_error()),
        result => Err(io::Error::other(format!(
            "unexpected mutex probe result: {result:#x}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn named_mutex_concurrent_start_has_one_owner_and_releases_for_the_next_start() {
        let name = format!(
            "Local\\TenRate.MutexRehearsal.concurrent.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let start = Arc::new(Barrier::new(16));
        let finish = Arc::new(Barrier::new(16));
        let mut contenders = Vec::new();
        for _ in 0..16 {
            let (name, start, finish) = (name.clone(), start.clone(), finish.clone());
            contenders.push(std::thread::spawn(move || {
                start.wait();
                let result = NamedMutexGuard::try_acquire(&name);
                finish.wait();
                result.unwrap().is_some()
            }));
        }
        assert_eq!(
            contenders
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>(),
            1,
            "concurrent starts must never admit two supervisors"
        );
        assert!(NamedMutexGuard::try_acquire(&name).unwrap().is_some());
    }
}
