// v2.22.35 - Native, owned file dialogs for portable backups and note exports.

fn desktop_transfer_file_dialog(
    save: bool,
    title: &str,
    default_name: &str,
    extension: &str,
    label: &str,
) -> io::Result<Option<PathBuf>> {
    #[cfg(target_os = "windows")]
    {
        transfer_native_dialog::choose(save, title, default_name, extension, label)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (save, title, default_name, extension, label);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "此平台没有原生文件选择器",
        ))
    }
}

#[cfg(target_os = "windows")]
mod transfer_native_dialog {
    use super::*;
    use std::os::windows::ffi::OsStringExt;

    #[repr(C)]
    struct OpenFileNameW {
        size: u32,
        owner: *mut c_void,
        instance: *mut c_void,
        filter: *const u16,
        custom_filter: *mut u16,
        max_custom_filter: u32,
        filter_index: u32,
        file: *mut u16,
        max_file: u32,
        file_title: *mut u16,
        max_file_title: u32,
        initial_directory: *const u16,
        title: *const u16,
        flags: u32,
        file_offset: u16,
        extension_offset: u16,
        default_extension: *const u16,
        custom_data: isize,
        hook: usize,
        template_name: *const u16,
        reserved: *mut c_void,
        reserved_flags: u32,
        extended_flags: u32,
    }

    #[link(name = "comdlg32")]
    extern "system" {
        fn GetOpenFileNameW(value: *mut OpenFileNameW) -> i32;
        fn GetSaveFileNameW(value: *mut OpenFileNameW) -> i32;
        fn CommDlgExtendedError() -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetActiveWindow() -> *mut c_void;
    }

    pub(super) fn choose(
        save: bool,
        title: &str,
        default_name: &str,
        extension: &str,
        label: &str,
    ) -> io::Result<Option<PathBuf>> {
        let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let title = wide(title);
        let extension_wide = wide(extension);
        let filter = wide(&format!("{label}\0*.{extension}\0所有文件\0*.*\0"));
        let mut buffer = vec![0_u16; 32_768];
        let name = default_name.encode_utf16().collect::<Vec<_>>();
        if name.len() >= buffer.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "文件名过长"));
        }
        buffer[..name.len()].copy_from_slice(&name);
        // Zero is valid for all optional pointers, flags, and the unused hook.
        let mut value: OpenFileNameW = unsafe { std::mem::zeroed() };
        value.size = std::mem::size_of::<OpenFileNameW>() as u32;
        value.owner = unsafe { GetActiveWindow() };
        value.filter = filter.as_ptr();
        value.filter_index = 1;
        value.file = buffer.as_mut_ptr();
        value.max_file = buffer.len() as u32;
        value.title = title.as_ptr();
        value.default_extension = extension_wide.as_ptr();
        // Explorer, no working-directory changes, existing destination folder.
        value.flags = 0x0008_0000 | 0x0000_0008 | 0x0000_0800 | 0x0000_0004;
        value.flags |= if save { 0x0000_0002 } else { 0x0000_1000 };
        let accepted = unsafe {
            if save {
                GetSaveFileNameW(&mut value)
            } else {
                GetOpenFileNameW(&mut value)
            }
        };
        if accepted == 0 {
            let code = unsafe { CommDlgExtendedError() };
            return if code == 0 {
                Ok(None)
            } else {
                Err(io::Error::other(format!("文件选择器错误：0x{code:08x}")))
            };
        }
        let len = buffer
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(buffer.len());
        Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
            &buffer[..len],
        ))))
    }
}
