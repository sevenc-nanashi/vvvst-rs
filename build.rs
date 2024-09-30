fn main() {
    // ダイアログのメッセージのカスタマイズに必要：https://docs.rs/rfd/latest/rfd/index.html#customize-button-texts-of-message-dialog-in-windows
    #[cfg(target_os = "windows")]
    embed_resource::compile("manifest.rc", embed_resource::NONE);
}
