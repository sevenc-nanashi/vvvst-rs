use nih_plug::prelude::*;
use nih_plug_webview::*;
use std::sync::Arc;

struct Vvvst {
    params: Arc<VvvstParams>,
}

impl Default for Vvvst {
    fn default() -> Self {
        Self {
            params: Arc::new(VvvstParams::default()),
        }
    }
}

#[derive(Params, Default)]
struct VvvstParams {}

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
        let editor = WebViewEditor::new(HTMLSource::String("<h1>test</h1>"), (200, 200))
            .with_background_color((150, 150, 150, 255))
            .with_developer_mode(true)
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
                    dbg!(&value);
                }
            });

        Some(Box::new(editor))
    }

    fn deactivate(&mut self) {}
}

impl Vst3Plugin for Vvvst {
    const VST3_CLASS_ID: [u8; 16] = *b"VoicevoxForVst3\0";
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
