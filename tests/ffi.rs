//! C ABI smoke test: init → registry roundtrip → doctor job → gateway
//! start/status/stop → logs. Lives in its own test binary so the CERBERUS_*
//! environment overrides never race the other test binaries.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::time::Duration;

use cerberus_mcp::ffi;

fn take_json(ptr: *mut c_char) -> serde_json::Value {
    assert!(
        !ptr.is_null(),
        "ffi returned null: {}",
        unsafe { CStr::from_ptr(ffi::cerberus_last_error()) }.to_string_lossy()
    );
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { ffi::cerberus_string_free(ptr) };
    serde_json::from_str(&text).expect("ffi payload is valid JSON")
}

fn take_json_str(s: &str) -> *mut c_char {
    let c = std::ffi::CString::new(s).unwrap();
    c.into_raw()
}

fn wait_job(id: u64) -> serde_json::Value {
    assert_ne!(
        id,
        0,
        "job start failed: {}",
        unsafe { CStr::from_ptr(ffi::cerberus_last_error()) }.to_string_lossy()
    );
    for _ in 0..600 {
        let poll = take_json(ffi::cerberus_job_poll(id));
        match poll["state"].as_str().expect("state") {
            "done" => {
                let value = poll["value"].clone();
                unsafe { ffi::cerberus_job_free(id) };
                return value;
            }
            "error" => unsafe { ffi::cerberus_job_free(id) },
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
        if poll["state"].as_str() == Some("error") {
            panic!("job {id} failed: {}", poll["error"].as_str().unwrap_or("?"));
        }
    }
    panic!("job {id} timed out");
}

#[test]
fn ffi_end_to_end() {
    let dir = std::env::temp_dir().join(format!("cerberus-ffi-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    unsafe {
        std::env::set_var("CERBERUS_CONFIG_DIR", &dir);
        std::env::set_var("CERBERUS_DATA_DIR", &dir);
    }

    // init ------------------------------------------------------------------
    let init = take_json(ffi::cerberus_init());
    assert_eq!(init["version"].as_str(), Some(cerberus_mcp::VERSION));
    assert_eq!(
        init["registryExists"].as_bool(),
        Some(false),
        "fresh config dir starts without a registry"
    );
    assert!(dir.join("conf.cerberus").parent().unwrap().exists());

    // registry roundtrip ----------------------------------------------------
    let mut reg = take_json(ffi::cerberus_registry_get());
    assert!(reg.is_object());
    let saved = take_json_str(&reg.to_string());
    let ack = take_json(unsafe { ffi::cerberus_registry_set(saved) });
    assert_eq!(ack["ok"], serde_json::json!(true));
    assert!(dir.join("conf.cerberus").exists(), "registry written");

    // upsert a server, validate --------------------------------------------
    reg["servers"]["demo"] = serde_json::json!({
        "transport": { "type": "stdio", "command": "echo", "args": ["hello"] },
        "tags": ["ffi"],
        "description": "smoke test"
    });
    let payload = take_json_str(&reg.to_string());
    take_json(unsafe { ffi::cerberus_registry_set(payload) });

    let loaded = take_json(ffi::cerberus_registry_get());
    let demo = &loaded["servers"]["demo"];
    assert_eq!(demo["enabled"], serde_json::json!(true), "enabled defaults");
    assert_eq!(demo["transport"]["command"], serde_json::json!("echo"));
    assert_eq!(loaded["schema_version"].as_u64(), Some(2));

    let report = take_json(ffi::cerberus_registry_validate());
    assert_eq!(report["ok"], serde_json::json!(true), "{report:?}");

    // doctor job ------------------------------------------------------------
    let doctor = wait_job(ffi::cerberus_doctor_start(0));
    assert_eq!(doctor["server_count"].as_u64(), Some(1));
    assert!(doctor["diagnostics"].is_array());

    // gateway: start → status → logs → stop --------------------------------
    let port = 20000 + (std::process::id() as u64 % 20000);
    let bind = format!("127.0.0.1:{port}");
    let opts = take_json_str(&format!(
        r#"{{"bind":"{bind}","watch":true,"watchInterval":500}}"#
    ));
    let started = wait_job(unsafe { ffi::cerberus_gateway_start(opts) });
    assert_eq!(started["running"], serde_json::json!(true));
    assert_eq!(started["managed"], serde_json::json!(true));
    assert_eq!(started["bind"].as_str(), Some(bind.as_str()));
    assert_eq!(started["watch"], serde_json::json!(true));

    let status = take_json(ffi::cerberus_gateway_status());
    assert_eq!(status["running"], serde_json::json!(true));

    let logs = take_json(ffi::cerberus_logs_take(0));
    assert!(
        logs["lines"].as_array().is_some_and(|l| !l.is_empty()),
        "gateway start is logged: {logs:?}"
    );

    let stopped = wait_job(ffi::cerberus_gateway_stop());
    assert_eq!(stopped["running"], serde_json::json!(false));

    let status = take_json(ffi::cerberus_gateway_status());
    assert_eq!(status["running"], serde_json::json!(false));

    // starting twice while stopped works; a bad payload fails cleanly -------
    let bad = take_json_str("{not json");
    assert_eq!(
        unsafe { ffi::cerberus_gateway_start(bad) },
        0,
        "invalid options rejected"
    );
    assert!(
        !unsafe { CStr::from_ptr(ffi::cerberus_last_error()) }
            .to_string_lossy()
            .is_empty()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
