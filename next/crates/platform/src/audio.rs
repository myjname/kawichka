//! Cancelable offline synthesis and playback on a specific WASAPI endpoint.
//! SAPI writes to memory, never directly to the default speakers. An already
//! opened WASAPI client cannot silently migrate to a different endpoint.
use kavychka_core::preferences::{AudioMode, Language};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, SyncSender},
};
use std::time::{Duration, Instant};
struct Request {
    text: String,
    file: Option<std::path::PathBuf>,
    language: Language,
    mode: AudioMode,
    generation: u64,
    device: String,
    notice: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Started,
    Completed,
    Cancelled,
    Failed,
}
pub type PhaseObserver = Arc<dyn Fn(Phase) + Send + Sync>;
#[derive(Clone, Debug)]
pub struct PlaybackEvent {
    pub phase: Phase,
    pub notice: Option<String>,
}
pub type PlaybackObserver = Arc<dyn Fn(PlaybackEvent) + Send + Sync>;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
pub fn devices() -> Result<Vec<Device>, String> {
    #[cfg(windows)]
    {
        let _apartment = windows_audio::Apartment::new()?;
        windows_audio::devices()
    }
    #[cfg(not(windows))]
    {
        Err("Windows audio adapter required".into())
    }
}
pub struct Output {
    sender: SyncSender<Option<Request>>,
    generation: Arc<AtomicU64>,
    busy: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    feedback: std::sync::mpsc::Receiver<Result<(), String>>,
}
impl Output {
    pub fn new() -> Self {
        Self::with_phase_observer(None)
    }
    /// An observer reports lifecycle facts, never text or key input. A Started
    /// fact means PCM was submitted to WASAPI, not that a human heard it.
    pub fn with_phase_observer(observer: Option<PhaseObserver>) -> Self {
        Self::with_event_observer(observer.map(|observer| {
            Arc::new(move |event: PlaybackEvent| observer(event.phase)) as PlaybackObserver
        }))
    }
    pub fn with_event_observer(observer: Option<PlaybackObserver>) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Option<Request>>(1);
        let (feedback, errors) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let busy = Arc::new(AtomicBool::new(false));
        let token = generation.clone();
        let working = busy.clone();
        let join = std::thread::spawn(move || {
            #[cfg(windows)]
            let apartment = windows_audio::Apartment::new();
            while let Ok(Some(request)) = receiver.recv() {
                let notice = request.notice.clone();
                let phase_observer = observer.as_ref().map(|observer| {
                    let observer = observer.clone();
                    Arc::new(move |phase| {
                        observer(PlaybackEvent {
                            phase,
                            notice: notice.clone(),
                        })
                    }) as PhaseObserver
                });
                if let Some(observer) = &phase_observer {
                    observer(Phase::Preparing);
                }
                let cancelled = || token.load(Ordering::Acquire) != request.generation;
                #[cfg(windows)]
                let result = match &apartment {
                    Ok(_) => windows_audio::play(&request, &cancelled, phase_observer.as_ref()),
                    Err(e) => Err(e.clone()),
                };
                #[cfg(not(windows))]
                let result: Result<Option<String>, String> =
                    Err("audio adapter is Windows-only".into());
                if let Some(observer) = &phase_observer {
                    observer(if cancelled() {
                        Phase::Cancelled
                    } else if matches!(&result, Ok(None)) {
                        Phase::Completed
                    } else {
                        Phase::Failed
                    });
                }
                if !cancelled() {
                    let _ = feedback.send(match result {
                        Err(e) | Ok(Some(e)) => Err(e),
                        Ok(None) => Ok(()),
                    });
                }
                working.store(false, Ordering::Release);
            }
        });
        Self {
            sender,
            generation,
            busy,
            join: Some(join),
            feedback: errors,
        }
    }
    pub fn play(&self, text: &str, language: Language, mode: AudioMode) -> Result<(), String> {
        self.play_on(text, language, mode, "")
    }
    pub fn play_on(
        &self,
        text: &str,
        language: Language,
        mode: AudioMode,
        device: &str,
    ) -> Result<(), String> {
        self.play_tagged(text, language, mode, device, None)
    }
    pub fn play_notice_on(
        &self,
        text: &str,
        language: Language,
        mode: AudioMode,
        device: &str,
        notice: &str,
    ) -> Result<(), String> {
        self.play_tagged(text, language, mode, device, Some(notice.into()))
    }
    fn play_tagged(
        &self,
        text: &str,
        language: Language,
        mode: AudioMode,
        device: &str,
        notice: Option<String>,
    ) -> Result<(), String> {
        if mode == AudioMode::Silent {
            return Ok(());
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("audio is already playing".into());
        }
        let request = Request {
            text: text.into(),
            file: None,
            language,
            mode,
            generation: self.generation.load(Ordering::Acquire),
            device: device.into(),
            notice,
        };
        if let Err(e) = self.sender.try_send(Some(request)) {
            self.busy.store(false, Ordering::Release);
            return Err(e.to_string());
        }
        Ok(())
    }
    pub fn play_file(&self, path: std::path::PathBuf, mode: AudioMode) -> Result<(), String> {
        self.play_file_on(path, mode, "")
    }
    pub fn play_file_on(
        &self,
        path: std::path::PathBuf,
        mode: AudioMode,
        device: &str,
    ) -> Result<(), String> {
        self.play_file_tagged(path, mode, device, None)
    }
    pub fn play_file_notice_on(
        &self,
        path: std::path::PathBuf,
        mode: AudioMode,
        device: &str,
        notice: &str,
    ) -> Result<(), String> {
        self.play_file_tagged(path, mode, device, Some(notice.into()))
    }
    fn play_file_tagged(
        &self,
        path: std::path::PathBuf,
        mode: AudioMode,
        device: &str,
        notice: Option<String>,
    ) -> Result<(), String> {
        if mode == AudioMode::Silent {
            return Ok(());
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("audio is already playing".into());
        }
        let request = Request {
            text: String::new(),
            file: Some(path),
            language: Language::Ru,
            mode,
            generation: self.generation.load(Ordering::Acquire),
            device: device.into(),
            notice,
        };
        if let Err(e) = self.sender.try_send(Some(request)) {
            self.busy.store(false, Ordering::Release);
            return Err(e.to_string());
        }
        Ok(())
    }
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub fn feedback(&self) -> Option<Result<(), String>> {
        self.feedback.try_iter().last()
    }
}
impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        self.stop();
        let _ = self.sender.send(None);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
pub fn chime() -> Vec<u8> {
    chime_at_rate(16000)
}
fn chime_at_rate(rate: usize) -> Vec<u8> {
    let samples = rate * 2 / 5;
    let mut pcm = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        let t = i as f64 / rate as f64;
        let local = (i % (rate / 5)) as f64 / (rate / 5) as f64;
        let envelope = (local * std::f64::consts::PI).sin().powi(2);
        let hz = if i < rate / 5 { 540. } else { 720. };
        let sample = ((t * hz * 2. * std::f64::consts::PI).sin() * envelope * 4000.) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    pcm.extend(std::iter::repeat_n(0u8, rate / 5));
    pcm
}
/// Bounded FLAC decoding and mono conversion; native sample rate is retained by playback.
pub fn decode_recording(
    path: &std::path::Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    decode_native(path, cancelled).map(|(pcm, _)| pcm)
}
fn decode_native(
    path: &std::path::Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<u8>, u32), String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 50_000_000 {
        return Err("recording exceeds 50 MB".into());
    }
    let mut reader =
        claxon::FlacReader::open(path).map_err(|e| format!("Invalid FLAC recording: {e}"))?;
    let info = reader.streaminfo();
    let channels = info.channels as usize;
    if !(1..=8).contains(&channels)
        || !(8000..=192000).contains(&info.sample_rate)
        || !(8..=32).contains(&info.bits_per_sample)
    {
        return Err("unsupported recording format".into());
    }
    let maximum = 16_000_000usize.min(info.sample_rate as usize * channels * 120);
    let mut pcm = Vec::<u8>::new();
    let mut sum = 0f64;
    let scale = 2f64.powi(info.bits_per_sample as i32 - 1);
    let mut count = 0;
    for sample in reader.samples() {
        if count >= maximum {
            return Err("recording exceeds playback duration limit".into());
        }
        if count % 1024 == 0 && cancelled() {
            return Ok((vec![], info.sample_rate));
        }
        sum += sample.map_err(|e| e.to_string())? as f64 / scale;
        count += 1;
        if count % channels == 0 {
            let sample = ((sum / channels as f64) * 32768.)
                .round()
                .clamp(-32768., 32767.) as i16;
            pcm.extend_from_slice(&sample.to_le_bytes());
            sum = 0.;
        }
    }
    if pcm.is_empty() || count % channels != 0 {
        return Err("empty or truncated recording".into());
    }
    Ok((pcm, info.sample_rate))
}
pub fn synthesis_probe(language: Language) -> Result<usize, String> {
    #[cfg(windows)]
    {
        let _apartment = windows_audio::Apartment::new()?;
        let pcm = windows_audio::synthesize(
            if language == Language::Ru {
                "Кавычка. Сделаем небольшой перерыв."
            } else {
                "Kavychka. Let us take a short break."
            },
            language,
            &|| false,
        )?;
        Ok(pcm.len())
    }
    #[cfg(not(windows))]
    {
        let _ = language;
        Err("Windows adapter required".into())
    }
}
pub fn hardware_probe() -> Result<(), String> {
    #[cfg(windows)]
    {
        let _apartment = windows_audio::Apartment::new()?;
        windows_audio::play(
            &Request {
                text: String::new(),
                file: None,
                language: Language::En,
                mode: AudioMode::Chime,
                generation: 0,
                device: String::new(),
                notice: None,
            },
            &|| false,
            None,
        )?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("Windows adapter required".into())
    }
}
#[cfg(windows)]
mod windows_audio {
    use super::*;
    use windows::{
        Win32::{
            Media::{Audio::*, Speech::*},
            System::Com::{StructuredStorage::CreateStreamOnHGlobal, *},
        },
        core::{GUID, PCWSTR, w},
    };
    pub struct Apartment;
    impl Apartment {
        pub fn new() -> Result<Self, String> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .ok()
                    .map_err(|e| e.to_string())?;
            }
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    fn format() -> WAVEFORMATEX {
        format_rate(16000)
    }
    fn format_rate(rate: u32) -> WAVEFORMATEX {
        WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: 1,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * 2,
            nBlockAlign: 2,
            wBitsPerSample: 16,
            cbSize: 0,
        }
    }
    pub fn synthesize(
        text: &str,
        language: Language,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, String> {
        unsafe {
            let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            let category: ISpObjectTokenCategory =
                CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| e.to_string())?;
            category
                .SetId(SPCAT_VOICES, false)
                .map_err(|e| e.to_string())?;
            let tokens = category
                .EnumTokens(PCWSTR::null(), PCWSTR::null())
                .map_err(|e| e.to_string())?;
            let mut count = 0;
            tokens.GetCount(&mut count).map_err(|e| e.to_string())?;
            let wanted = if language == Language::Ru { 0x19 } else { 0x09 };
            let mut selected = false;
            for index in 0..count {
                let token = tokens.Item(index).map_err(|e| e.to_string())?;
                if let Ok(attributes) = token.OpenKey(w!("Attributes"))
                    && let Ok(value) = attributes.GetStringValue(w!("Language"))
                {
                    let languages = value.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(value.0.cast()));
                    if languages
                        .split(';')
                        .any(|v| u32::from_str_radix(v, 16).is_ok_and(|v| v & 0x3ff == wanted))
                    {
                        voice.SetVoice(&token).map_err(|e| e.to_string())?;
                        selected = true;
                        break;
                    }
                }
            }
            if !selected {
                return Err(if language == Language::Ru {
                    "Русский системный голос не установлен. Текст и сигнал доступны."
                } else {
                    "No English system voice is installed. Text and chime remain available."
                }
                .into());
            }
            let memory =
                CreateStreamOnHGlobal(windows::Win32::Foundation::HGLOBAL::default(), true)
                    .map_err(|e| e.to_string())?;
            let stream: ISpStream = CoCreateInstance(&SpStream, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            // SPDFID_WaveFormatEx, also declared in the Windows SDK sapi.h.
            let wave = GUID::from_u128(0xc31adbae_527f_4ff5_a230_f62bb61ff70c);
            stream
                .SetBaseStream(&memory, &wave, &format())
                .map_err(|e| e.to_string())?;
            voice.SetOutput(&stream, false).map_err(|e| e.to_string())?;
            let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            voice
                .Speak(
                    PCWSTR(text.as_ptr()),
                    (SVSFlagsAsync.0 | SVSFIsNotXML.0) as u32,
                    None,
                )
                .map_err(|e| e.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if cancelled() || Instant::now() > deadline {
                    let _ = voice.Speak(PCWSTR::null(), SVSFPurgeBeforeSpeak.0 as u32, None);
                    return Err("speech cancelled or timed out".into());
                }
                let mut status = SPVOICESTATUS::default();
                voice
                    .GetStatus(&mut status, std::ptr::null_mut())
                    .map_err(|e| e.to_string())?;
                if status.dwRunningState == SPRS_DONE.0 as u32 {
                    status.hrLastResult.ok().map_err(|e| e.to_string())?;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let mut end = 0;
            memory
                .Seek(0, STREAM_SEEK_END, Some(&mut end))
                .map_err(|e| e.to_string())?;
            if end > 3_200_000 {
                return Err("system speech exceeded the 100-second memory limit".into());
            }
            memory
                .Seek(0, STREAM_SEEK_SET, None)
                .map_err(|e| e.to_string())?;
            let mut bytes = vec![0u8; end as usize];
            let mut read = 0;
            memory
                .Read(
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    Some(&mut read),
                )
                .ok()
                .map_err(|e| e.to_string())?;
            bytes.truncate(read as usize);
            if bytes.len() < 320 || !bytes.len().is_multiple_of(2) {
                return Err("system voice produced invalid or empty PCM".into());
            }
            Ok(bytes)
        }
    }
    fn endpoint_id(device: &IMMDevice) -> Result<String, String> {
        unsafe {
            let id = device.GetId().map_err(|e| e.to_string())?;
            let value = id.to_string().map_err(|e| e.to_string());
            CoTaskMemFree(Some(id.0.cast()));
            value
        }
    }
    pub fn devices() -> Result<Vec<Device>, String> {
        use windows::Win32::{
            Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
            System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
        };
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| e.to_string())?;
            let default = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .ok()
                .and_then(|d| endpoint_id(&d).ok());
            let collection = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(|e| e.to_string())?;
            let mut devices = Vec::new();
            for i in 0..collection.GetCount().map_err(|e| e.to_string())? {
                let device = collection.Item(i).map_err(|e| e.to_string())?;
                let id = endpoint_id(&device)?;
                let name = (|| -> Result<String, String> {
                    let store = device
                        .OpenPropertyStore(STGM_READ)
                        .map_err(|e| e.to_string())?;
                    let mut value = store
                        .GetValue(&PKEY_Device_FriendlyName)
                        .map_err(|e| e.to_string())?;
                    let text = PropVariantToStringAlloc(&value);
                    let _ = PropVariantClear(&mut value);
                    let text = text.map_err(|e| e.to_string())?;
                    let name = text.to_string().map_err(|e| e.to_string());
                    CoTaskMemFree(Some(text.0.cast()));
                    name
                })()
                .unwrap_or_else(|_| format!("Audio output {}", i + 1));
                devices.push(Device {
                    is_default: default.as_ref() == Some(&id),
                    id,
                    name,
                });
            }
            Ok(devices)
        }
    }
    struct Playing(IAudioClient);
    impl Drop for Playing {
        fn drop(&mut self) {
            unsafe {
                let _ = self.0.Stop();
            }
        }
    }
    pub fn play(
        request: &Request,
        cancelled: &dyn Fn() -> bool,
        observer: Option<&PhaseObserver>,
    ) -> Result<Option<String>, String> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| e.to_string())?;
            let device = if request.device.is_empty() {
                enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
            } else {
                let id: Vec<u16> = request.device.encode_utf16().chain(Some(0)).collect();
                enumerator.GetDevice(PCWSTR(id.as_ptr()))
            }
            .map_err(|e| format!("Selected audio output unavailable; no fallback: {e}"))?;
            if device.GetState().map_err(|e| e.to_string())? != DEVICE_STATE_ACTIVE {
                return Err("Selected audio output is disconnected; no fallback.".into());
            }
            let endpoint = endpoint_id(&device)?;
            let mut warning = None;
            let (voice, rate) = if matches!(request.mode, AudioMode::All | AudioMode::Voice) {
                let voice = match &request.file {
                    Some(file) => super::decode_native(file, cancelled),
                    None => synthesize(&request.text, request.language, cancelled)
                        .map(|pcm| (pcm, 16000)),
                };
                match voice {
                    Ok(voice) => voice,
                    Err(e) => {
                        warning = Some(e);
                        (vec![], 16000)
                    }
                }
            } else {
                (vec![], 16000)
            };
            let mut pcm = if matches!(request.mode, AudioMode::All | AudioMode::Chime) {
                super::chime_at_rate(rate as usize)
            } else {
                vec![]
            };
            pcm.extend(voice);
            if cancelled() || pcm.is_empty() {
                return Ok(warning);
            }
            let client: IAudioClient = device
                .Activate(CLSCTX_INPROC_SERVER, None)
                .map_err(|e| e.to_string())?;
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
                        | AUDCLNT_STREAMFLAGS_NOPERSIST,
                    1_000_000,
                    0,
                    &format_rate(rate),
                    None,
                )
                .map_err(|e| e.to_string())?;
            let buffer = client.GetBufferSize().map_err(|e| e.to_string())?;
            let renderer: IAudioRenderClient = client.GetService().map_err(|e| e.to_string())?;
            let total = pcm.len() / 2;
            let mut at = 0;
            client.Start().map_err(|e| e.to_string())?;
            let playing = Playing(client);
            let mut last_device_check = Instant::now() - Duration::from_secs(1);
            while at < total || playing.0.GetCurrentPadding().map_err(|e| e.to_string())? > 0 {
                if cancelled() {
                    return Ok(None);
                }
                if last_device_check.elapsed() >= Duration::from_millis(100) {
                    let changed = if request.device.is_empty() {
                        let current = enumerator
                            .GetDefaultAudioEndpoint(eRender, eConsole)
                            .map_err(|e| e.to_string())?;
                        endpoint_id(&current)? != endpoint
                    } else {
                        device.GetState().map_err(|e| e.to_string())? != DEVICE_STATE_ACTIVE
                    };
                    if changed {
                        return Err(
                            "Audio output changed; speech stopped without switching devices."
                                .into(),
                        );
                    }
                    last_device_check = Instant::now();
                }
                let padding = playing.0.GetCurrentPadding().map_err(|e| e.to_string())?;
                let frames = ((buffer.saturating_sub(padding)) as usize).min(total - at);
                if frames > 0 {
                    let target = renderer
                        .GetBuffer(frames as u32)
                        .map_err(|e| e.to_string())?;
                    std::ptr::copy_nonoverlapping(pcm.as_ptr().add(at * 2), target, frames * 2);
                    renderer
                        .ReleaseBuffer(frames as u32, 0)
                        .map_err(|e| e.to_string())?;
                    if at == 0
                        && let Some(observer) = observer
                    {
                        observer(Phase::Started);
                    }
                    at += frames;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(warning)
        }
    }
}
