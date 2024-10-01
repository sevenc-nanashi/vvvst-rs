mod models;
mod utils;
use base64::{engine::general_purpose::STANDARD as base64, Engine as _};
use include_dir::{include_dir, Dir};
use nih_plug::prelude::*;
use nih_plug_webview::*;
use serde_json::Value;
use std::borrow::Cow;
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    sync::{Arc, LazyLock, Mutex as StdMutex, Once},
};
use tokio::{runtime::Runtime, sync::RwLock};
use tracing::{error, info, warn};
use utils::TokioMutexParam;

use models::*;

pub static RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new().expect("Failed to create runtime"));
static INITIALIZE_LOG: Once = Once::new();

static EDITOR: Dir = include_dir!("$CARGO_MANIFEST_DIR/editor");

// TODO: そのうちマルチトラック・ステレオにする
#[derive(Debug, Default)]
struct Mixes {
    mixes: Vec<f32>,
    sample_rate: f32,
}

struct Vvvst {
    params: Arc<VvvstParams>,
    mixes: Arc<RwLock<Mixes>>,

    // 一瞬で終わるのでstdのMutexで十分...のはず？
    response_receiver: Arc<StdMutex<std::sync::mpsc::Receiver<Response>>>,

    response_sender: Arc<std::sync::mpsc::Sender<Response>>,
}

impl Default for Vvvst {
    fn default() -> Self {
        INITIALIZE_LOG.call_once(|| {
            if option_env!("VVVST_LOG").map_or(false, |v| v.len() > 0) {
                let dest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR").to_string())
                    .join("logs")
                    .join(format!(
                        "{}.log",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs()
                    ));

                let Ok(writer) = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&dest)
                else {
                    return;
                };

                let default_panic_hook = std::panic::take_hook();

                std::panic::set_hook(Box::new(move |info| {
                    let mut panic_writer =
                        std::fs::File::create(dest.with_extension("panic")).unwrap();
                    let _ = writeln!(panic_writer, "{:?}", info);

                    default_panic_hook(info);
                }));

                let _ = tracing_subscriber::fmt()
                    .with_writer(writer)
                    .with_ansi(false)
                    .try_init();
            }

            // TODO: ちゃんとエラーダイアログを出す
            let default_panic_hook = std::panic::take_hook();

            std::panic::set_hook(Box::new(move |info| {
                rfd::MessageDialog::new()
                    .set_title("VVVST: Panic")
                    .set_description(&format!("VVVST Panicked: {:?}", info))
                    .set_level(rfd::MessageLevel::Error)
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();

                default_panic_hook(info);

                std::process::exit(1);
            }));
        });
        let (response_sender, response_receiver) = std::sync::mpsc::channel();
        Self {
            params: Arc::new(VvvstParams::default()),
            mixes: Arc::new(RwLock::new(Mixes::default())),
            response_sender: Arc::new(response_sender),
            response_receiver: Arc::new(StdMutex::new(response_receiver)),
        }
    }
}

#[derive(Params, Default)]
struct VvvstParams {
    #[persist = "samples"]
    voices: TokioMutexParam<HashMap<SingingVoiceKey, Vec<u8>>>,
    #[persist = "phrases"]
    phrases: TokioMutexParam<Vec<Phrase>>,
    #[persist = "project"]
    project: TokioMutexParam<String>,
}

impl Vvvst {
    async fn process_request(
        params: Arc<VvvstParams>,
        request: RequestInner,

        mixes: Arc<RwLock<Mixes>>,
    ) -> anyhow::Result<Value> {
        match request {
            RequestInner::GetVersion => Ok(serde_json::to_value(env!("CARGO_PKG_VERSION"))?),
            RequestInner::GetProjectName => Ok(serde_json::to_value("VVVST")?),
            RequestInner::GetConfig => {
                // Windows: %APPDATA%/voicevox/config.json
                // macOS: ~/Library/Application Support/voicevox/config.json
                // Linux: ~/.config/voicevox/config.json
                let config_path = if cfg!(target_os = "windows") {
                    let appdata = std::env::var("APPDATA")?;
                    std::path::PathBuf::from(appdata).join("voicevox/config.json")
                } else if cfg!(target_os = "macos") {
                    let home = std::env::var("HOME")?;
                    std::path::PathBuf::from(home)
                        .join("Library/Application Support/voicevox/config.json")
                } else {
                    let home = std::env::var("HOME")?;
                    std::path::PathBuf::from(home).join(".config/voicevox/config.json")
                };

                if !config_path.exists() {
                    return Ok(serde_json::Value::Null);
                }
                let config = tokio::fs::read_to_string(config_path).await?;

                Ok(serde_json::to_value(config)?)
            }
            RequestInner::GetProject => {
                let project = params.project.lock().await.clone();
                Ok(serde_json::to_value(project)?)
            }
            RequestInner::SetProject(project) => {
                let mut project_ref = params.project.lock().await;
                *project_ref = project;
                Ok(serde_json::Value::Null)
            }
            RequestInner::SetPhrases(phrases) => {
                let mut phrases_ref = params.phrases.lock().await;
                *phrases_ref = phrases;

                let mut samples = params.voices.lock().await.clone();
                let missing_voices = phrases_ref
                    .iter()
                    .filter_map(|phrase| {
                        if samples.contains_key(&phrase.voice) {
                            None
                        } else {
                            Some(phrase.voice.clone())
                        }
                    })
                    .collect::<HashSet<_>>();
                let unused_voices = samples
                    .keys()
                    .filter(|voice| !phrases_ref.iter().any(|phrase| phrase.voice == **voice))
                    .cloned()
                    .collect::<HashSet<_>>();
                for audio_hash in unused_voices {
                    samples.remove(&audio_hash);
                }
                Ok(serde_json::to_value(SetPhraseResult {
                    missing_voices: missing_voices.into_iter().collect(),
                })?)
            }
            RequestInner::SetVoices(samples) => {
                {
                    let mut samples_ref = params.voices.lock().await;
                    for (audio_hash, sample) in samples {
                        samples_ref.insert(audio_hash, base64.decode(sample)?);
                    }
                }

                let params = Arc::clone(&params);
                let mixes = Arc::clone(&mixes);

                tokio::spawn(async move {
                    Vvvst::update_mixes(params, mixes, None).await;
                });
                Ok(serde_json::Value::Null)
            }
            RequestInner::ShowImportFileDialog(params) => {
                let dialog = match &params {
                    ShowImportFileDialog {
                        title,
                        name: Some(name),
                        filters: Some(filters),
                    } => rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .add_filter(name, filters),
                    ShowImportFileDialog { title, .. } => {
                        rfd::AsyncFileDialog::new().set_title(title)
                    }
                };

                let result = dialog.pick_file().await;
                return Ok(serde_json::to_value(
                    result.map(|path| path.path().to_string_lossy().to_string()),
                )?);
            }
            RequestInner::ReadFile(path) => {
                let content = tokio::fs::read(path).await?;
                let encoded = base64.encode(&content);
                Ok(serde_json::to_value(encoded)?)
            }
            RequestInner::ExportProject => {
                let destination = rfd::AsyncFileDialog::new()
                    .set_title("プロジェクトファイルの書き出し")
                    .add_filter("VOICEVOX Project File", &["vvproj"])
                    .save_file()
                    .await;
                if let Some(destination) = destination {
                    let project = params.project.lock().await.clone();
                    tokio::fs::write(destination.path(), project).await?;
                    return Ok(serde_json::Value::Bool(true));
                } else {
                    return Ok(serde_json::Value::Bool(false));
                }
            }
            RequestInner::ShowMessageDialog(params) => {
                let dialog = rfd::AsyncMessageDialog::new()
                    .set_title(&params.title)
                    .set_description(&params.message)
                    .set_buttons(rfd::MessageButtons::Ok);
                let dialog = match params.r#type {
                    DialogType::Info => dialog.set_level(rfd::MessageLevel::Info),
                    DialogType::Warning => dialog.set_level(rfd::MessageLevel::Warning),
                    DialogType::Error => dialog.set_level(rfd::MessageLevel::Error),
                    _ => dialog,
                };
                dialog.show().await;

                return Ok(serde_json::Value::Null);
            }
            RequestInner::ShowQuestionDialog(params) => {
                anyhow::ensure!(
                    (1..=3).contains(&params.buttons.len()),
                    "The number of buttons must be 1 to 3"
                );
                let dialog = rfd::AsyncMessageDialog::new()
                    .set_title(&params.title)
                    .set_description(&params.message);
                let dialog = match params.r#type {
                    DialogType::Info => dialog.set_level(rfd::MessageLevel::Info),
                    DialogType::Warning => dialog.set_level(rfd::MessageLevel::Warning),
                    DialogType::Error => dialog.set_level(rfd::MessageLevel::Error),
                    _ => dialog,
                };
                let dialog = dialog.set_buttons(match params.buttons.len() {
                    1 => rfd::MessageButtons::OkCustom(params.buttons[0].clone()),
                    2 => rfd::MessageButtons::OkCancelCustom(
                        params.buttons[0].clone(),
                        params.buttons[1].clone(),
                    ),
                    3 => rfd::MessageButtons::YesNoCancelCustom(
                        params.buttons[0].clone(),
                        params.buttons[1].clone(),
                        params.buttons[2].clone(),
                    ),
                    _ => unreachable!(),
                });
                let result = dialog.show().await;
                let rfd::MessageDialogResult::Custom(custom_text) = result else {
                    anyhow::bail!("Unexpected dialog result: {:?}", result);
                };
                return Ok(serde_json::to_value(
                    params
                        .buttons
                        .iter()
                        .position(|button| button == &custom_text),
                )?);
            }
        }
    }

    async fn update_mixes(
        params: Arc<VvvstParams>,
        mixes: Arc<RwLock<Mixes>>,
        new_sample_rate: Option<f32>,
    ) {
        let phrases = params.phrases.lock().await.clone();
        let voices = params.voices.lock().await.clone();
        let mut mixes = mixes.write().await;
        mixes.mixes.clear();
        info!("updating mixes using {} phrases", phrases.len());

        let new_sample_rate = new_sample_rate.unwrap_or(mixes.sample_rate);

        let max_start = phrases
            .iter()
            .map(|phrase| phrase.start)
            .fold(0.0, f32::max);
        let mut mix = vec![0.0; (max_start * new_sample_rate) as usize];
        for phrase in phrases {
            if let Some(voice) = voices.get(&phrase.voice) {
                let mut wav = wav_io::reader::Reader::from_vec(voice.clone()).unwrap();
                let header = wav.read_header().unwrap();
                let base_samples = wav.get_samples_f32().unwrap();
                let samples = if header.channels == 1 {
                    base_samples
                } else {
                    wav_io::utils::stereo_to_mono(base_samples)
                };
                let samples = wav_io::resample::linear(
                    samples,
                    1,
                    header.sample_rate,
                    (new_sample_rate) as u32,
                );
                let start = (phrase.start * new_sample_rate).floor() as isize;
                let end = start + samples.len() as isize;

                if end > mix.len() as isize {
                    mix.resize(end as usize, 0.0);
                }
                for i in 0..samples.len() {
                    let frame = start + i as isize;
                    if frame < 0 {
                        continue;
                    }
                    let frame = frame as usize;
                    if mix[frame] > f32::MAX - samples[i] {
                        mix[frame] = f32::MAX;
                    } else if mix[frame] < f32::MIN - samples[i] {
                        mix[frame] = f32::MIN;
                    } else {
                        mix[frame] += samples[i];
                    }
                }
            }
        }

        info!("mixes updated, {} samples", mix.len());

        mixes.mixes = mix;
        mixes.sample_rate = new_sample_rate;
    }
}

impl Plugin for Vvvst {
    type BackgroundTask = ();
    type SysExMessage = ();

    const NAME: &'static str = "VVVST";
    const VENDOR: &'static str = "Nanashi. <@sevenc-nanashi>";
    const URL: &'static str = "https://github.com/sevenc-nanashi/vvvst-rs";
    const EMAIL: &'static str = "sevenc7c@sevenc7c.com";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(0),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let transport = context.transport();
        if let Ok(mixes) = self.mixes.try_read() {
            if transport.sample_rate == mixes.sample_rate {
                if transport.playing && mixes.mixes.len() > 0 {
                    let current_sample = transport.pos_samples().unwrap() as usize;
                    let samples_len = buffer.samples();
                    let sample_range =
                        current_sample..(current_sample + samples_len).min(mixes.mixes.len());
                    let mix_part_len = sample_range.len();
                    if sample_range.start < mixes.mixes.len() {
                        let slices = buffer.as_slice();
                        if sample_range.len() == samples_len {
                            slices[0].copy_from_slice(&mixes.mixes[sample_range.clone()]);
                            slices[1].copy_from_slice(&mixes.mixes[sample_range]);
                        } else {
                            slices[0][0..mix_part_len]
                                .copy_from_slice(&mixes.mixes[sample_range.clone()]);
                            slices[0][mix_part_len..samples_len].fill(0.0);
                            slices[1][0..mix_part_len].copy_from_slice(&mixes.mixes[sample_range]);
                            slices[1][mix_part_len..samples_len].fill(0.0);
                        }
                    }
                }
            } else {
                RUNTIME.spawn(Vvvst::update_mixes(
                    Arc::clone(&self.params),
                    Arc::clone(&self.mixes),
                    Some(transport.sample_rate),
                ));
            }
        }

        ProcessStatus::Normal
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = Arc::clone(&self.params);
        let response_sender = self.response_sender.clone();
        let response_receiver = self.response_receiver.clone();
        let mixes = Arc::clone(&self.mixes);

        let editor = WebViewEditor::new(
            HTMLSource::URL(if cfg!(debug_assertions) {
                info!("using dev server");
                option_env!("VVVST_DEV_SERVER_URL").unwrap_or("http://localhost:5173")
            } else {
                info!("using bundled editor");
                "app://."
            }),
            (1024, 720),
        )
        .with_custom_protocol("app".to_string(), |request| {
            Ok(EDITOR
                .get_file(request.uri().path())
                .map(|file| {
                    info!("serving file: {:?}", file.path());
                    http::Response::builder()
                        .status(200)
                        .header(
                            "Content-Type",
                            mime_guess::from_path(file.path())
                                .first_or_octet_stream()
                                .as_ref(),
                        )
                        .body(Cow::Borrowed(file.contents()))
                        .unwrap()
                })
                .unwrap_or_else(|| {
                    http::Response::builder()
                        .status(404)
                        .body(Cow::Borrowed(b"" as &[u8]))
                        .unwrap()
                }))
        })
        .with_background_color((165, 212, 173, 255))
        .with_developer_mode(cfg!(debug_assertions))
        .with_keyboard_handler(move |event| {
            println!("keyboard event: {event:#?}");
            event.key == Key::Escape
        })
        .with_mouse_handler(|event| match event {
            MouseEvent::DragEntered { .. } => {
                println!("drag entered");
                EventStatus::AcceptDrop(DropEffect::Copy)
            }
            MouseEvent::DragMoved { .. } => {
                println!("drag moved");
                EventStatus::AcceptDrop(DropEffect::Copy)
            }
            MouseEvent::DragLeft => {
                println!("drag left");
                EventStatus::Ignored
            }
            MouseEvent::DragDropped { data, .. } => {
                if let DropData::Files(files) = data {
                    println!("drag dropped: {:?}", files);
                }
                EventStatus::AcceptDrop(DropEffect::Copy)
            }
            _ => EventStatus::Ignored,
        })
        .with_event_loop(move |ctx, setter, window| {
            while let Ok(value) = ctx.next_event() {
                let value = match serde_json::from_value::<Request>(value.clone()) {
                    Ok(value) => value,
                    Err(err) => {
                        // 可能な限りエラーを返してあげる
                        let request_id = value["requestId"].as_u64();
                        if let Some(request_id) = request_id {
                            let response = Response {
                                request_id: RequestId(request_id as u32),
                                payload: Err(format!("failed to parse request: {}", err)),
                            };
                            warn!("failed to parse request: {}", err);
                            response_sender.send(response).unwrap();
                        } else {
                            error!("failed to parse request: {}", err);
                        }
                        continue;
                    }
                };
                let params = Arc::clone(&params);
                let response_sender = Arc::clone(&response_sender);
                let mixes = Arc::clone(&mixes);

                RUNTIME.spawn(async move {
                    let result = Vvvst::process_request(params, value.inner, mixes).await;
                    let response = Response {
                        request_id: value.request_id,
                        payload: match result {
                            Ok(value) => Ok(value),
                            Err(err) => Err(err.to_string()),
                        },
                    };
                    response_sender.send(response).unwrap();
                });
            }

            while let Ok(response) = response_receiver.lock().unwrap().try_recv() {
                ctx.send_json(serde_json::to_value(response).unwrap())
                    .unwrap();
            }
        });

        Some(Box::new(editor))
    }

    fn deactivate(&mut self) {}
}

impl Vst3Plugin for Vvvst {
    const VST3_CLASS_ID: [u8; 16] = *b"VVVST___________";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Synth, Vst3SubCategory::Instrument];
}

impl ClapPlugin for Vvvst {
    const CLAP_ID: &'static str = "com.sevenc-nanashi.vvvst";
    // const CLAP_ID: &'static str = "jp.hiroshiba.vvvst";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Voicevox for DAW");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/sevenc-nanashi/vvvst-rs");
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::Instrument, ClapFeature::Synthesizer];
}

nih_export_vst3!(Vvvst);
nih_export_clap!(Vvvst);
