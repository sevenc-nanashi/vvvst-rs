mod models;
mod utils;
use base64::{engine::general_purpose::STANDARD as base64, Engine as _};
use include_dir::{include_dir, Dir};
use nih_plug::prelude::*;
use nih_plug_webview::*;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, Mutex};
use tokio::runtime::Runtime;
use utils::TokioMutexParam;

use models::*;

pub static RUNTIME: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new().expect("Failed to create runtime"));

static EDITOR: Dir = include_dir!("$CARGO_MANIFEST_DIR/editor");

struct Vvvst {
    params: Arc<VvvstParams>,

    // 一瞬で終わるのでstdのMutexで十分...のはず？
    response_receiver: Arc<Mutex<std::sync::mpsc::Receiver<Response>>>,

    response_sender: Arc<std::sync::mpsc::Sender<Response>>,
}

impl Default for Vvvst {
    fn default() -> Self {
        let (response_sender, response_receiver) = std::sync::mpsc::channel();
        Self {
            params: Arc::new(VvvstParams::default()),
            response_sender: Arc::new(response_sender),
            response_receiver: Arc::new(Mutex::new(response_receiver)),
        }
    }
}

#[derive(Params, Default)]
struct VvvstParams {
    #[persist = "samples"]
    samples: TokioMutexParam<BTreeMap<AudioHash, Vec<f32>>>,
    #[persist = "phrases"]
    phrases: TokioMutexParam<Vec<Phrase>>,
    #[persist = "project"]
    project: TokioMutexParam<String>,
}

impl Vvvst {
    async fn process_request(
        params: Arc<VvvstParams>,
        request: RequestInner,
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

                let mut samples = params.samples.lock().await.clone();
                let missing_audio_hashes = phrases_ref
                    .iter()
                    .filter_map(|phrase| {
                        if samples.get(&phrase.audio_hash).is_some() {
                            None
                        } else {
                            Some(phrase.audio_hash)
                        }
                    })
                    .collect::<BTreeSet<_>>();
                let unused_audio_hashes = samples
                    .keys()
                    .filter(|audio_hash| {
                        !phrases_ref
                            .iter()
                            .any(|phrase| phrase.audio_hash == **audio_hash)
                    })
                    .cloned()
                    .collect::<BTreeSet<_>>();
                for audio_hash in unused_audio_hashes {
                    samples.remove(&audio_hash);
                }
                Ok(serde_json::to_value(SetPhraseResult {
                    missing_audio_hashes: missing_audio_hashes.into_iter().collect(),
                })?)
            }
            RequestInner::SetSamples(samples) => {
                let mut samples_ref = params.samples.lock().await;
                for (audio_hash, sample) in samples {
                    samples_ref.insert(audio_hash, sample);
                }
                Ok(serde_json::Value::Null)
            }
            RequestInner::ShowImportFileDialog(params) => {
                let dialog = rfd::AsyncFileDialog::new().set_title(&params.title);
                let dialog = if params.name.is_some() && params.filters.is_some() {
                    dialog.add_filter(&params.name.unwrap(), &params.filters.unwrap())
                } else {
                    dialog
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
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = Arc::clone(&self.params);
        let response_sender = self.response_sender.clone();
        let response_receiver = self.response_receiver.clone();

        let editor = WebViewEditor::new(
            HTMLSource::URL(if cfg!(debug_assertions) {
                option_env!("VVVST_DEV_SERVER_URL").unwrap_or("http://localhost:5173")
            } else {
                "app://."
            }),
            (1024, 720),
        )
        .with_custom_protocol("app".to_string(), |request| {
            Ok(EDITOR
                .get_file(request.uri().path())
                .map(|file| {
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
                            response_sender.send(response).unwrap();
                        }
                        continue;
                    }
                };
                let params = Arc::clone(&params);
                let response_sender = Arc::clone(&response_sender);

                RUNTIME.spawn(async move {
                    let result = Vvvst::process_request(params, value.inner).await;
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

            loop {
                let response = { response_receiver.lock().unwrap().try_recv() };
                if let Ok(response) = response {
                    nih_plug::debug::nih_log!("response: {:?}", response);
                    ctx.send_json(serde_json::to_value(response).unwrap())
                        .unwrap();
                } else {
                    break;
                }
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
