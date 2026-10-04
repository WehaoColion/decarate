// v0.0.2 - Retain real kernel objects across bounded owner-process exit and normal release.
use std::ffi::c_void;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
type Handle = *mut c_void;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(attributes: *mut c_void, owner: i32, name: *const u16) -> Handle;
    fn CreateEventW(attributes: *mut c_void, manual: i32, initial: i32, name: *const u16)
        -> Handle;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn ReleaseMutex(handle: Handle) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}
pub(super) struct RetainedObject(Handle, bool);
impl Drop for RetainedObject {
    fn drop(&mut self) {
        unsafe {
            if self.1 {
                ReleaseMutex(self.0);
            }
            CloseHandle(self.0);
        }
    }
}
pub(super) fn unique_name(label: &str) -> String {
    format!(
        "Local\\TenRate.MutexRehearsal.{}.{label}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}
pub(super) fn unowned(name: &str) -> RetainedObject {
    mutex(name, false)
}
pub(super) fn owned(name: &str) -> RetainedObject {
    mutex(name, true)
}
fn mutex(name: &str, owner: bool) -> RetainedObject {
    assert!(name.starts_with("Local\\TenRate.MutexRehearsal."));
    let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null_mut(), i32::from(owner), name.as_ptr()) };
    assert!(!handle.is_null(), "{}", std::io::Error::last_os_error());
    RetainedObject(handle, owner)
}

pub(super) fn wrong_object(name: &str) -> RetainedObject {
    assert!(name.starts_with("Local\\TenRate.MutexRehearsal."));
    let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let handle = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, name.as_ptr()) };
    assert!(!handle.is_null(), "{}", std::io::Error::last_os_error());
    RetainedObject(handle, false)
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
pub(super) struct AbandonedFixture {
    pub(super) name: String,
    _retained: RetainedObject,
    directory: PathBuf,
}
impl Drop for AbandonedFixture {
    fn drop(&mut self) {
        if let (Ok(root), Ok(temp)) = (
            fs::canonicalize(&self.directory),
            fs::canonicalize(std::env::temp_dir()),
        ) {
            if root.starts_with(temp)
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("tenrate_mutex_owner_")
            {
                let _ = fs::remove_dir_all(root);
            }
        }
    }
}

pub(super) fn owner_child_if_requested() -> bool {
    let Some(directory) = std::env::var_os("TENRATE_MUTEX_OWNER_DIR") else {
        return false;
    };
    let directory = fs::canonicalize(directory).unwrap();
    assert!(directory.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
    assert!(directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("tenrate_mutex_owner_"));
    assert_eq!(
        fs::read(directory.join("fixture.marker")).unwrap(),
        b"mutex-owner-v1"
    );
    let name = fs::read_to_string(directory.join("mutex.name")).unwrap();
    let _ownership = owned(&name);
    fs::write(directory.join("ready"), b"owned").unwrap();
    let started = Instant::now();
    loop {
        if directory.join("exit").exists() {
            std::process::exit(88);
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "parent did not request the owned fixture exit"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn abandoned(label: &str) -> AbandonedFixture {
    use std::os::windows::process::CommandExt;
    let name = unique_name(label);
    let directory = std::env::temp_dir().join(format!(
        "tenrate_mutex_owner_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("fixture.marker"), b"mutex-owner-v1").unwrap();
    fs::write(directory.join("mutex.name"), &name).unwrap();
    let error = fs::File::create(directory.join("child.err")).unwrap();
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::mutex_liveness_after_owner_process_exit",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("TENRATE_MUTEX_OWNER_DIR", &directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(error)
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while !directory.join("ready").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "{}",
            fs::read_to_string(directory.join("child.err")).unwrap()
        );
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "owned mutex child startup timed out"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let wide = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let retained = unsafe { OpenMutexW(0x0010_0001, 0, wide.as_ptr()) };
    assert!(!retained.is_null(), "{}", std::io::Error::last_os_error());
    let fixture = AbandonedFixture {
        name,
        _retained: RetainedObject(retained, false),
        directory,
    };
    fs::write(fixture.directory.join("exit"), b"exit-without-release").unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert_eq!(status.code(), Some(88));
            println!(
                "mutex owner process {} exited 88 while a separate handle retained its object",
                child.0.id()
            );
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "owned mutex child exit timed out"
        );
        thread::sleep(Duration::from_millis(10));
    }
    fixture
}
