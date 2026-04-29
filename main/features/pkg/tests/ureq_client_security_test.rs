// Security tests for UreqClient — verifies error handling on hostile/broken endpoints.
use swe_justpkg_pkg::HttpClient;
use swe_justpkg_pkg::UreqClient;

#[test]
fn test_get_bytes_connection_refused_returns_err() {
    // Port 1 on localhost is reserved and guaranteed to refuse connections.
    let client = UreqClient;
    let result = client.get_bytes("http://127.0.0.1:1/test");
    assert!(
        result.is_err(),
        "connection refused must not silently succeed"
    );
}

#[test]
fn test_get_stream_connection_refused_returns_err() {
    let client = UreqClient;
    let mut buf = Vec::new();
    let result = client.get_stream("http://127.0.0.1:1/test", &mut buf);
    assert!(
        result.is_err(),
        "connection refused must not silently succeed"
    );
    assert!(
        buf.is_empty(),
        "no bytes must be written on connection failure"
    );
}
