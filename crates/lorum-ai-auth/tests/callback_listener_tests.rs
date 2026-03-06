use url::Url;

use lorum_ai_auth::callback_listener::{
    callback_error_is_retryable, callback_url_from_request_target, is_local_redirect_uri,
    parse_oauth_code_from_form_body,
};
use lorum_ai_auth::OAuthCallbackError;

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
