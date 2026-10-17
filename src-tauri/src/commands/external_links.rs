use crate::commands::error::CommandResult;

fn validate_external_url(raw: &str) -> Result<String, String> {
    if raw.len() > 8_192 {
        return Err("external URL is too long".to_string());
    }
    let parsed =
        reqwest::Url::parse(raw.trim()).map_err(|err| format!("invalid external URL: {err}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("external URL must use http or https".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("external URL must include a host".to_string());
    }
    Ok(parsed.to_string())
}

#[tauri::command]
pub fn open_external_url(url: String) -> CommandResult<()> {
    let validated = validate_external_url(&url)?;
    webbrowser::open(&validated).map_err(|err| format!("open browser failed: {err}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_external_url;

    #[test]
    fn accepts_only_absolute_web_urls() {
        assert_eq!(
            validate_external_url("https://www.example.com/path").as_deref(),
            Ok("https://www.example.com/path")
        );
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("mailto:mara@example.com").is_err());
        assert!(validate_external_url("/relative/path").is_err());
    }
}
