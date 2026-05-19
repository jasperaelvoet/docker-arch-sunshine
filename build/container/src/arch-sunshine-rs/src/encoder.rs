use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use once_cell::sync::Lazy;

static ELEMENT_CACHE: Lazy<Mutex<BTreeMap<String, bool>>> = Lazy::new(|| Mutex::new(BTreeMap::new()));

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EncoderEntry {
    pub name: String,
    #[serde(default)]
    pub elements: Vec<String>,
    pub pipeline: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PipelineConfig {
    #[serde(default)]
    pub defaults: BTreeMap<String, serde_json::Value>,
    pub video_sources: BTreeMap<String, String>,
    pub payloader: BTreeMap<String, String>,
    pub encoders: BTreeMap<String, Vec<EncoderEntry>>,
}

pub fn load_config(path: &Path) -> Result<PipelineConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("pipeline config not found: {}", path.display()))?;
    let parsed = serde_json::from_str(&text)
        .with_context(|| format!("pipeline config is invalid JSON: {}", path.display()))?;
    Ok(parsed)
}

fn command_available(name: &str) -> bool {
    which(name).is_some()
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn gst_element_exists(name: &str) -> bool {
    if let Some(cached) = ELEMENT_CACHE.lock().unwrap().get(name) {
        return *cached;
    }
    let exists = if !command_available("gst-inspect-1.0") {
        false
    } else {
        Command::new("gst-inspect-1.0")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    ELEMENT_CACHE.lock().unwrap().insert(name.into(), exists);
    exists
}

#[derive(Debug, Clone, Serialize)]
pub struct EncoderStatus {
    pub name: String,
    pub usable: bool,
    pub missing_elements: Vec<String>,
}

pub fn encoder_status(config: &PipelineConfig) -> BTreeMap<String, Vec<EncoderStatus>> {
    let mut out = BTreeMap::new();
    for (codec, encoders) in &config.encoders {
        let entries = encoders
            .iter()
            .map(|enc| {
                let missing: Vec<String> = enc
                    .elements
                    .iter()
                    .filter(|el| !gst_element_exists(el))
                    .cloned()
                    .collect();
                EncoderStatus {
                    name: enc.name.clone(),
                    usable: missing.is_empty(),
                    missing_elements: missing,
                }
            })
            .collect();
        out.insert(codec.clone(), entries);
    }
    out
}

pub fn choose_encoder<'a>(
    config: &'a PipelineConfig,
    codec: &str,
    requested_name: Option<&str>,
) -> Result<&'a EncoderEntry> {
    let encoders = config
        .encoders
        .get(codec)
        .ok_or_else(|| anyhow!("unsupported codec: {codec}"))?;

    let pool: Vec<&EncoderEntry> = if let Some(name) = requested_name {
        let filtered: Vec<&EncoderEntry> = encoders.iter().filter(|e| e.name == name).collect();
        if filtered.is_empty() {
            return Err(anyhow!("encoder {name:?} is not configured for {codec}"));
        }
        filtered
    } else {
        encoders.iter().collect()
    };

    for enc in &pool {
        if enc.elements.iter().all(|el| gst_element_exists(el)) {
            return Ok(*enc);
        }
    }

    let details: Vec<String> = pool
        .iter()
        .map(|enc| {
            let missing: Vec<&String> = enc
                .elements
                .iter()
                .filter(|el| !gst_element_exists(el))
                .collect();
            let missing_str = missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("{} missing {}", enc.name, missing_str)
        })
        .collect();
    Err(anyhow!(
        "no usable {codec} encoder found: {}",
        details.join(", ")
    ))
}

#[derive(Debug, Clone)]
pub struct PipelineRequest<'a> {
    pub codec: &'a str,
    pub encoder: Option<&'a str>,
    pub source: &'a str,
    pub pipewire_node: &'a str,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub vbv_buffer: u32,
    pub payload_mtu: u32,
    pub host: &'a str,
    pub port: u16,
}

pub fn build_pipeline(req: &PipelineRequest, config: &PipelineConfig) -> Result<(String, String)> {
    let enc = choose_encoder(config, req.codec, req.encoder)?;

    let mut values: BTreeMap<String, String> = BTreeMap::new();
    values.insert("width".into(), req.width.to_string());
    values.insert("height".into(), req.height.to_string());
    values.insert("fps".into(), req.fps.to_string());
    values.insert("bitrate_kbps".into(), req.bitrate.to_string());
    values.insert("vbv_buffer_kbits".into(), req.vbv_buffer.to_string());
    values.insert("payload_mtu".into(), req.payload_mtu.to_string());
    values.insert("pipewire_node".into(), req.pipewire_node.to_string());

    for (k, v) in &config.defaults {
        values.entry(k.clone()).or_insert_with(|| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        });
    }

    let source_template = config
        .video_sources
        .get(req.source)
        .ok_or_else(|| anyhow!("unknown source {:?}", req.source))?;
    let payloader_template = config
        .payloader
        .get(req.codec)
        .ok_or_else(|| anyhow!("missing payloader for {}", req.codec))?;

    let source = render_template(source_template, &values);
    let encode = render_template(&enc.pipeline, &values);
    let payloader = render_template(payloader_template, &values);

    let pipeline = format!(
        "{source} ! queue max-size-buffers=2 leaky=downstream ! \
         {encode} ! {payloader} ! \
         queue max-size-buffers=2 leaky=downstream ! \
         udpsink host={host} port={port} sync=false async=false",
        source = source,
        encode = encode,
        payloader = payloader,
        host = req.host,
        port = req.port,
    );
    Ok((enc.name.clone(), pipeline))
}

fn render_template(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut key = String::new();
            while let Some(&next) = chars.peek() {
                if next == '}' {
                    chars.next();
                    break;
                }
                key.push(next);
                chars.next();
            }
            if let Some(v) = values.get(&key) {
                out.push_str(v);
            } else {
                out.push('{');
                out.push_str(&key);
                out.push('}');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct EncoderSummary {
    pub codec: String,
    pub chosen: Option<String>,
}

pub fn chosen_encoder_summary(config: &PipelineConfig) -> Vec<EncoderSummary> {
    ["h264", "hevc", "av1"]
        .iter()
        .map(|codec| EncoderSummary {
            codec: (*codec).into(),
            chosen: choose_encoder(config, codec, None).ok().map(|e| e.name.clone()),
        })
        .collect()
}
