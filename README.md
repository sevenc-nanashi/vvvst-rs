# vvvst-rs

VVVSTのRust版。nih-plug、nih-plug-webview。
エディタ：<https://github.com/sevenc-nanashi/voicevox/tree/add/vst>

TODO: ちゃんと書く

## ビルド方法

```
cargo xtask bundle vvvst-rs
```

## 環境変数

ビルド時に設定する。
- `VVVST_LOG`：設定すると`./logs`下にログが出力される。
- `VVVST_DEV_SERVER_URL`：開発用サーバーのURL。デフォルトは`http://localhost:5173`。
