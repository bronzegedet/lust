pub fn http_get(url: &str) -> String {
    reqwest::blocking::get(url)
        .and_then(|r| r.text())
        .unwrap_or_else(|e| format!("HTTP error: {}", e))
}
