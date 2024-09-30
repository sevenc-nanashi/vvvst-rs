use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct AudioHash(pub u32);

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
    SetSamples(BTreeMap<AudioHash, Vec<f32>>),
    ShowImportFileDialog(ShowImportFileDialog),
    ReadFile(String),
    ExportProject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowImportFileDialog {
    pub title: String,
    pub filters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phrase {
    pub start_at: f32,
    pub duration: f32,
    pub audio_hash: AudioHash,
}
