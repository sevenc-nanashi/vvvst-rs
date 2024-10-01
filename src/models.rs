use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SingingVoiceKey(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub u32);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub request_id: RequestId,
    pub payload: Result<Value, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub request_id: RequestId,
    pub inner: RequestInner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "payload")]
pub enum RequestInner {
    GetVersion,
    GetProjectName,

    GetConfig,
    GetProject,
    SetProject(String),
    SetPhrases(Vec<Phrase>),
    SetVoices(HashMap<SingingVoiceKey, String>),

    ShowMessageDialog(ShowMessageDialog),
    ShowImportFileDialog(ShowImportFileDialog),
    ShowQuestionDialog(ShowQuestionDialog),

    ReadFile(String),

    ExportProject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowImportFileDialog {
    pub title: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub filters: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phrase {
    pub start: f32,
    pub voice: SingingVoiceKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPhraseResult {
    pub missing_voices: Vec<SingingVoiceKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowMessageDialog {
    pub r#type: DialogType,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogType {
    None,
    Info,
    Warning,
    Error,
    Question,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowQuestionDialog {
    pub r#type: DialogType,
    pub title: String,
    pub message: String,
    pub buttons: Vec<String>,
    #[serde(default)]
    pub cancel_id: Option<usize>,
    #[serde(default)]
    pub default_id: Option<usize>,
}
