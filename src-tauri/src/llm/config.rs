use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub mode: String,
    pub api_base_url: String,
    pub api_key: String,
    pub default_model: String,
    pub text_mode: String,
    pub image_endpoint: String,
    pub image_key: String,
    pub image_model: String,
    pub review_threshold: u8,
    pub enable_local_save: bool,
}
