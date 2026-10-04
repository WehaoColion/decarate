// v2.22.35 - Local file transcription with cancellable decoding and reviewed document creation.

const SPEECH_MAX_SECONDS: u64 = 2 * 60 * 60;
const SPEECH_PCM_RATE: u32 = 16_000;
const SPEECH_PCM_BYTES_PER_SECOND: u64 = SPEECH_PCM_RATE as u64 * 2;
const SPEECH_MAX_PCM_BYTES: u64 = SPEECH_MAX_SECONDS * SPEECH_PCM_BYTES_PER_SECOND;
const SPEECH_MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct SpeechTranscriptionState {
    running: Option<SpeechTranscriptionJob>,
    preview: Option<SpeechTranscriptionPreview>,
    preview_open: bool,
}

struct SpeechTranscriptionJob {
    origin: AiWorkspaceIdentity,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    progress: Arc<std::sync::Mutex<SpeechTranscriptionProgress>>,
    receiver: mpsc::Receiver<io::Result<LocalSpeechTranscript>>,
    title: String,
}

impl Drop for SpeechTranscriptionJob {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Default)]
struct SpeechTranscriptionProgress {
    stage: &'static str,
    decoded_millis: u64,
    recognized_millis: u64,
}

struct SpeechTranscriptionPreview {
    origin: AiWorkspaceIdentity,
    title: String,
    text: String,
    duration_millis: u64,
}

#[derive(Debug)]
struct LocalSpeechTranscript {
    text: String,
    duration_millis: u64,
}

#[derive(Clone)]
struct SpeechTranscriptionControl {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    supervisor: Option<CancellationToken>,
    progress: Arc<std::sync::Mutex<SpeechTranscriptionProgress>>,
}

impl SpeechTranscriptionControl {
    fn check(&self) -> io::Result<()> {
        if self.cancel.load(std::sync::atomic::Ordering::Acquire)
            || self
                .supervisor
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
        {
            Err(io::Error::new(io::ErrorKind::Interrupted, "转写已取消"))
        } else {
            Ok(())
        }
    }

    fn progress(&self, stage: &'static str, decoded_millis: u64, recognized_millis: u64) {
        if let Ok(mut progress) = self.progress.lock() {
            *progress = SpeechTranscriptionProgress {
                stage,
                decoded_millis,
                recognized_millis,
            };
        }
    }
}

fn speech_require_workspace(
    origin: &AiWorkspaceIdentity,
    current: &AiWorkspaceIdentity,
) -> io::Result<()> {
    if origin == current {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "工作区已切换，转写结果未写入",
        ))
    }
}

// Validate before any file/COM access. Reject URLs, UNC, device paths, ADS and
// relative names; the subsequent open also rejects reparse points/network drives.
fn speech_local_path_syntax(value: &str) -> io::Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() < 4
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
        || value[2..].contains(':')
        || value.contains('\0')
        || value.contains(['?', '*'])
        || value[3..].split(['\\', '/']).any(|part| {
            part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "请选择本机磁盘中的音视频文件",
        ));
    }
    Ok(())
}

fn speech_validate_pcm_size(current: u64, added: usize, timestamp_100ns: i64) -> io::Result<u64> {
    let total = current
        .checked_add(added as u64)
        .filter(|value| *value <= SPEECH_MAX_PCM_BYTES)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "文件超过 2 小时，转写已停止"))?;
    if timestamp_100ns > (SPEECH_MAX_SECONDS * 10_000_000) as i64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "文件超过 2 小时，转写已停止",
        ));
    }
    if added % 2 != 0 || total % 2 != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "音频样本不完整"));
    }
    Ok(total)
}

fn speech_append_phrase(text: &mut String, phrase: &str) -> io::Result<()> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        return Ok(());
    }
    let separator = usize::from(!text.is_empty());
    if text
        .len()
        .checked_add(phrase.len())
        .and_then(|n| n.checked_add(separator))
        .is_none_or(|size| size > SPEECH_MAX_TEXT_BYTES)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "转写文本过长，请拆分文件后重试",
        ));
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(phrase);
    Ok(())
}

fn speech_document_candidate(
    state: &str,
    preview: &SpeechTranscriptionPreview,
    current: &AiWorkspaceIdentity,
    id: &str,
    now: i64,
) -> io::Result<String> {
    speech_require_workspace(&preview.origin, current)?;
    if preview.text.trim().is_empty()
        || preview.text.len() > SPEECH_MAX_TEXT_BYTES
        || preview.title.chars().count() > 160
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "请检查转写标题和文字的长度",
        ));
    }
    // Even an accidental reused identifier must not turn this operation into
    // overwriting an existing or deleted document.
    let current: Value = serde_json::from_str(state).map_err(io::Error::other)?;
    if id.trim().is_empty()
        || current
            .get("notes")
            .and_then(Value::as_array)
            .is_some_and(|notes| {
                notes
                    .iter()
                    .any(|note| note.get("id").and_then(Value::as_str) == Some(id))
            })
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "文档标识已存在，请重试",
        ));
    }
    let title = if preview.title.trim().is_empty() {
        "文件转写"
    } else {
        preview.title.trim()
    };
    let note = desktop_note_save_value(
        id,
        DesktopNoteKind::Document,
        title,
        &preview.text,
        None,
        now,
    );
    app_data::upsert_note_app_data_json(state, &note.to_string(), now)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "无法创建转写文档"))
}

fn speech_wave_header(pcm_bytes: u64) -> io::Result<[u8; 44]> {
    let pcm_bytes =
        u32::try_from(speech_validate_pcm_size(pcm_bytes, 0, 0)?).map_err(io::Error::other)?;
    let mut header = [0_u8; 44];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(pcm_bytes + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&1_u16.to_le_bytes());
    header[24..28].copy_from_slice(&SPEECH_PCM_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&(SPEECH_PCM_BYTES_PER_SECOND as u32).to_le_bytes());
    header[32..34].copy_from_slice(&2_u16.to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&pcm_bytes.to_le_bytes());
    Ok(header)
}

struct SpeechTemporaryWave {
    path: PathBuf,
}

impl SpeechTemporaryWave {
    fn create(workspace: &Path) -> io::Result<(Self, fs::File)> {
        let workspace = fs::canonicalize(workspace)?;
        let directory = workspace.join("transcription_temp");
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || speech_metadata_is_reparse(&metadata)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "转写临时目录不安全",
            ));
        }
        if fs::canonicalize(&directory)?.parent() != Some(workspace.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "转写临时目录不属于当前工作区",
            ));
        }
        let path = directory.join(format!("{}.wav", random_desktop_identifier("speech")));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok((Self { path }, file))
    }
}

impl Drop for SpeechTemporaryWave {
    fn drop(&mut self) {
        // Only remove the single file this operation created. Never enumerate or
        // recursively delete a workspace directory.
        let _ = fs::remove_file(&self.path);
    }
}

fn speech_metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = metadata;
        false
    }
}

fn transcribe_local_speech_file(
    input: &Path,
    workspace: &Path,
    control: &SpeechTranscriptionControl,
) -> io::Result<LocalSpeechTranscript> {
    control.check()?;
    #[cfg(target_os = "windows")]
    {
        local_speech_windows::transcribe(input, workspace, control)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (input, workspace);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "文件转写需要 Windows",
        ))
    }
}

#[cfg(target_os = "windows")]
mod local_speech_windows {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_platform::core::{w, IUnknown, Interface, PCWSTR, PWSTR};
    use windows_platform::Win32::Media::MediaFoundation::*;
    use windows_platform::Win32::Media::Speech::*;
    use windows_platform::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_platform::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };

    fn native<T>(result: windows_platform::core::Result<T>, operation: &str) -> io::Result<T> {
        result.map_err(|error| io::Error::other(format!("{operation}：{error}")))
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    struct ComApartment;
    impl ComApartment {
        fn start() -> io::Result<Self> {
            native(
                unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() },
                "无法初始化本地语音组件",
            )?;
            Ok(Self)
        }
    }
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    struct MediaFoundation;
    impl MediaFoundation {
        fn start() -> io::Result<Self> {
            // Local byte streams only. Do not initialize Winsock transports.
            native(
                unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) },
                "无法初始化音频解码器",
            )?;
            Ok(Self)
        }
    }
    impl Drop for MediaFoundation {
        fn drop(&mut self) {
            let _ = unsafe { MFShutdown() };
        }
    }

    fn local_file(input: &Path) -> io::Result<(PathBuf, fs::File)> {
        let input_text = input
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "文件路径无法识别"))?;
        speech_local_path_syntax(input_text)?;
        let extension = input
            .extension()
            .and_then(|part| part.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(
            extension.as_str(),
            "wav"
                | "mp3"
                | "mp4"
                | "m4a"
                | "m4v"
                | "aac"
                | "wma"
                | "wmv"
                | "asf"
                | "avi"
                | "mov"
                | "mkv"
                | "flac"
                | "ogg"
                | "opus"
                | "webm"
                | "3gp"
                | "3g2"
                | "aif"
                | "aiff"
                | "mpeg"
                | "mpg"
                | "m2ts"
                | "ts"
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "请选择音频或视频文件；不支持播放列表和网络地址",
            ));
        }
        let drive = format!("{}\\", &input_text[..2])
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        if !matches!(
            unsafe { GetDriveTypeW(PCWSTR(drive.as_ptr())) },
            2 | 3 | 5 | 6
        ) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "请先将文件复制到本机磁盘",
            ));
        }
        for ancestor in input.ancestors() {
            let metadata = fs::symlink_metadata(ancestor)?;
            if metadata.file_type().is_symlink() || speech_metadata_is_reparse(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "请使用本机普通文件，不能通过链接或云端占位文件转写",
                ));
            }
        }
        // Deny writers/deletion for the duration of decoding. The selected file
        // cannot be replaced after validation while the decoder reopens it.
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0020_0000)
            .open(input)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || speech_metadata_is_reparse(&metadata) || metadata.len() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "文件为空或不是普通音视频文件",
            ));
        }
        let canonical = fs::canonicalize(input)?;
        // MF's file byte stream accepts DOS paths. Strip only the canonical DOS
        // prefix; never translate a device or UNC path into an accepted input.
        let canonical_text = canonical
            .to_str()
            .ok_or_else(|| io::Error::other("文件路径无法识别"))?;
        let ordinary = canonical_text
            .strip_prefix("\\\\?\\")
            .unwrap_or(canonical_text);
        speech_local_path_syntax(ordinary)?;
        Ok((PathBuf::from(ordinary), file))
    }

    fn chinese_token(category_name: PCWSTR) -> io::Result<ISpObjectToken> {
        let category: ISpObjectTokenCategory = native(
            unsafe { CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_INPROC_SERVER) },
            "无法读取本机语音引擎",
        )?;
        native(
            unsafe { category.SetId(category_name, false) },
            "无法读取本机语音引擎",
        )?;
        let tokens = native(
            unsafe { category.EnumTokens(w!("Language=804"), PCWSTR::null()) },
            "无法查找中文语音引擎",
        )?;
        let mut count = 0;
        native(
            unsafe { tokens.GetCount(&mut count) },
            "无法查找中文语音引擎",
        )?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "未找到本机中文离线语音引擎，请在 Windows 语言设置中安装中文语音识别",
            ));
        }
        native(unsafe { tokens.Item(0) }, "无法加载中文语音引擎")
    }

    fn check_pcm_type(reader: &IMFSourceReader) -> io::Result<()> {
        let current = native(
            unsafe { reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32) },
            "无法核验音频格式",
        )?;
        let major = native(
            unsafe { current.GetGUID(&MF_MT_MAJOR_TYPE) },
            "音频格式缺少类型",
        )?;
        let subtype = native(
            unsafe { current.GetGUID(&MF_MT_SUBTYPE) },
            "音频格式缺少编码",
        )?;
        for (key, expected) in [
            (MF_MT_AUDIO_NUM_CHANNELS, 1),
            (MF_MT_AUDIO_SAMPLES_PER_SECOND, SPEECH_PCM_RATE),
            (MF_MT_AUDIO_BITS_PER_SAMPLE, 16),
            (MF_MT_AUDIO_BLOCK_ALIGNMENT, 2),
            (
                MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                SPEECH_PCM_BYTES_PER_SECOND as u32,
            ),
        ] {
            if native(unsafe { current.GetUINT32(&key) }, "音频格式缺少参数")? != expected {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "音频无法转换为语音识别所需格式",
                ));
            }
        }
        if major != MFMediaType_Audio || subtype != MFAudioFormat_PCM {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "音频解码器返回了不支持的编码",
            ));
        }
        Ok(())
    }

    struct LockedAudioBuffer<'a>(&'a IMFMediaBuffer);
    impl Drop for LockedAudioBuffer<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.0.Unlock() };
        }
    }

    fn decode_pcm(
        input: &Path,
        file: &mut fs::File,
        control: &SpeechTranscriptionControl,
    ) -> io::Result<u64> {
        let input_wide = wide(input);
        let byte_stream = native(
            unsafe {
                MFCreateFile(
                    MF_ACCESSMODE_READ,
                    MF_OPENMODE_FAIL_IF_NOT_EXIST,
                    MF_FILEFLAGS_NONE,
                    PCWSTR(input_wide.as_ptr()),
                )
            },
            "无法打开本地音视频",
        )?;
        let reader = native(
            unsafe { MFCreateSourceReaderFromByteStream(&byte_stream, None) },
            "Windows 无法解码此文件，文件可能损坏或缺少相应编解码器",
        )?;
        let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        native(
            unsafe { reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false) },
            "无法选择音轨",
        )?;
        native(
            unsafe { reader.SetStreamSelection(stream, true) },
            "文件没有可转写的音轨",
        )?;
        let output_type = native(unsafe { MFCreateMediaType() }, "无法配置音频格式")?;
        native(
            unsafe { output_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio) },
            "无法配置音频格式",
        )?;
        native(
            unsafe { output_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM) },
            "无法配置音频格式",
        )?;
        for (key, value) in [
            (MF_MT_AUDIO_NUM_CHANNELS, 1),
            (MF_MT_AUDIO_SAMPLES_PER_SECOND, SPEECH_PCM_RATE),
            (MF_MT_AUDIO_BITS_PER_SAMPLE, 16),
            (MF_MT_AUDIO_BLOCK_ALIGNMENT, 2),
            (
                MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                SPEECH_PCM_BYTES_PER_SECOND as u32,
            ),
        ] {
            native(
                unsafe { output_type.SetUINT32(&key, value) },
                "无法配置音频格式",
            )?;
        }
        native(
            unsafe { reader.SetCurrentMediaType(stream, None, &output_type) },
            "无法将音轨转换为单声道语音",
        )?;
        check_pcm_type(&reader)?;
        file.write_all(&speech_wave_header(0)?)?;
        let started = Instant::now();
        let mut total = 0_u64;
        let mut last_progress = Instant::now();
        loop {
            control.check()?;
            if started.elapsed() > Duration::from_secs(20 * 60) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "音频解码超时，请拆分文件后重试",
                ));
            }
            let mut flags = 0;
            let mut timestamp = 0;
            let mut sample = None;
            native(
                unsafe {
                    reader.ReadSample(
                        stream,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut timestamp),
                        Some(&mut sample),
                    )
                },
                "读取音轨失败",
            )?;
            control.check()?;
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "音轨解码失败"));
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                check_pcm_type(&reader)?;
            }
            speech_validate_pcm_size(total, 0, timestamp)?;
            if let Some(sample) = sample {
                let end_timestamp = timestamp
                    .saturating_add(unsafe { sample.GetSampleDuration() }.unwrap_or(0).max(0));
                speech_validate_pcm_size(total, 0, end_timestamp)?;
                let buffer = native(
                    unsafe { sample.ConvertToContiguousBuffer() },
                    "无法读取音频样本",
                )?;
                let mut pointer = std::ptr::null_mut();
                let mut length = 0;
                native(
                    unsafe { buffer.Lock(&mut pointer, None, Some(&mut length)) },
                    "无法读取音频样本",
                )?;
                let _lock = LockedAudioBuffer(&buffer);
                let next = speech_validate_pcm_size(total, length as usize, end_timestamp)?;
                if length != 0 {
                    if pointer.is_null() {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "音频样本为空"));
                    }
                    let bytes = unsafe { std::slice::from_raw_parts(pointer, length as usize) };
                    for chunk in bytes.chunks(64 * 1024) {
                        control.check()?;
                        file.write_all(chunk)?;
                    }
                }
                total = next;
            }
            if last_progress.elapsed() >= Duration::from_millis(150) {
                control.progress("正在解码", total * 1000 / SPEECH_PCM_BYTES_PER_SECOND, 0);
                last_progress = Instant::now();
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        if total == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "文件没有有效音频",
            ));
        }
        control.check()?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&speech_wave_header(total)?)?;
        file.sync_all()?;
        Ok(total)
    }

    struct SpeechEvent(SPEVENT);
    impl SpeechEvent {
        fn id(&self) -> i32 {
            self.0._bitfield & 0xffff
        }
        fn parameter_type(&self) -> i32 {
            ((self.0._bitfield as u32) >> 16) as i32
        }
        fn recognition_text(&self) -> io::Result<String> {
            if self.parameter_type() != SPET_LPARAM_IS_OBJECT.0 || self.0.lParam.0 == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "识别结果格式无效",
                ));
            }
            // Borrow through AddRef; the original reference belongs to the
            // event and is released by its Drop implementation on every path.
            let raw = self.0.lParam.0 as *mut std::ffi::c_void;
            let borrowed = unsafe { IUnknown::from_raw_borrowed(&raw) }
                .ok_or_else(|| io::Error::other("识别结果为空"))?;
            let result: ISpRecoResult = native(borrowed.cast(), "无法读取识别结果")?;
            let mut text = PWSTR::null();
            native(
                unsafe { result.GetText(0, u32::MAX, true, &mut text, None) },
                "无法读取转写文字",
            )?;
            struct ComText(PWSTR);
            impl Drop for ComText {
                fn drop(&mut self) {
                    unsafe {
                        CoTaskMemFree(Some(self.0 .0 as *const std::ffi::c_void));
                    }
                }
            }
            let text = ComText(text);
            if text.0.is_null() {
                return Ok(String::new());
            }
            unsafe { text.0.to_string() }.map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("转写文字编码无效：{error}"),
                )
            })
        }
    }
    impl Drop for SpeechEvent {
        fn drop(&mut self) {
            let pointer = self.0.lParam.0 as *mut std::ffi::c_void;
            if pointer.is_null() {
                return;
            }
            unsafe {
                match self.parameter_type() {
                    kind if kind == SPET_LPARAM_IS_OBJECT.0 || kind == SPET_LPARAM_IS_TOKEN.0 => {
                        drop(IUnknown::from_raw(pointer));
                    }
                    kind if kind == SPET_LPARAM_IS_POINTER.0 || kind == SPET_LPARAM_IS_STRING.0 => {
                        CoTaskMemFree(Some(pointer.cast_const()))
                    }
                    _ => {}
                }
            }
        }
    }

    struct ActiveRecognition<'a>(&'a ISpRecognizer);
    impl Drop for ActiveRecognition<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.0.SetRecoState(SPRST_INACTIVE_WITH_PURGE) };
        }
    }

    fn recognize_wave(
        path: &Path,
        pcm_bytes: u64,
        control: &SpeechTranscriptionControl,
    ) -> io::Result<String> {
        control.check()?;
        let recognizer: ISpRecognizer = native(
            unsafe { CoCreateInstance(&SpInprocRecognizer, None, CLSCTX_INPROC_SERVER) },
            "无法启动本机离线语音识别",
        )?;
        native(
            unsafe { recognizer.SetRecognizer(&chinese_token(SPCAT_RECOGNIZERS)?) },
            "无法使用中文语音引擎",
        )?;
        native(
            unsafe { recognizer.SetRecoState(SPRST_INACTIVE) },
            "无法配置语音引擎",
        )?;
        let stream: ISpStream = native(
            unsafe { CoCreateInstance(&SpStream, None, CLSCTX_INPROC_SERVER) },
            "无法创建语音文件流",
        )?;
        let name = wide(path);
        native(
            unsafe { stream.BindToFile(PCWSTR(name.as_ptr()), SPFM_OPEN_READONLY, None, None, 0) },
            "无法打开解码后的音频",
        )?;
        // Never pass None here: None would select the default microphone.
        native(
            unsafe { recognizer.SetInput(&stream, true) },
            "无法设置语音文件输入",
        )?;
        let context = native(
            unsafe { recognizer.CreateRecoContext() },
            "无法创建识别上下文",
        )?;
        native(unsafe { context.SetNotifyWin32Event() }, "无法注册识别通知")?;
        // SPFEI_FLAGCHECK from the Windows SDK sapi.h, plus only the two events
        // consumed below. Without reserved bits, SAPI rejects this mask.
        let interest = (1_u64 << SPEI_RESERVED1.0)
            | (1_u64 << SPEI_RESERVED2.0)
            | (1_u64 << SPEI_RECOGNITION.0)
            | (1_u64 << SPEI_END_SR_STREAM.0);
        native(
            unsafe { context.SetInterest(interest, interest) },
            "无法注册识别事件",
        )?;
        let grammar = native(unsafe { context.CreateGrammar(1) }, "无法创建中文听写")?;
        native(
            unsafe { grammar.LoadDictation(PCWSTR::null(), SPLO_STATIC) },
            "中文语音引擎不支持本地听写",
        )?;
        native(
            unsafe { grammar.SetDictationState(SPRS_ACTIVE) },
            "无法开始中文听写",
        )?;
        let _active = ActiveRecognition(&recognizer);
        native(
            unsafe { recognizer.SetRecoState(SPRST_ACTIVE_ALWAYS) },
            "无法开始语音识别",
        )?;
        let duration_millis = pcm_bytes * 1000 / SPEECH_PCM_BYTES_PER_SECOND;
        control.progress("正在识别", duration_millis, 0);
        let started = Instant::now();
        let deadline =
            Duration::from_millis(duration_millis.saturating_mul(3).saturating_add(120_000));
        let mut last_offset = 0;
        let mut text = String::new();
        let mut ended = false;
        while !ended {
            control.check()?;
            if started.elapsed() > deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "语音识别超时，请拆分文件后重试",
                ));
            }
            // S_FALSE is a normal timeout. GetEvents' fetched count decides
            // whether there is an event; cancellation is checked every 100 ms.
            native(
                unsafe { context.WaitForNotifyEvent(100) },
                "等待识别结果失败",
            )?;
            loop {
                control.check()?;
                let mut raw = SPEVENT::default();
                let mut fetched = 0;
                native(
                    unsafe { context.GetEvents(1, &mut raw, &mut fetched) },
                    "获取识别结果失败",
                )?;
                if fetched == 0 {
                    break;
                }
                let event = SpeechEvent(raw);
                last_offset = last_offset.max(event.0.ullAudioStreamOffset.min(pcm_bytes));
                if event.id() == SPEI_RECOGNITION.0 {
                    speech_append_phrase(&mut text, &event.recognition_text()?)?;
                } else if event.id() == SPEI_END_SR_STREAM.0 {
                    // End-of-stream carries the engine's final HRESULT rather
                    // than an object pointer. Do not accept partial output on failure.
                    native(
                        windows_platform::core::HRESULT(event.0.lParam.0 as i32).ok(),
                        "语音引擎未完成转写",
                    )?;
                    ended = true;
                }
                control.progress(
                    "正在识别",
                    duration_millis,
                    last_offset * 1000 / SPEECH_PCM_BYTES_PER_SECOND,
                );
            }
        }
        control.check()?;
        if text.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "未识别到中文语音，请检查音轨是否清晰",
            ));
        }
        Ok(text)
    }

    pub(super) fn transcribe(
        input: &Path,
        workspace: &Path,
        control: &SpeechTranscriptionControl,
    ) -> io::Result<LocalSpeechTranscript> {
        control.check()?;
        let (input, _input_guard) = local_file(input)?;
        let _com = ComApartment::start()?;
        // Fail before decoding a large video when the Chinese engine is absent.
        let _token = chinese_token(SPCAT_RECOGNIZERS)?;
        let _foundation = MediaFoundation::start()?;
        control.check()?;
        let (temporary, mut file) = SpeechTemporaryWave::create(workspace)?;
        control.progress("正在解码", 0, 0);
        let decoded = decode_pcm(&input, &mut file, control);
        // Close the writer before SAPI reads the file, also on decode failure.
        drop(file);
        let pcm_bytes = decoded?;
        control.check()?;
        let text = recognize_wave(&temporary.path, pcm_bytes, control)?;
        control.check()?;
        // All COM readers have left scope before the temporary file is removed.
        Ok(LocalSpeechTranscript {
            text,
            duration_millis: pcm_bytes * 1000 / SPEECH_PCM_BYTES_PER_SECOND,
        })
    }

    #[cfg(test)]
    pub(super) fn synthesize_test_wave(path: &Path) -> io::Result<()> {
        use windows_platform::core::GUID;
        use windows_platform::Win32::Media::Audio::WAVEFORMATEX;
        let _com = ComApartment::start()?;
        let voice: ISpVoice = native(
            unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_INPROC_SERVER) },
            "无法创建测试语音",
        )?;
        native(
            unsafe { voice.SetVoice(&chinese_token(SPCAT_VOICES)?) },
            "无法使用中文合成语音",
        )?;
        let stream: ISpStream = native(
            unsafe { CoCreateInstance(&SpStream, None, CLSCTX_INPROC_SERVER) },
            "无法创建测试音频",
        )?;
        // SPDFID_WaveFormatEx is absent from windows-rs metadata; this is the
        // documented SAPI GUID, not a speech codec identifier.
        const WAVE_FORMAT: GUID = GUID::from_u128(0xc31adbae_527f_4ff5_a230_f62bb61ff70c);
        let format = WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: 1,
            nSamplesPerSec: SPEECH_PCM_RATE,
            nAvgBytesPerSec: SPEECH_PCM_BYTES_PER_SECOND as u32,
            nBlockAlign: 2,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let name = wide(path);
        native(
            unsafe {
                stream.BindToFile(
                    PCWSTR(name.as_ptr()),
                    SPFM_CREATE_ALWAYS,
                    Some(&WAVE_FORMAT),
                    Some(&format),
                    0,
                )
            },
            "无法创建测试 WAV",
        )?;
        // Output is explicitly the file stream; this smoke test never speaks
        // through the default audio endpoint and does not use a microphone.
        native(
            unsafe { voice.SetOutput(&stream, true) },
            "无法设置测试输出",
        )?;
        native(
            unsafe {
                voice.Speak(
                    w!("今天我们学习时间管理。完成工作以后，请休息五分钟。明天继续学习中文。"),
                    SPF_DEFAULT.0 as u32,
                    None,
                )
            },
            "无法合成测试语音",
        )?;
        drop(voice);
        native(unsafe { stream.Close() }, "无法保存测试音频")
    }
}

impl TimerWindowsClient {
    fn start_speech_transcription(&mut self, ctx: &egui::Context) {
        if self.workspace_edit_locked()
            || self.rich_editor.active
            || self.speech_transcription.running.is_some()
        {
            return;
        }
        if self.speech_transcription.preview.is_some() {
            self.speech_transcription.preview_open = true;
            self.status = "请先保存或放弃当前转写结果".to_string();
            return;
        }
        let input =
            match desktop_transfer_file_dialog(false, "选择本地音频或视频", "", "*", "音视频文件")
            {
                Ok(Some(input)) => input,
                Ok(None) => return,
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            };
        if let Err(error) = self.flush_document_operation_drafts() {
            self.status = format!("草稿保存失败：{error}");
            return;
        }
        let origin = self.ai_workspace_identity();
        let workspace = origin.namespace_root.clone();
        let title = format!(
            "{} 转写",
            input
                .file_stem()
                .and_then(|part| part.to_str())
                .unwrap_or("文件")
        )
        .chars()
        .take(160)
        .collect();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let progress = Arc::new(std::sync::Mutex::new(SpeechTranscriptionProgress::default()));
        let (sender, receiver) = mpsc::channel();
        let mut control = SpeechTranscriptionControl {
            cancel: cancel.clone(),
            supervisor: None,
            progress: progress.clone(),
        };
        let repaint = ctx.clone();
        match self.task_supervisor.spawn(
            RuntimeTaskKind::SpeechTranscription,
            TaskDurability::Ephemeral,
            move |supervisor| {
                control.supervisor = Some(supervisor);
                let result = transcribe_local_speech_file(&input, &workspace, &control);
                let _ = sender.send(result);
                repaint.request_repaint();
            },
        ) {
            Ok(_) => {
                self.speech_transcription.running = Some(SpeechTranscriptionJob {
                    origin,
                    cancel,
                    progress,
                    receiver,
                    title,
                });
                self.status = "正在转写本地文件".to_string();
            }
            Err(error) => self.status = format!("无法开始转写：{error}"),
        }
    }

    fn poll_speech_transcription(&mut self, ctx: &egui::Context) {
        let current = self.ai_workspace_identity();
        if self
            .speech_transcription
            .preview
            .as_ref()
            .is_some_and(|preview| preview.origin != current)
        {
            self.speech_transcription.preview = None;
            self.speech_transcription.preview_open = false;
        }
        if let Some(job) = self.speech_transcription.running.as_ref() {
            if job.origin != current {
                job.cancel.store(true, std::sync::atomic::Ordering::Release);
            }
            ctx.request_repaint_after(Duration::from_millis(200));
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("转写任务意外结束")))
                }
            };
            if let Some(result) = result {
                let job = self
                    .speech_transcription
                    .running
                    .take()
                    .expect("running job");
                if speech_require_workspace(&job.origin, &current).is_err() {
                    self.status = "工作区已切换，转写已取消".to_string();
                } else if job.cancel.load(std::sync::atomic::Ordering::Acquire) {
                    self.status = "转写已取消".to_string();
                } else {
                    match result {
                        Ok(result) => {
                            self.speech_transcription.preview = Some(SpeechTranscriptionPreview {
                                origin: job.origin.clone(),
                                title: job.title.clone(),
                                text: result.text,
                                duration_millis: result.duration_millis,
                            });
                            self.speech_transcription.preview_open = true;
                            self.status = "转写完成，请核对后保存为文档".to_string();
                        }
                        Err(error) => self.status = format!("转写未完成：{error}"),
                    }
                }
            }
        }
        self.show_speech_transcription_preview(ctx);
    }

    fn ui_speech_transcription(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = self.speech_transcription.running.as_ref() {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                if let Ok(progress) = job.progress.try_lock() {
                    let seconds = if progress.stage == "正在识别" {
                        progress.recognized_millis
                    } else {
                        progress.decoded_millis
                    } / 1000;
                    ui.label(format!(
                        "{} {:02}:{:02}",
                        if progress.stage.is_empty() {
                            "准备转写"
                        } else {
                            progress.stage
                        },
                        seconds / 60,
                        seconds % 60
                    ));
                }
                let cancelling = job.cancel.load(std::sync::atomic::Ordering::Acquire);
                if ui
                    .add_enabled(
                        !cancelling,
                        egui::Button::new(if cancelling {
                            "正在取消"
                        } else {
                            "取消转写"
                        }),
                    )
                    .clicked()
                {
                    job.cancel.store(true, std::sync::atomic::Ordering::Release);
                }
            });
        } else if self.speech_transcription.preview.is_some() {
            if ui.button("查看转写结果").clicked() {
                self.speech_transcription.preview_open = true;
            }
        } else if ui
            .add_enabled(
                !self.workspace_edit_locked() && !self.rich_editor.active,
                egui::Button::new("文件转写"),
            )
            .on_hover_text("选择本地音频或视频，使用本机中文语音引擎，最长 2 小时")
            .clicked()
        {
            self.start_speech_transcription(ui.ctx());
        }
    }

    fn show_speech_transcription_preview(&mut self, ctx: &egui::Context) {
        if !self.speech_transcription.preview_open {
            return;
        }
        let editable = !self.workspace_edit_locked() && !self.rich_editor.active;
        let Some(preview) = self.speech_transcription.preview.as_mut() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut discard = false;
        egui::Window::new("转写结果")
            .id(egui::Id::new("local_speech_preview"))
            .open(&mut open)
            .default_width(660.0)
            .resizable(true)
            .show(ctx, |ui| {
                let seconds = preview.duration_millis / 1000;
                ui.label(format!(
                    "音频 {:02}:{:02} · 核对文字后保存为新文档",
                    seconds / 60,
                    seconds % 60
                ));
                ui.add(
                    egui::TextEdit::singleline(&mut preview.title)
                        .desired_width(f32::INFINITY)
                        .hint_text("文档标题"),
                );
                egui::ScrollArea::vertical()
                    .max_height((ctx.screen_rect().height() - 220.0).clamp(120.0, 540.0))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut preview.text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(14),
                        );
                    });
                ui.horizontal_wrapped(|ui| {
                    save = ui
                        .add_enabled(
                            editable && !preview.text.trim().is_empty(),
                            egui::Button::new("保存为新文档"),
                        )
                        .clicked();
                    discard = ui.button("放弃结果").clicked();
                });
            });
        self.speech_transcription.preview_open = open;
        if discard {
            self.speech_transcription.preview = None;
            self.speech_transcription.preview_open = false;
        } else if save {
            self.save_speech_transcription_preview();
        }
    }

    fn save_speech_transcription_preview(&mut self) -> bool {
        if self.workspace_edit_locked() || self.rich_editor.active {
            return false;
        }
        let Some(preview) = self.speech_transcription.preview.as_ref() else {
            return false;
        };
        if let Err(error) = speech_require_workspace(&preview.origin, &self.ai_workspace_identity())
        {
            self.status = error.to_string();
            return false;
        }
        if preview.text.trim().is_empty() || preview.text.len() > SPEECH_MAX_TEXT_BYTES {
            self.status = "请检查转写文字的长度".to_string();
            return false;
        }
        if let Err(error) = self.flush_document_operation_drafts() {
            self.status = format!("现有草稿保存失败：{error}");
            return false;
        }
        let Some(preview) = self.speech_transcription.preview.as_ref() else {
            return false;
        };
        if let Err(error) = speech_require_workspace(&preview.origin, &self.ai_workspace_identity())
        {
            self.status = error.to_string();
            return false;
        }
        let now = now_millis();
        let id = random_desktop_identifier("note");
        let next = match speech_document_candidate(
            &self.state_json,
            preview,
            &self.ai_workspace_identity(),
            &id,
            now,
        ) {
            Ok(next) => next,
            Err(error) => {
                self.status = error.to_string();
                return false;
            }
        };
        if !self.replace_state(Some(next), "转写已保存为知识文档") {
            return false;
        }
        self.speech_transcription.preview = None;
        self.speech_transcription.preview_open = false;
        self.switch_tab(AppTab::Knowledge);
        self.select_note_by_id(&id);
        true
    }
}

#[cfg(test)]
mod speech_transcription_tests {
    use super::*;

    struct TestWorkspace(PathBuf);
    impl TestWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(random_desktop_identifier("timer-speech-test"));
            fs::create_dir(&root).expect("isolated speech test directory");
            Self(root)
        }
    }
    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn control() -> SpeechTranscriptionControl {
        SpeechTranscriptionControl {
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            supervisor: None,
            progress: Arc::new(std::sync::Mutex::new(SpeechTranscriptionProgress::default())),
        }
    }

    fn identity() -> AiWorkspaceIdentity {
        AiWorkspaceIdentity {
            namespace_root: PathBuf::from(r"C:\private\account-a"),
            state_path: PathBuf::from(r"C:\private\account-a\state.json"),
            server_instance_id: "server-a".to_string(),
            account_namespace: "account-a".to_string(),
            user_id: "user-a".to_string(),
        }
    }

    #[test]
    fn speech_paths_reject_remote_relative_device_and_alternate_stream_inputs() {
        for path in [
            "https://example.com/a.wav",
            "file:///C:/a.wav",
            r"\\server\share\a.wav",
            r"\\?\C:\a.wav",
            r"\\.\pipe\audio",
            r"C:a.wav",
            r"C:\a.wav:stream",
            r"C:\one\..\a.wav",
            r"C:\one.\a.wav",
            r"C:\a?.wav",
            "C:\\a\0.wav",
            "a.wav",
        ] {
            assert!(speech_local_path_syntax(path).is_err(), "accepted {path:?}");
        }
        for path in [r"C:\audio\采访录音.wav", "D:/media/my recording.mp4"] {
            speech_local_path_syntax(path).expect("normal local path");
        }
    }

    #[test]
    fn speech_pcm_limits_reject_overflow_partial_samples_and_overlong_timestamps() {
        assert_eq!(
            speech_validate_pcm_size(SPEECH_MAX_PCM_BYTES - 2, 2, 72_000_000_000).unwrap(),
            SPEECH_MAX_PCM_BYTES
        );
        assert!(speech_validate_pcm_size(SPEECH_MAX_PCM_BYTES, 2, 0).is_err());
        assert!(speech_validate_pcm_size(u64::MAX, 2, 0).is_err());
        assert!(speech_validate_pcm_size(0, 1, 0).is_err());
        assert!(speech_validate_pcm_size(1, 0, 0).is_err());
        assert!(speech_validate_pcm_size(0, 0, 72_000_000_001).is_err());
        assert!(speech_validate_pcm_size(0, 0, i64::MAX).is_err());
    }

    #[test]
    fn speech_wave_header_matches_pcm_file_layout() {
        let header = speech_wave_header(32_000).unwrap();
        assert_eq!(&header[..4], b"RIFF");
        assert_eq!(&header[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 32_036);
        assert_eq!(u16::from_le_bytes(header[22..24].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            16_000
        );
        assert_eq!(u16::from_le_bytes(header[34..36].try_into().unwrap()), 16);
        assert_eq!(&header[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes(header[40..44].try_into().unwrap()),
            32_000
        );
    }

    #[test]
    fn speech_result_keeps_repeated_phrases_and_bounds_text_before_appending() {
        let mut text = String::new();
        speech_append_phrase(&mut text, " 同一句话 ").unwrap();
        speech_append_phrase(&mut text, "同一句话").unwrap();
        speech_append_phrase(&mut text, "  ").unwrap();
        assert_eq!(text, "同一句话\n同一句话");
        let original = text.clone();
        assert!(speech_append_phrase(&mut text, &"x".repeat(SPEECH_MAX_TEXT_BYTES)).is_err());
        assert_eq!(
            text, original,
            "failed append must not leave a partial phrase"
        );
    }

    #[test]
    fn speech_cancelled_before_start_does_not_access_input_or_create_temp_files() {
        let workspace = TestWorkspace::new();
        let control = control();
        control
            .cancel
            .store(true, std::sync::atomic::Ordering::Release);
        let error =
            transcribe_local_speech_file(Path::new("does-not-exist.wav"), &workspace.0, &control)
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_dir(&workspace.0).unwrap().count(), 0);
    }

    #[test]
    fn speech_supervisor_shutdown_reaches_backend_cancellation() {
        let mut supervisor = TaskSupervisor::default();
        let (ready_sender, ready_receiver) = mpsc::channel();
        let (finish_sender, finish_receiver) = mpsc::channel();
        let mut control = control();
        supervisor
            .spawn(
                RuntimeTaskKind::SpeechTranscription,
                TaskDurability::Ephemeral,
                move |token| {
                    control.supervisor = Some(token);
                    ready_sender.send(()).unwrap();
                    let start = Instant::now();
                    while control.check().is_ok() && start.elapsed() < Duration::from_secs(3) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    finish_sender
                        .send(control.check().map_err(|error| error.kind()))
                        .unwrap();
                },
            )
            .unwrap();
        ready_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        supervisor.begin_shutdown();
        assert_eq!(
            finish_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            Err(io::ErrorKind::Interrupted)
        );
        supervisor.drain_for(Duration::from_secs(2));
        assert_eq!(supervisor.active_count(), 0);
    }

    #[test]
    fn speech_temp_cleanup_removes_only_owned_wave_on_error() {
        let workspace = TestWorkspace::new();
        let neighbor = workspace.0.join("unrelated.txt");
        fs::write(&neighbor, b"preserve").unwrap();
        let owned_path;
        {
            let (temporary, mut file) = SpeechTemporaryWave::create(&workspace.0).unwrap();
            owned_path = temporary.path.clone();
            file.write_all(&speech_wave_header(0).unwrap()).unwrap();
            drop(file);
            assert!(owned_path.exists());
            // An interrupted decode returns through the same RAII cleanup.
        }
        assert!(!owned_path.exists());
        assert_eq!(fs::read(&neighbor).unwrap(), b"preserve");
        assert_eq!(
            fs::read_dir(workspace.0.join("transcription_temp"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn speech_scope_checks_every_identity_component_before_document_creation() {
        let origin = identity();
        let preview = SpeechTranscriptionPreview {
            origin: origin.clone(),
            title: "已校对".to_string(),
            text: "内容".to_string(),
            duration_millis: 5000,
        };
        let state = app_data::default_app_data_json(100);
        for index in 0..5 {
            let mut switched = origin.clone();
            match index {
                0 => switched.namespace_root.push("other"),
                1 => switched.state_path = PathBuf::from(r"C:\other\state.json"),
                2 => switched.server_instance_id.push('2'),
                3 => switched.account_namespace.push('2'),
                _ => switched.user_id.push('2'),
            }
            let error =
                speech_document_candidate(&state, &preview, &switched, "new-transcript", 200)
                    .unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::PermissionDenied,
                "identity component {index}"
            );
        }
    }

    #[test]
    fn speech_reviewed_result_creates_document_and_preserves_existing_content() {
        let origin = identity();
        let original = desktop_note_save_value(
            "existing-note",
            DesktopNoteKind::Sticky,
            "现有便签",
            "原文不可覆盖",
            None,
            100,
        );
        let initial = app_data::upsert_note_app_data_json(
            &app_data::default_app_data_json(100),
            &original.to_string(),
            100,
        )
        .unwrap();
        let preview = SpeechTranscriptionPreview {
            origin: origin.clone(),
            title: "人工校对后的标题".to_string(),
            text: "人工校对后的全文\n第二段".to_string(),
            duration_millis: 9000,
        };
        let next =
            speech_document_candidate(&initial, &preview, &origin, "new-transcript", 200).unwrap();
        let before: Value = serde_json::from_str(&initial).unwrap();
        let after: Value = serde_json::from_str(&next).unwrap();
        let notes = after["notes"].as_array().unwrap();
        assert_eq!(notes.len(), before["notes"].as_array().unwrap().len() + 1);
        assert_eq!(
            notes
                .iter()
                .find(|note| note["id"] == "existing-note")
                .unwrap(),
            before["notes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|note| note["id"] == "existing-note")
                .unwrap()
        );
        let created = notes
            .iter()
            .find(|note| note["id"] == "new-transcript")
            .unwrap();
        assert_eq!(created["kind"], "DOCUMENT");
        assert_eq!(created["title"], preview.title);
        assert_eq!(created["content"], preview.text);
        for key in [
            "slots",
            "sessions",
            "financeProfile",
            "categories",
            "themeMode",
        ] {
            assert_eq!(before.get(key), after.get(key), "changed {key}");
        }
        assert_eq!(
            speech_document_candidate(&initial, &preview, &origin, "existing-note", 201)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    #[ignore = "Explicit native smoke test: synthesizes Chinese speech to a private WAV file; never uses speaker, microphone, or network"]
    fn speech_native_chinese_synthetic_wave_transcribes_without_playback() {
        let workspace = TestWorkspace::new();
        let input = workspace.0.join("synthetic_chinese.wav");
        local_speech_windows::synthesize_test_wave(&input).expect("installed Chinese SAPI voice");
        let transcript = transcribe_local_speech_file(&input, &workspace.0, &control())
            .expect("local MF decode and Chinese SAPI dictation");
        assert!(!transcript.text.trim().is_empty());
        assert!(
            transcript
                .text
                .chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)),
            "expected Chinese, got {:?}",
            transcript.text
        );
        assert!(
            transcript.duration_millis > 1000
                && transcript.duration_millis <= SPEECH_MAX_SECONDS * 1000
        );
        assert_eq!(
            fs::read_dir(workspace.0.join("transcription_temp"))
                .unwrap()
                .count(),
            0,
            "decoded PCM must be removed after recognition"
        );
        assert!(
            input.exists(),
            "selected input must not be changed or removed"
        );
        eprintln!(
            "Synthetic local Chinese transcription ({} ms): {}",
            transcript.duration_millis, transcript.text
        );
    }
}
