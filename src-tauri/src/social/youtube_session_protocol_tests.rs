//! Pure protocol boundary tests. Network and persistence flows are exercised in
//! the parent module's mock-transport tests.

use super::*;

#[test]
fn session_address_only_accepts_the_youtube_upload_origin_and_endpoint() {
    for uri in [
        INIT_URL,
        "https://www.googleapis.com/upload/youtube/v3/videos?upload_id=opaque-session",
        "https://www.googleapis.com:443/upload/youtube/v3/videos?uploadType=resumable&upload_id=opaque-session",
    ] {
        assert!(validate_session_uri(uri).is_ok(), "valid upload address rejected");
    }

    let rejected = [
        "http://www.googleapis.com/upload/youtube/v3/videos?upload_id=secret",
        "https://www.googleapis.com.evil.invalid/upload/youtube/v3/videos?upload_id=secret",
        "https://www.googleapis.com@evil.invalid/upload/youtube/v3/videos?upload_id=secret",
        "https://user@www.googleapis.com/upload/youtube/v3/videos?upload_id=secret",
        "https://user:password@www.googleapis.com/upload/youtube/v3/videos?upload_id=secret",
        "https://www.googleapis.com:8443/upload/youtube/v3/videos?upload_id=secret",
        "https://www.googleapis.com/youtube/v3/videos?upload_id=secret",
        "https://www.googleapis.com/upload/youtube/v3/videos/other?upload_id=secret",
        "https://www.googleapis.com/upload/youtube/v3/videos?upload_id=secret#fragment",
        "/upload/youtube/v3/videos?upload_id=secret",
        "not a URL containing secret",
    ];
    for uri in rejected {
        let error = validate_session_uri(uri).unwrap_err().to_string();
        assert!(
            !error.contains("secret"),
            "session credentials reached an error"
        );
        assert!(!error.contains(uri), "raw session address reached an error");
    }
    let oversized = format!(
        "https://www.googleapis.com/upload/youtube/v3/videos?upload_id={}",
        "x".repeat(8192)
    );
    assert!(validate_session_uri(&oversized).is_err());
}

#[test]
fn accepted_ranges_use_the_inclusive_server_end_plus_one() {
    assert_eq!(acknowledged_offset(None, true, 0, 100, 100).unwrap(), 0);
    assert_eq!(
        acknowledged_offset(Some("bytes=0-9"), true, 0, 100, 100).unwrap(),
        10
    );
    assert_eq!(
        acknowledged_offset(Some(" bytes=0-99 "), true, 40, 100, 100).unwrap(),
        100
    );
    assert_eq!(
        acknowledged_offset(Some("bytes=0-49"), false, 20, 50, 100).unwrap(),
        50
    );
    assert_eq!(
        acknowledged_offset(Some("bytes=0-99"), false, 50, 100, 100).unwrap(),
        100
    );
}

#[test]
fn invalid_or_nonprogressing_ranges_cannot_authorize_more_bytes() {
    for range in [
        "",
        "0-9",
        "bytes=5-9",
        "bytes=0-",
        "bytes=0--1",
        "bytes=0-1.0",
        "bytes=0-1e1",
        "bytes=0-9,10-19",
        "bytes=0-18446744073709551615",
        "bytes=0-18446744073709551616",
    ] {
        assert!(
            acknowledged_offset(Some(range), true, 0, 100, 100).is_err(),
            "malformed range accepted: {range}"
        );
    }
    // Missing Range only means zero received on an initial status probe.
    assert!(acknowledged_offset(None, false, 0, 10, 100).is_err());
    assert!(acknowledged_offset(None, true, 10, 100, 100).is_err());
    // Never rewind a previously acknowledged position or resend forever.
    assert!(acknowledged_offset(Some("bytes=0-8"), true, 10, 100, 100).is_err());
    assert!(acknowledged_offset(Some("bytes=0-9"), false, 10, 20, 100).is_err());
    // A chunk response cannot acknowledge unsent bytes or exceed the file.
    assert!(acknowledged_offset(Some("bytes=0-10"), false, 0, 10, 100).is_err());
    assert!(acknowledged_offset(Some("bytes=0-100"), true, 0, 100, 100).is_err());
}

fn completion_response(body: &[u8]) -> WireResponse {
    WireResponse {
        status: 201,
        location: None,
        range: None,
        retry_after: None,
        body: body.to_vec(),
    }
}

#[test]
fn completion_requires_a_valid_video_id_and_never_echoes_response_secrets() {
    assert_eq!(
        video_id(&completion_response(br#"{"id":"AbCdEf123_-"}"#)).unwrap(),
        "AbCdEf123_-"
    );
    for body in [
        "",
        "not JSON",
        "{}",
        "[]",
        "{\"id\":123}",
        "{\"id\":\"short\"}",
        "{\"id\":\"AbCdEf123_-X\"}",
        "{\"id\":\"AbCdEf123/1\"}",
        "{\"id\":\"AbCdEf123 1\"}",
        "{\"id\":\"https://www.googleapis.com/upload/youtube/v3/videos?upload_id=secret\"}",
    ] {
        let error = video_id(&completion_response(body.as_bytes()))
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret"));
        assert!(!error.contains("upload_id"));
    }
}

#[test]
fn retry_after_honors_seconds_and_http_dates_with_bounded_fallbacks() {
    assert_eq!(retry_deadline(Some("120"), 1000, 5), 1120);
    assert_eq!(retry_deadline(Some(" 0 "), 1000, 5), 1000);
    assert_eq!(
        retry_deadline(Some("Wed, 21 Oct 2015 07:28:00 GMT"), 1_445_412_000, 5),
        1_445_412_480
    );
    assert_eq!(
        retry_deadline(Some("Wed, 21 Oct 2015 07:28:00 GMT"), 1_500_000_000, 5),
        1_500_000_000
    );
    for header in [
        None,
        Some("invalid"),
        Some("-1"),
        Some("18446744073709551616"),
    ] {
        assert_eq!(retry_deadline(header, 1000, 5), 1005);
    }
    assert_eq!(
        retry_deadline(Some("18446744073709551615"), 1000, 5),
        i64::MAX
    );
    assert_eq!(retry_deadline(None, i64::MAX - 1, 5), i64::MAX);
}
