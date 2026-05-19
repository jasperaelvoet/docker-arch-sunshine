use crate::paths::{DEFAULT_FPS, DEFAULT_HEIGHT, DEFAULT_SCALE, DEFAULT_WIDTH};

pub fn sanitize_positive_int(value: &str, fallback: u32, maximum: u32) -> u32 {
    match value.parse::<i64>() {
        Ok(v) if v >= 1 && (v as u64) <= maximum as u64 => v as u32,
        _ => fallback,
    }
}

pub fn sanitize_refresh_rate(value: &str, fallback: &str) -> String {
    let parsed: Option<f64> = value.parse().ok();
    match parsed {
        Some(v) if v > 0.0 && v <= 240.0 => format_float(v),
        _ => fallback.to_string(),
    }
}

pub fn sanitize_audio_channels(value: &str, fallback: u32) -> u32 {
    match value.parse::<u32>() {
        Ok(c) if matches!(c, 2 | 6 | 8) => c,
        _ => fallback,
    }
}

pub fn audio_channels_from_configuration(value: &str, fallback: u32) -> u32 {
    match value.trim() {
        "2.0" => 2,
        "5.1" => 6,
        "7.1" => 8,
        other => sanitize_audio_channels(other, fallback),
    }
}

pub fn requested_audio_channels() -> u32 {
    let fallback_text = std::env::var("SUNSHINE_AUDIO_CHANNELS").unwrap_or_else(|_| "2".into());
    let fallback = sanitize_audio_channels(&fallback_text, 2);
    let raw = std::env::var("SUNSHINE_CLIENT_AUDIO_CONFIGURATION").unwrap_or_default();
    audio_channels_from_configuration(&raw, fallback)
}

pub fn default_audio_channels() -> u32 {
    let raw = std::env::var("SUNSHINE_AUDIO_CHANNELS").unwrap_or_else(|_| "2".into());
    sanitize_audio_channels(&raw, 2)
}

pub fn sanitize_scale_request(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed == "auto" {
        return "auto".into();
    }
    let parsed: Option<f64> = trimmed.parse().ok();
    match parsed {
        Some(v) if (0.5..=4.0).contains(&v) => format_float(v),
        _ => fallback.to_string(),
    }
}

pub fn scale_value(value: &str, fallback: f64) -> f64 {
    match value.parse::<f64>() {
        Ok(v) if v > 0.0 => v,
        _ => fallback,
    }
}

pub fn format_scale(value: f64) -> String {
    format_float(value)
}

fn format_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        let s = format!("{}", value);
        if s.contains('.') {
            s
        } else {
            format!("{}", value)
        }
    }
}

pub fn resolve_session_scale(width: u32, height: u32, requested: &str) -> String {
    let req = sanitize_scale_request(requested, DEFAULT_SCALE);
    if req != "auto" {
        return req;
    }
    if width >= 3840 || height >= 2160 {
        "2".into()
    } else if width >= 2880 || height >= 1800 {
        "1.75".into()
    } else if width >= 2560 || height >= 1440 {
        "1.5".into()
    } else if width >= 1921 || height >= 1200 {
        "1.25".into()
    } else {
        "1".into()
    }
}

pub fn resolve_client_scale(width: u32, height: u32) -> String {
    let user_scale = sanitize_scale_request(
        &std::env::var("SUNSHINE_SCALE").unwrap_or_else(|_| DEFAULT_SCALE.into()),
        DEFAULT_SCALE,
    );
    if user_scale != "auto" {
        return user_scale;
    }
    let auto_scale = resolve_session_scale(width, height, DEFAULT_SCALE);
    let client_scale = sanitize_scale_request(
        &std::env::var("SUNSHINE_CLIENT_SCALE").unwrap_or_else(|_| DEFAULT_SCALE.into()),
        DEFAULT_SCALE,
    );
    if client_scale == "auto" {
        return auto_scale;
    }
    let max = scale_value(&auto_scale, 1.0).max(scale_value(&client_scale, 1.0));
    format_scale(max)
}

pub fn kwin_virtual_geometry(width: u32, height: u32, scale: &str) -> (u32, u32) {
    let factor = scale_value(scale, 1.0);
    let logical_w = ((width as f64) / factor).round().max(1.0) as u32;
    let logical_h = ((height as f64) / factor).round().max(1.0) as u32;
    (logical_w, logical_h)
}

pub fn plasma_scale_dpi(scale: &str) -> u32 {
    let v = scale_value(scale, 1.0);
    ((96.0 * v).round() as i64).max(1) as u32
}

pub fn plasma_screen_scale_factors(scale: &str) -> String {
    let text = format_scale(scale_value(scale, 1.0));
    crate::paths::KWIN_OUTPUT_NAMES
        .iter()
        .map(|name| format!("{name}={text};"))
        .collect::<String>()
}

pub fn default_geometry() -> (u32, u32, String, String) {
    let width = sanitize_positive_int(
        &std::env::var("SUNSHINE_WIDTH").unwrap_or_else(|_| DEFAULT_WIDTH.to_string()),
        DEFAULT_WIDTH,
        8192,
    );
    let height = sanitize_positive_int(
        &std::env::var("SUNSHINE_HEIGHT").unwrap_or_else(|_| DEFAULT_HEIGHT.to_string()),
        DEFAULT_HEIGHT,
        8192,
    );
    let fps = sanitize_refresh_rate(
        &std::env::var("SUNSHINE_FPS").unwrap_or_else(|_| DEFAULT_FPS.into()),
        DEFAULT_FPS,
    );
    let scale_input = std::env::var("SUNSHINE_SCALE").unwrap_or_else(|_| DEFAULT_SCALE.into());
    let scale = resolve_session_scale(width, height, &scale_input);
    (width, height, fps, scale)
}

pub fn requested_client_geometry() -> (u32, u32, String, String) {
    let (dw, dh, df, _ds) = default_geometry();
    let width = sanitize_positive_int(
        &std::env::var("SUNSHINE_CLIENT_WIDTH").unwrap_or_else(|_| dw.to_string()),
        dw,
        8192,
    );
    let height = sanitize_positive_int(
        &std::env::var("SUNSHINE_CLIENT_HEIGHT").unwrap_or_else(|_| dh.to_string()),
        dh,
        8192,
    );
    let fps = sanitize_refresh_rate(
        &std::env::var("SUNSHINE_CLIENT_FPS").unwrap_or_else(|_| df.clone()),
        &df,
    );
    let scale = resolve_client_scale(width, height);
    (width, height, fps, scale)
}
