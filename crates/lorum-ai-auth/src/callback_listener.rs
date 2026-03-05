use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use url::Url;

use crate::{OAuthCallbackError, OAuthCallbackFlow};

const MAX_RETRYABLE_CALLBACK_ATTEMPTS: u32 = 3;
const CALLBACK_STREAM_READ_TIMEOUT_MS: u64 = 250;
const CALLBACK_STREAM_READ_MAX_WAIT_SECS: u64 = 5;

#[derive(Debug)]
pub enum CallbackResult {
    Code(String),
    NotLocal,
    Timeout,
    BindFailed,
    Error(String),
}

pub struct LocalCallbackListener {
    timeout: Duration,
    max_retryable_attempts: u32,
}

impl LocalCallbackListener {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            max_retryable_attempts: MAX_RETRYABLE_CALLBACK_ATTEMPTS,
        }
    }

    pub fn wait_for_code(&self, redirect_uri: &str, expected_state: &str) -> CallbackResult {
        let redirect = match Url::parse(redirect_uri) {
            Ok(url) => url,
            Err(_) => return CallbackResult::NotLocal,
        };

        if !is_local_redirect_uri(&redirect) {
            return CallbackResult::NotLocal;
        }

        let host = match redirect.host_str() {
            Some(h) => h,
            None => return CallbackResult::NotLocal,
        };
        let port = match redirect.port_or_known_default() {
            Some(p) => p,
            None => return CallbackResult::NotLocal,
        };
        let bind_host = if host == "localhost" {
            "127.0.0.1"
        } else {
            host
        };

        let listener = match TcpListener::bind((bind_host, port)) {
            Ok(l) => l,
            Err(_) => return CallbackResult::BindFailed,
        };
        if let Err(err) = listener.set_nonblocking(true) {
            return CallbackResult::Error(format!("set nonblocking failed: {err}"));
        }

        let deadline = Instant::now() + self.timeout;
        let callback_flow = OAuthCallbackFlow::new(0, self.timeout.as_secs());
        let mut retryable_attempts = 0_u32;

        loop {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    if let Err(err) = stream.set_nonblocking(false) {
                        return CallbackResult::Error(format!(
                            "set callback stream blocking mode failed: {err}"
                        ));
                    }
                    if let Err(err) = stream.set_read_timeout(Some(Duration::from_millis(
                        CALLBACK_STREAM_READ_TIMEOUT_MS,
                    ))) {
                        return CallbackResult::Error(format!(
                            "set callback stream read timeout failed: {err}"
                        ));
                    }

                    let request = match read_http_request_message(&mut stream) {
                        Ok(r) => r,
                        Err(err) => return CallbackResult::Error(err),
                    };

                    let Some(target) = parse_http_request_target(&request) else {
                        let _ = stream.write_all(
                            b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\ninvalid callback request",
                        );
                        continue;
                    };

                    let callback_url = match callback_url_from_request_target(&redirect, target) {
                        Ok(value) => value,
                        Err(_) => {
                            let _ = stream.write_all(
                                b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\ninvalid callback target",
                            );
                            continue;
                        }
                    };

                    let code_result =
                        match callback_flow.parse_callback_url(&callback_url, expected_state) {
                            Ok(code) => Ok(code),
                            Err(err) => {
                                match parse_oauth_code_from_form_body(&request, expected_state) {
                                    Ok(Some(code)) => Ok(code),
                                    Ok(None) => Err(err),
                                    Err(form_err) => Err(form_err),
                                }
                            }
                        };

                    match code_result {
                        Ok(code) => {
                            let _ = stream.write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOAuth login complete. Return to terminal.",
                            );
                            return CallbackResult::Code(code);
                        }
                        Err(err) => {
                            if callback_error_is_retryable(&err) {
                                let _ = stream.write_all(
                                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOAuth callback received but incomplete. Continue login flow and return to terminal.",
                                );
                                retryable_attempts += 1;
                                if retryable_attempts >= self.max_retryable_attempts {
                                    return CallbackResult::Timeout;
                                }
                                continue;
                            }
                            let _ = stream.write_all(
                                b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOAuth callback rejected. Return to terminal.",
                            );
                            if matches!(
                                err,
                                OAuthCallbackError::AuthorizationFailed { .. }
                            ) {
                                return CallbackResult::Error(format!(
                                    "oauth authorization failed: {err}"
                                ));
                            }
                            return CallbackResult::Error(format!(
                                "callback validation failed: {err}"
                            ));
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return CallbackResult::Timeout;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(err) => {
                    return CallbackResult::Error(format!("accept callback failed: {err}"));
                }
            }
        }
    }

    pub fn bind_host_port(redirect_uri: &str) -> Option<(String, u16)> {
        let redirect = Url::parse(redirect_uri).ok()?;
        if !is_local_redirect_uri(&redirect) {
            return None;
        }
        let host = redirect.host_str()?;
        let port = redirect.port_or_known_default()?;
        let bind_host = if host == "localhost" {
            "127.0.0.1".to_string()
        } else {
            host.to_string()
        };
        Some((bind_host, port))
    }
}

fn read_http_request_message(stream: &mut std::net::TcpStream) -> Result<String, String> {
    const MAX_REQUEST_BYTES: usize = 64 * 1024;
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 1024];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0_usize;
    let read_deadline = Instant::now() + Duration::from_secs(CALLBACK_STREAM_READ_MAX_WAIT_SECS);

    loop {
        let bytes_read = match stream.read(&mut chunk) {
            Ok(bytes) => bytes,
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                if Instant::now() >= read_deadline {
                    return Err(format!("read callback request timed out: {err}"));
                }
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(err) => return Err(format!("read callback request failed: {err}")),
        };

        if bytes_read == 0 {
            if Instant::now() >= read_deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        buf.extend_from_slice(&chunk[..bytes_read]);
        if buf.len() > MAX_REQUEST_BYTES {
            return Err("callback request exceeded max size".to_string());
        }

        if header_end.is_none() {
            if let Some(end) = find_http_header_end(&buf) {
                header_end = Some(end);
                let headers = String::from_utf8_lossy(&buf[..end]);
                content_length = parse_http_content_length(&headers).unwrap_or(0);
            }
        }

        if let Some(end) = header_end {
            if buf.len() >= end + content_length {
                break;
            }
        }
    }

    if buf.is_empty() {
        return Err("callback request was empty".to_string());
    }

    if let Some(end) = header_end {
        if buf.len() < end + content_length {
            return Err("callback request body was incomplete".to_string());
        }
    }

    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn find_http_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

fn parse_http_content_length(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            if let Ok(parsed) = value.trim().parse::<usize>() {
                return Some(parsed);
            }
        }
    }
    None
}

fn parse_oauth_code_from_form_body(
    request: &str,
    expected_state: &str,
) -> Result<Option<String>, OAuthCallbackError> {
    if parse_http_request_method(request) != Some("POST") {
        return Ok(None);
    }

    let body = parse_http_request_body(request);
    if body.trim().is_empty() {
        return Ok(None);
    }

    let params: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    if params.is_empty() {
        return Ok(None);
    }
    if let Some(error) = params.get("error").cloned() {
        let description = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| "authorization failed".to_string());
        return Err(OAuthCallbackError::AuthorizationFailed { error, description });
    }
    if !params.contains_key("code") && !params.contains_key("state") {
        return Ok(None);
    }

    let code = params
        .get("code")
        .cloned()
        .ok_or(OAuthCallbackError::MissingCode)?;
    let state = params
        .get("state")
        .cloned()
        .ok_or(OAuthCallbackError::MissingState)?;
    if state != expected_state {
        return Err(OAuthCallbackError::StateMismatch);
    }

    Ok(Some(code))
}

fn callback_error_is_retryable(err: &OAuthCallbackError) -> bool {
    matches!(
        err,
        OAuthCallbackError::InvalidUrl
            | OAuthCallbackError::MissingCode
            | OAuthCallbackError::MissingState
            | OAuthCallbackError::StateMismatch
    )
}

fn parse_http_request_method(request: &str) -> Option<&str> {
    request.lines().next()?.split_whitespace().next()
}

fn parse_http_request_body(request: &str) -> &str {
    request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("")
}

fn parse_http_request_target(request: &str) -> Option<&str> {
    request.lines().next()?.split_whitespace().nth(1)
}

pub fn callback_url_from_request_target(redirect: &Url, target: &str) -> Result<String, String> {
    let normalized_target = target.trim();
    if normalized_target.is_empty() {
        return Err("request target was empty".to_string());
    }

    if normalized_target.starts_with("http://") || normalized_target.starts_with("https://") {
        Url::parse(normalized_target).map_err(|err| err.to_string())?;
        return Ok(normalized_target.to_string());
    }

    let host = redirect
        .host_str()
        .ok_or_else(|| "redirect URI missing host".to_string())?;
    let port = redirect
        .port_or_known_default()
        .ok_or_else(|| "redirect URI missing port".to_string())?;
    let path = if normalized_target.starts_with('/') {
        normalized_target.to_string()
    } else {
        format!("/{normalized_target}")
    };

    Ok(format!(
        "{}://{}:{}{}",
        redirect.scheme(),
        host,
        port,
        path
    ))
}

pub fn is_local_redirect_uri(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }

    matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_redirect_uri_detection_is_strict() {
        assert!(is_local_redirect_uri(
            &Url::parse("http://127.0.0.1:1455/callback").expect("parse localhost")
        ));
        assert!(is_local_redirect_uri(
            &Url::parse("http://localhost:1455/callback").expect("parse localhost")
        ));
        assert!(!is_local_redirect_uri(
            &Url::parse("https://localhost:1455/callback").expect("parse https")
        ));
        assert!(!is_local_redirect_uri(
            &Url::parse("http://example.com:1455/callback").expect("parse remote")
        ));
    }

    #[test]
    fn callback_url_from_origin_form_target_is_constructed() {
        let redirect = Url::parse("http://127.0.0.1:1455/callback").expect("parse redirect");
        let callback_url =
            callback_url_from_request_target(&redirect, "/callback?code=abc&state=s1")
                .expect("construct callback url");
        assert_eq!(
            callback_url,
            "http://127.0.0.1:1455/callback?code=abc&state=s1"
        );
    }

    #[test]
    fn callback_url_from_absolute_form_target_is_passed_through() {
        let redirect = Url::parse("http://127.0.0.1:1455/callback").expect("parse redirect");
        let callback_url = callback_url_from_request_target(
            &redirect,
            "http://localhost:1455/callback?code=abc&state=s1",
        )
        .expect("pass through absolute callback target");
        assert_eq!(
            callback_url,
            "http://localhost:1455/callback?code=abc&state=s1"
        );
    }

    #[test]
    fn callback_error_retryability_covers_expected_callback_parse_errors() {
        assert!(callback_error_is_retryable(&OAuthCallbackError::InvalidUrl));
        assert!(callback_error_is_retryable(
            &OAuthCallbackError::MissingCode
        ));
        assert!(callback_error_is_retryable(
            &OAuthCallbackError::MissingState
        ));
        assert!(callback_error_is_retryable(
            &OAuthCallbackError::StateMismatch
        ));
        assert!(!callback_error_is_retryable(
            &OAuthCallbackError::MissingManualCode
        ));
        assert!(!callback_error_is_retryable(
            &OAuthCallbackError::AuthorizationFailed {
                error: "invalid_scope".to_string(),
                description: "scope rejected".to_string(),
            }
        ));
    }

    #[test]
    fn parse_oauth_code_from_form_body_supports_form_post_callback() {
        let request = concat!(
            "POST /callback HTTP/1.1\r\n",
            "Host: 127.0.0.1:1455\r\n",
            "Content-Type: application/x-www-form-urlencoded\r\n",
            "Content-Length: 23\r\n",
            "\r\n",
            "code=abc&state=s1"
        );

        let code = parse_oauth_code_from_form_body(request, "s1")
            .expect("parse form post callback")
            .expect("form post should include code");
        assert_eq!(code, "abc");
    }

    #[test]
    fn parse_oauth_code_from_form_body_rejects_state_mismatch() {
        let request = concat!(
            "POST /callback HTTP/1.1\r\n",
            "Host: 127.0.0.1:1455\r\n",
            "Content-Type: application/x-www-form-urlencoded\r\n",
            "Content-Length: 23\r\n",
            "\r\n",
            "code=abc&state=bad"
        );

        let err = parse_oauth_code_from_form_body(request, "s1")
            .expect_err("mismatched form-post state must fail");
        assert_eq!(err, OAuthCallbackError::StateMismatch);
    }

    #[test]
    fn parse_oauth_code_from_form_body_reports_authorization_failure() {
        let request = concat!(
            "POST /callback HTTP/1.1\r\n",
            "Host: 127.0.0.1:1455\r\n",
            "Content-Type: application/x-www-form-urlencoded\r\n",
            "Content-Length: 65\r\n",
            "\r\n",
            "error=invalid_scope&error_description=scope+rejected&state=s1"
        );

        let err = parse_oauth_code_from_form_body(request, "s1")
            .expect_err("authorization failure must be surfaced");
        assert_eq!(
            err,
            OAuthCallbackError::AuthorizationFailed {
                error: "invalid_scope".to_string(),
                description: "scope rejected".to_string(),
            }
        );
    }
}
