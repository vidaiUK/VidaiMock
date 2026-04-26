use super::*;

fn write_provider(path: &std::path::Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
            path,
            format!(
                "name: \"openai\"\nmatcher: \"^/v1/chat/completions$\"\nresponse_body: '{}'\npriority: 100\n",
                body.replace('\'', "''")
            ),
        )
        .unwrap();
}

fn write_streaming_provider(path: &std::path::Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
            path,
            format!(
                "name: \"openai\"\nmatcher: \"^/v1/chat/completions$\"\nresponse_body: '{}'\nstream:\n  enabled: true\npriority: 100\n",
                body.replace('\'', "''")
            ),
        )
        .unwrap();
}

fn write_tenant_metadata(base_dir: &std::path::Path, tenant_id: &str, body: &str) {
    let metadata_path = base_dir.join("tenants").join(tenant_id).join("tenant.toml");
    fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
    fs::write(metadata_path, body).unwrap();
}

fn multi_tenant_config(base_dir: &std::path::Path) -> AppConfig {
    write_tenant_metadata(
        base_dir,
        "acme",
        r#"
id = "acme"
"#,
    );
    write_tenant_metadata(
        base_dir,
        "globex",
        r#"
id = "globex"
"#,
    );

    AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        workers: 1,
        log_level: "debug".to_string(),
        config_dir: PathBuf::from("config"),
        tenancy: TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: base_dir.join("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig::default(),
        },
        latency: crate::config::LatencyConfig::default(),
        chaos: crate::config::ChaosConfig::default(),
        endpoints: vec![crate::config::EndpointConfig {
            path: "/v1/chat/completions".to_string(),
            format: "openai".to_string(),
            content_type: None,
        }],
        response_file: None,
        reload_args: None,
    }
}

fn managed_multi_tenant_config(base_dir: &std::path::Path) -> AppConfig {
    let secret_dir = base_dir.join("secrets");
    fs::create_dir_all(&secret_dir).unwrap();
    let acme_key_path = secret_dir.join("acme.key");
    let acme_admin_key_path = secret_dir.join("acme-admin.key");
    fs::write(&acme_key_path, "secret-acme").unwrap();
    fs::write(&acme_admin_key_path, "tenant-admin-acme").unwrap();
    write_tenant_metadata(
        base_dir,
        "acme",
        &format!(
            r#"
id = "acme"
display_name = "Acme Corp"

[labels]
tier = "gold"
region = "eu-west"

[[keys]]
source = "header"
name = "x-api-key"
value_file = "{}"

[management_auth]
header = "x-tenant-admin-key"
value_file = "{}"
"#,
            toml_path(&acme_key_path),
            toml_path(&acme_admin_key_path)
        ),
    );
    write_tenant_metadata(
        base_dir,
        "globex",
        r#"
id = "globex"
display_name = "Globex"

[labels]
tier = "silver"
region = "us-east"

[[keys]]
source = "header"
name = "x-api-key"
value = "secret-globex"

[management_auth]
header = "x-tenant-admin-key"
value = "tenant-admin-globex"
"#,
    );

    AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        workers: 1,
        log_level: "debug".to_string(),
        config_dir: PathBuf::from("config"),
        tenancy: TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: base_dir.join("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig {
                header: "x-admin-key".to_string(),
                value: "global-admin-secret".to_string(),
                value_file: None,
                value_env: None,
            },
        },
        latency: crate::config::LatencyConfig::default(),
        chaos: crate::config::ChaosConfig::default(),
        endpoints: vec![crate::config::EndpointConfig {
            path: "/v1/chat/completions".to_string(),
            format: "openai".to_string(),
            content_type: None,
        }],
        response_file: None,
        reload_args: None,
    }
}

fn unmanaged_admin_multi_tenant_config(base_dir: &std::path::Path) -> AppConfig {
    let mut config = managed_multi_tenant_config(base_dir);
    config.tenancy.admin_auth.value.clear();
    config
}

fn single_mode_management_config(base_dir: &std::path::Path) -> AppConfig {
    AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        workers: 1,
        log_level: "debug".to_string(),
        config_dir: base_dir.join("config"),
        tenancy: TenancyConfig {
            mode: TenancyMode::Single,
            tenants_dir: base_dir.join("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig {
                header: "x-admin-key".to_string(),
                value: "global-admin-secret".to_string(),
                value_file: None,
                value_env: None,
            },
        },
        latency: crate::config::LatencyConfig::default(),
        chaos: crate::config::ChaosConfig::default(),
        endpoints: vec![crate::config::EndpointConfig {
            path: "/v1/chat/completions".to_string(),
            format: "openai".to_string(),
            content_type: None,
        }],
        response_file: None,
        reload_args: None,
    }
}

fn management_test_base(name: &str) -> PathBuf {
    let temp_base = std::env::current_dir()
        .unwrap()
        .join(format!("target/{}", name));
    if temp_base.exists() {
        fs::remove_dir_all(&temp_base).unwrap();
    }
    temp_base
}

fn toml_path(value: &std::path::Path) -> String {
    value.display().to_string().replace('\\', "\\\\")
}

fn write_file_backed_management_config(
    base_dir: &std::path::Path,
    admin_key: &str,
    acme_key: &str,
    duplicate_acme: bool,
) -> PathBuf {
    fs::create_dir_all(base_dir).unwrap();
    let config_path = base_dir.join("mock-server.toml");
    fs::write(
        &config_path,
        format!(
            r#"
port = 8100
workers = 1
log_level = "debug"
config_dir = "{config_dir}"

[tenancy]
mode = "multi"
tenants_dir = "{tenants_dir}"
tenant_header = "x-tenant"

[tenancy.admin_auth]
header = "x-admin-key"
value = "{admin_key}"
"#,
            config_dir = toml_path(&base_dir.join("config")),
            tenants_dir = toml_path(&base_dir.join("tenants")),
            admin_key = admin_key,
        ),
    )
    .unwrap();

    write_tenant_metadata(
        base_dir,
        "acme",
        &format!(
            r#"
id = "acme"

[[keys]]
source = "header"
name = "x-api-key"
value = "{}"
"#,
            acme_key
        ),
    );
    write_tenant_metadata(
        base_dir,
        "globex",
        r#"
id = "globex"

[[keys]]
source = "header"
name = "x-api-key"
value = "secret-globex"
"#,
    );

    if duplicate_acme {
        write_tenant_metadata(
            base_dir,
            "globex-copy",
            r#"
id = "acme"

[[keys]]
source = "header"
name = "x-api-key"
value = "secret-globex"
"#,
        );
    } else {
        let duplicate_dir = base_dir.join("tenants/globex-copy");
        if duplicate_dir.exists() {
            fs::remove_dir_all(duplicate_dir).unwrap();
        }
    }

    config_path
}

fn load_file_backed_management_config(config_path: &std::path::Path) -> AppConfig {
    let args =
        crate::config::Cli::parse_from(&["mock-server", "--config", config_path.to_str().unwrap()]);
    AppConfig::build_config(args).unwrap()
}

#[tokio::test]
async fn test_same_path_different_tenants_use_different_runtimes() {
    let temp_base = std::env::current_dir()
        .unwrap()
        .join("target/test_multi_tenant_runtimes");
    if temp_base.exists() {
        fs::remove_dir_all(&temp_base).unwrap();
    }

    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = multi_tenant_config(&temp_base);
    let store = Arc::new(TenantStoreHandle::new(
        build_runtime_store(&config).unwrap(),
    ));
    let app = create_app(config, None, store).await;

    let default_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let default_bytes = axum::body::to_bytes(default_response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8(default_bytes.to_vec())
        .unwrap()
        .contains("\"default\""));

    let acme_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let acme_bytes = axum::body::to_bytes(acme_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let acme_body = String::from_utf8(acme_bytes.to_vec()).unwrap();
    assert!(acme_body.contains("\"acme\""));
    assert!(!acme_body.contains("\"globex\""));

    let globex_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "globex")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let globex_bytes = axum::body::to_bytes(globex_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let globex_body = String::from_utf8(globex_bytes.to_vec()).unwrap();
    assert!(globex_body.contains("\"globex\""));
    assert!(!globex_body.contains("\"acme\""));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_models_endpoint_is_tenant_isolated() {
    let temp_base = management_test_base("test_models_endpoint_tenant_isolation");
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    fs::write(
        temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"
name: "acme-openai"
matcher: "^/v1/chat/completions$"
response_body: '{"tenant":"acme"}'
priority: 100
"#,
    )
    .unwrap();
    fs::write(
        temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"
name: "globex-openai"
matcher: "^/v1/chat/completions$"
response_body: '{"tenant":"globex"}'
priority: 100
"#,
    )
    .unwrap();

    let config = multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let acme_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("x-tenant", "acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(acme_response.status(), StatusCode::OK);
    let acme_body: serde_json::Value =
        serde_json::from_str(&response_text(acme_response).await).unwrap();
    let acme_models = acme_body["data"].to_string();
    assert!(acme_models.contains("acme-openai"));
    assert!(!acme_models.contains("globex-openai"));

    let globex_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("x-tenant", "globex")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(globex_response.status(), StatusCode::OK);
    let globex_body: serde_json::Value =
        serde_json::from_str(&response_text(globex_response).await).unwrap();
    let globex_models = globex_body["data"].to_string();
    assert!(globex_models.contains("globex-openai"));
    assert!(!globex_models.contains("acme-openai"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_models_endpoint_resolves_tenant_from_query_key() {
    let temp_base = management_test_base("test_models_endpoint_query_tenant_resolution");
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    fs::write(
        temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"
name: "acme-openai"
matcher: "^/v1/chat/completions$"
response_body: '{"tenant":"acme"}'
priority: 100
"#,
    )
    .unwrap();
    fs::write(
        temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"
name: "globex-openai"
matcher: "^/v1/chat/completions$"
response_body: '{"tenant":"globex"}'
priority: 100
"#,
    )
    .unwrap();

    let config = AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        workers: 1,
        log_level: "debug".to_string(),
        config_dir: PathBuf::from("config"),
        tenancy: TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig::default(),
        },
        latency: crate::config::LatencyConfig::default(),
        chaos: crate::config::ChaosConfig::default(),
        endpoints: vec![crate::config::EndpointConfig {
            path: "/v1/chat/completions".to_string(),
            format: "openai".to_string(),
            content_type: None,
        }],
        response_file: None,
        reload_args: None,
    };
    write_tenant_metadata(
        &temp_base,
        "acme",
        r#"
id = "acme"

[[keys]]
source = "query"
name = "api_key"
value = "secret-acme"
"#,
    );
    write_tenant_metadata(
        &temp_base,
        "globex",
        r#"
id = "globex"

[[keys]]
source = "query"
name = "api_key"
value = "secret-globex"
"#,
    );
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let acme_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models?api_key=secret-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(acme_response.status(), StatusCode::OK);
    let acme_body: serde_json::Value =
        serde_json::from_str(&response_text(acme_response).await).unwrap();
    let acme_models = acme_body["data"].to_string();
    assert!(acme_models.contains("acme-openai"));
    assert!(!acme_models.contains("globex-openai"));

    let globex_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models?api_key=secret-globex")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(globex_response.status(), StatusCode::OK);
    let globex_body: serde_json::Value =
        serde_json::from_str(&response_text(globex_response).await).unwrap();
    let globex_models = globex_body["data"].to_string();
    assert!(globex_models.contains("globex-openai"));
    assert!(!globex_models.contains("acme-openai"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_multi_mode_tenant_policies_change_latency_and_chaos_defaults() {
    let temp_base = management_test_base("test_multi_mode_tenant_policy_defaults");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );
    let mut config = multi_tenant_config(&temp_base);
    config.latency.base_ms = 0;
    config.latency.jitter_pct = 0.0;
    config.chaos.enabled = true;
    config.chaos.drop_pct = 0.0;
    write_tenant_metadata(
        &temp_base,
        "acme",
        r#"
id = "acme"

[latency]
base_ms = 70
jitter_pct = 0.0
"#,
    );
    write_tenant_metadata(
        &temp_base,
        "globex",
        r#"
id = "globex"

[chaos]
drop_pct = 100.0
"#,
    );
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let acme_start = std::time::Instant::now();
    let acme_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let acme_elapsed = acme_start.elapsed();
    assert_eq!(acme_response.status(), StatusCode::OK);
    assert!(
        acme_elapsed >= std::time::Duration::from_millis(45),
        "expected acme tenant latency override to apply, got {:?}",
        acme_elapsed
    );
    assert!(response_text(acme_response).await.contains("\"acme\""));

    let globex_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "globex")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(globex_response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_default_tenant_fallback_uses_default_tenant_policy() {
    let temp_base = management_test_base("test_default_tenant_policy_fallback");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    let mut config = multi_tenant_config(&temp_base);
    config.chaos.enabled = true;
    config.chaos.drop_pct = 0.0;
    write_tenant_metadata(
        &temp_base,
        "default",
        r#"
id = "default"

[chaos]
drop_pct = 100.0
"#,
    );
    write_tenant_metadata(
        &temp_base,
        "acme",
        r#"
id = "acme"
"#,
    );
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let default_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(default_response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let acme_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(acme_response.status(), StatusCode::OK);
    assert!(response_text(acme_response).await.contains("\"acme\""));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_request_headers_override_tenant_policy_per_request() {
    let temp_base = management_test_base("test_request_headers_override_tenant_policy");
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    let mut config = multi_tenant_config(&temp_base);
    config.chaos.enabled = true;
    config.chaos.drop_pct = 0.0;
    write_tenant_metadata(
        &temp_base,
        "acme",
        r#"
id = "acme"

[latency]
base_ms = 120
jitter_pct = 0.0

[chaos]
drop_pct = 0.0
"#,
    );
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let baseline_start = std::time::Instant::now();
    let baseline_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let baseline_elapsed = baseline_start.elapsed();
    assert_eq!(baseline_response.status(), StatusCode::OK);
    assert!(
        baseline_elapsed >= std::time::Duration::from_millis(90),
        "expected tenant latency default to apply, got {:?}",
        baseline_elapsed
    );

    let override_start = std::time::Instant::now();
    let override_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "acme")
                .header("x-vidai-latency", "0")
                .header("x-vidai-chaos-drop", "100")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let override_elapsed = override_start.elapsed();
    assert_eq!(
        override_response.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(
        override_elapsed + std::time::Duration::from_millis(50) < baseline_elapsed,
        "expected request headers to beat tenant defaults, baseline {:?}, override {:?}",
        baseline_elapsed,
        override_elapsed
    );

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_single_mode_latency_and_chaos_still_use_global_defaults() {
    let temp_base = management_test_base("test_single_mode_global_latency_and_chaos");
    write_provider(
        &temp_base.join("config/providers/openai.yaml"),
        r#"{"tenant":"single"}"#,
    );

    let config = AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        workers: 1,
        log_level: "debug".to_string(),
        config_dir: temp_base.join("config"),
        tenancy: TenancyConfig {
            mode: TenancyMode::Single,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig::default(),
        },
        latency: crate::config::LatencyConfig {
            mode: "benchmark".to_string(),
            base_ms: 70,
            jitter_pct: 0.0,
        },
        chaos: crate::config::ChaosConfig {
            enabled: true,
            malformed_pct: 0.0,
            drop_pct: 100.0,
            trickle_ms: 0,
            disconnect_pct: 0.0,
        },
        endpoints: vec![crate::config::EndpointConfig {
            path: "/v1/chat/completions".to_string(),
            format: "openai".to_string(),
            content_type: None,
        }],
        response_file: None,
        reload_args: None,
    };
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let start = std::time::Instant::now();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        elapsed >= std::time::Duration::from_millis(45),
        "expected single mode to keep global latency defaults, got {:?}",
        elapsed
    );

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_normal_render_path_includes_resolved_tenant_metadata() {
    let temp_base = management_test_base("test_normal_render_tenant_metadata");
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant_id":"{{ tenant.id }}","display_name":"{{ tenant.display_name }}","tier":"{{ tenant.labels.tier }}","has_management_auth":"{{ tenant.management_auth is defined }}"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("\"tenant_id\":\"acme\""));
    assert!(body.contains("\"display_name\":\"Acme Corp\""));
    assert!(body.contains("\"tier\":\"gold\""));
    assert!(body.contains("\"has_management_auth\":\"false\""));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_streaming_render_path_includes_resolved_tenant_metadata() {
    let temp_base = management_test_base("test_streaming_render_tenant_metadata");
    write_streaming_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"choices":[{"message":{"content":"{{ tenant.id }}|{{ tenant.display_name }}|{{ tenant.labels.region }}"}}]}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"stream":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("acme"));
    assert!(body.contains("Acme"));
    assert!(body.contains("Corp"));
    assert!(body.contains("eu-west"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_resolution_failures_are_generic_externally_but_structured_internally() {
    let temp_base = management_test_base("test_tenant_rejections_are_generic");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let scenarios = vec![
        (
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "missing")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
            "unknown_tenant",
        ),
        (
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "unknown")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
            "unknown_key",
        ),
        (
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
            "missing_key",
        ),
        (
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-tenant", "globex")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
            "header_key_conflict",
        ),
    ];

    for (request, expected_reason) in scenarios {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let metrics = response
            .extensions()
            .get::<TenantRequestMetrics>()
            .cloned()
            .expect("rejected tenant response should include rejection metrics");
        let body = response_text(response).await;
        assert_eq!(body, "Tenant authentication failed.");
        match metrics {
            TenantRequestMetrics::Rejected { reason } => assert_eq!(reason, expected_reason),
            TenantRequestMetrics::Accepted { tenant } => {
                panic!(
                    "expected rejection metrics, got accepted tenant label {}",
                    tenant
                )
            }
        }
    }

    let accepted = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted_metrics = accepted
        .extensions()
        .get::<TenantRequestMetrics>()
        .cloned()
        .expect("accepted tenant response should include tenant metrics");
    match accepted_metrics {
        TenantRequestMetrics::Accepted { tenant } => assert_eq!(tenant, "acme"),
        TenantRequestMetrics::Rejected { reason } => {
            panic!(
                "expected accepted tenant metrics, got rejection reason {}",
                reason
            )
        }
    }

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_global_admin_can_list_tenants() {
    let temp_base = management_test_base("test_admin_lists_tenants");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/tenants")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_text(response).await).unwrap();
    assert_eq!(body["mode"], "multi");
    assert_eq!(body["tenants"].as_array().unwrap().len(), 3);
    assert!(body["tenants"].to_string().contains("\"default\""));
    assert!(body["tenants"].to_string().contains("\"acme\""));
    assert!(body["tenants"].to_string().contains("\"globex\""));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_admin_endpoints_fail_when_admin_auth_is_unset() {
    let temp_base = management_test_base("test_admin_requires_configured_auth");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = unmanaged_admin_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/tenants")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::UNAUTHORIZED);

    let inspect_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/tenants/acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(inspect_response.status(), StatusCode::UNAUTHORIZED);

    let reload_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/reload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload_response.status(), StatusCode::UNAUTHORIZED);

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_admin_endpoints_accept_authorization_bearer_secret() {
    let temp_base = management_test_base("test_admin_accepts_bearer_secret");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let mut config = managed_multi_tenant_config(&temp_base);
    config.tenancy.admin_auth.header = "authorization".to_string();
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/tenants")
                .header("authorization", "Bearer global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_global_admin_can_inspect_one_tenant() {
    let temp_base = management_test_base("test_admin_inspects_tenant");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/tenants/acme")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_text(response).await).unwrap();
    assert_eq!(body["id"], "acme");
    assert_eq!(body["is_default"], false);
    assert_eq!(body["requires_key"], true);

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_global_admin_can_trigger_reload() {
    let temp_base = management_test_base("test_admin_reload");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config_path = write_file_backed_management_config(
        &temp_base,
        "global-admin-secret",
        "secret-acme",
        false,
    );
    let config = load_file_backed_management_config(&config_path);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(before).await.contains("acme-before"));

    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/reload")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::OK);
    assert!(response_text(reload).await.contains("acme"));

    let after = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(after).await.contains("acme-after"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_admin_reload_rereads_config_file_changes() {
    let temp_base = management_test_base("test_admin_reload_rereads_config_file");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config_path = write_file_backed_management_config(
        &temp_base,
        "global-admin-secret",
        "secret-acme",
        false,
    );
    let config = load_file_backed_management_config(&config_path);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    write_file_backed_management_config(
        &temp_base,
        "global-admin-secret",
        "secret-acme-rotated",
        false,
    );

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/reload")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::OK);

    let old_key_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_key_response.status(), StatusCode::UNAUTHORIZED);

    let new_key_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme-rotated")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_key_response.status(), StatusCode::OK);
    assert!(response_text(new_key_response).await.contains("acme"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_failed_config_file_reload_keeps_previous_runtime_active() {
    let temp_base = management_test_base("test_failed_config_reload_keeps_runtime");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config_path = write_file_backed_management_config(
        &temp_base,
        "global-admin-secret",
        "secret-acme",
        false,
    );
    let config = load_file_backed_management_config(&config_path);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    write_file_backed_management_config(&temp_base, "global-admin-secret", "secret-acme", true);

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/reload")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let after_failed_reload = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_failed_reload.status(), StatusCode::OK);
    assert!(response_text(after_failed_reload).await.contains("acme"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_admin_reload_rejects_restart_required_config_changes() {
    let temp_base = management_test_base("test_admin_reload_rejects_restart_required_changes");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config_path = write_file_backed_management_config(
        &temp_base,
        "global-admin-secret",
        "secret-acme",
        false,
    );
    let config = load_file_backed_management_config(&config_path);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );
    fs::write(
        &config_path,
        format!(
            r#"
port = 8100
workers = 1
log_level = "debug"
config_dir = "{config_dir}"

[latency]
mode = "realistic"
base_ms = 25
jitter_pct = 0.0

[tenancy]
mode = "multi"
tenants_dir = "{tenants_dir}"
tenant_header = "x-tenant"

[tenancy.admin_auth]
header = "x-admin-key"
value = "global-admin-secret"
"#,
            config_dir = toml_path(&temp_base.join("config")),
            tenants_dir = toml_path(&temp_base.join("tenants")),
        ),
    )
    .unwrap();

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/reload")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::CONFLICT);
    assert!(response_text(reload).await.contains("latency"));

    let after_failed_reload = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_failed_reload.status(), StatusCode::OK);
    assert!(response_text(after_failed_reload)
        .await
        .contains("acme-before"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_admin_can_inspect_own_tenant() {
    let temp_base = management_test_base("test_tenant_inspects_own");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/tenant")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_text(response).await).unwrap();
    assert_eq!(body["id"], "acme");

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_request_key_can_call_mock_endpoint_but_cannot_reload_tenant() {
    let temp_base = management_test_base("test_request_key_cannot_manage_tenant");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let mock_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mock_response.status(), StatusCode::OK);

    let tenant_reload = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-api-key", "secret-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tenant_reload.status(), StatusCode::UNAUTHORIZED);

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_admin_can_reload_own_tenant() {
    let temp_base = management_test_base("test_tenant_reload_own");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::OK);
    assert!(response_text(reload).await.contains("acme"));

    let acme_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(acme_response).await.contains("acme-after"));

    let globex_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-globex")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(globex_response).await.contains("globex"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_failed_tenant_reload_keeps_previous_runtime_active() {
    let temp_base = management_test_base("test_failed_tenant_reload_keeps_runtime");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(before).await.contains("acme-before"));

    fs::remove_file(temp_base.join("secrets/acme.key")).unwrap();

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let after_failed_reload = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(after_failed_reload)
        .await
        .contains("acme-before"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_malformed_tenant_provider_reload_keeps_previous_runtime_active() {
    let temp_base = management_test_base("test_malformed_tenant_provider_reload_keeps_runtime");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(before).await.contains("acme-before"));

    fs::write(
        temp_base.join("tenants/acme/providers/openai.yaml"),
        "name: [broken-provider",
    )
    .unwrap();

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let after_failed_reload = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(after_failed_reload)
        .await
        .contains("acme-before"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_malformed_tenant_template_reload_keeps_previous_runtime_active() {
    let temp_base = management_test_base("test_malformed_tenant_template_reload_keeps_runtime");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    fs::create_dir_all(temp_base.join("tenants/acme/providers")).unwrap();
    fs::write(
        temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"
name: "acme-template"
matcher: "^/v1/chat/completions$"
response_template: "openai/custom.json.j2"
priority: 100
"#,
    )
    .unwrap();
    fs::create_dir_all(temp_base.join("tenants/acme/templates/openai")).unwrap();
    fs::write(
        temp_base.join("tenants/acme/templates/openai/custom.json.j2"),
        r#"{"tenant":"acme-before"}"#,
    )
    .unwrap();
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(before).await.contains("acme-before"));

    fs::write(
        temp_base.join("tenants/acme/templates/openai/custom.json.j2"),
        "{% if %}",
    )
    .unwrap();

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let after_failed_reload = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(after_failed_reload)
        .await
        .contains("acme-before"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_reload_collision_fails_and_keeps_previous_runtime_and_auth_state() {
    let temp_base = management_test_base("test_tenant_reload_collision_keeps_runtime");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let before = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(before).await.contains("acme-before"));

    fs::write(temp_base.join("secrets/acme.key"), "secret-globex").unwrap();
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let old_key_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_key_response.status(), StatusCode::OK);
    assert!(response_text(old_key_response)
        .await
        .contains("acme-before"));

    let globex_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-globex")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(globex_response.status(), StatusCode::OK);
    assert!(response_text(globex_response).await.contains("globex"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_reload_management_auth_conflict_keeps_previous_runtime_and_admin_auth() {
    let temp_base = management_test_base("test_tenant_reload_management_auth_conflict");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );
    write_tenant_metadata(
        &temp_base,
        "acme",
        r#"
id = "acme"

[[keys]]
source = "header"
name = "x-api-key"
value = "secret-acme"

[management_auth]
header = "x-tenant-admin-key"
value = "tenant-admin-globex"
"#,
    );

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let inspect = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tenant")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(inspect.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&response_text(inspect).await).unwrap();
    assert_eq!(body["id"], "acme");

    let acme_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(acme_response.status(), StatusCode::OK);
    assert!(response_text(acme_response).await.contains("acme-before"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_reload_succeeds_when_other_tenant_secret_source_is_broken() {
    let temp_base = management_test_base("test_tenant_reload_ignores_other_tenant_secret");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let globex_key_path = temp_base.join("secrets/globex.key");
    fs::write(&globex_key_path, "secret-globex").unwrap();
    write_tenant_metadata(
        &temp_base,
        "globex",
        &format!(
            r#"
id = "globex"

[[keys]]
source = "header"
name = "x-api-key"
value_file = "{}"
"#,
            toml_path(&globex_key_path)
        ),
    );

    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );
    fs::remove_file(&globex_key_path).unwrap();

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::OK);

    let acme_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(acme_response).await.contains("acme-after"));

    let globex_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-globex")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(globex_response.status(), StatusCode::OK);
    assert!(response_text(globex_response).await.contains("globex"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_reload_succeeds_when_admin_auth_secret_source_is_broken() {
    let temp_base = management_test_base("test_tenant_reload_ignores_admin_auth_secret");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let mut config = managed_multi_tenant_config(&temp_base);
    let admin_key_path = temp_base.join("secrets/admin.key");
    fs::write(&admin_key_path, "global-admin-secret").unwrap();
    config.tenancy.admin_auth.value.clear();
    config.tenancy.admin_auth.value_file = Some(admin_key_path.clone());

    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );
    fs::remove_file(&admin_key_path).unwrap();

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::OK);

    let acme_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(acme_response).await.contains("acme-after"));

    let admin_response = app
        .oneshot(
            Request::builder()
                .uri("/admin/tenants")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_response.status(), StatusCode::OK);

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_secret_rotation_is_picked_up_by_tenant_reload() {
    let temp_base = management_test_base("test_tenant_secret_rotation_reload");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-before"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    fs::write(temp_base.join("secrets/acme.key"), "secret-acme-rotated").unwrap();
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme-after"}"#,
    );

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::OK);

    let old_key_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_key_response.status(), StatusCode::UNAUTHORIZED);

    let new_key_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("x-api-key", "secret-acme-rotated")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response_text(new_key_response).await.contains("acme-after"));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_tenant_admin_cannot_target_another_tenant() {
    let temp_base = management_test_base("test_tenant_cannot_target_other");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let inspect = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tenant")
                .header("x-tenant", "globex")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(inspect.status(), StatusCode::UNAUTHORIZED);

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-tenant", "globex")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload.status(), StatusCode::UNAUTHORIZED);

    let admin_attempt = app
        .oneshot(
            Request::builder()
                .uri("/admin/tenants/globex")
                .header("x-tenant-admin-key", "tenant-admin-acme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_attempt.status(), StatusCode::UNAUTHORIZED);

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_management_responses_do_not_expose_secret_bearing_fields() {
    let temp_base = management_test_base("test_management_response_sanitization");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );
    write_provider(
        &temp_base.join("tenants/globex/providers/openai.yaml"),
        r#"{"tenant":"globex"}"#,
    );

    let config = managed_multi_tenant_config(&temp_base);
    let request_secret_path = temp_base.join("secrets/acme.key");
    let management_secret_path = temp_base.join("secrets/acme-admin.key");
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/tenants/acme")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(!body.contains("global-admin-secret"));
    assert!(!body.contains("secret-acme"));
    assert!(!body.contains("tenant-admin-acme"));
    assert!(!body.contains("tenant-admin-globex"));
    assert!(!body.contains(&request_secret_path.display().to_string()));
    assert!(!body.contains(&management_secret_path.display().to_string()));
    assert!(!body.contains("value_file"));
    assert!(!body.contains("value_env"));
    assert!(!body.contains("\"value\""));

    fs::remove_dir_all(temp_base).unwrap();
}

#[tokio::test]
async fn test_single_mode_tenant_management_requires_admin_auth() {
    let temp_base = management_test_base("test_single_mode_management");
    write_provider(
        &temp_base.join("config/providers/openai.yaml"),
        r#"{"tenant":"single"}"#,
    );

    let config = single_mode_management_config(&temp_base);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    let tenant_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tenant")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tenant_response.status(), StatusCode::UNAUTHORIZED);

    let tenant_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tenant")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tenant_response.status(), StatusCode::OK);
    let tenant_body: serde_json::Value =
        serde_json::from_str(&response_text(tenant_response).await).unwrap();
    assert_eq!(tenant_body["id"], "default");

    let tenant_reload_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tenant_reload_response.status(), StatusCode::UNAUTHORIZED);

    let tenant_reload_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tenant/reload")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tenant_reload_response.status(), StatusCode::OK);

    let admin_response = app
        .oneshot(
            Request::builder()
                .uri("/admin/tenants")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_response.status(), StatusCode::OK);
    let admin_body: serde_json::Value =
        serde_json::from_str(&response_text(admin_response).await).unwrap();
    assert_eq!(admin_body["mode"], "single");
    assert_eq!(admin_body["tenants"].as_array().unwrap().len(), 1);
    assert_eq!(admin_body["tenants"][0]["id"], "default");

    fs::remove_dir_all(temp_base).unwrap();
}

fn write_echo_endpoint_config(config: &mut AppConfig) {
    config.endpoints.push(crate::config::EndpointConfig {
        path: "/echo".to_string(),
        format: "echo".to_string(),
        content_type: None,
    });
}

/// M-1 regression: echo endpoints must pass real query params to tenant
/// resolution so tenants whose only API key is source=query can be reached.
/// Previously echo_handler hard-coded an empty HashMap, making query-keyed
/// tenants always fall back to the default tenant regardless of the key in
/// the URL. Verify by checking that the resolved tenant label in the
/// response metrics matches "acme", not the default.
#[tokio::test]
async fn test_echo_handler_resolves_tenant_from_query_key() {
    let temp_base = management_test_base("test_echo_query_key_tenant_resolution");
    write_tenant_metadata(
        &temp_base,
        "acme",
        r#"
id = "acme"

[[keys]]
source = "query"
name = "api_key"
value = "secret-acme"
"#,
    );

    let mut config = AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        workers: 1,
        log_level: "debug".to_string(),
        config_dir: PathBuf::from("config"),
        tenancy: TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig::default(),
        },
        latency: crate::config::LatencyConfig::default(),
        chaos: crate::config::ChaosConfig::default(),
        endpoints: vec![crate::config::EndpointConfig {
            path: "/v1/chat/completions".to_string(),
            format: "openai".to_string(),
            content_type: None,
        }],
        response_file: None,
        reload_args: None,
    };
    write_echo_endpoint_config(&mut config);

    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    // With the correct query key the echo endpoint must resolve the tenant and
    // return 200 with the request body mirrored back.
    let body_content = r#"{"hello":"world"}"#;
    let ok_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo?api_key=secret-acme")
                .body(Body::from(body_content))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        ok_response.status(),
        StatusCode::OK,
        "echo with valid query key must resolve tenant and return 200"
    );

    // Verify the ACME tenant was resolved, not the default fallback.
    // Previously the empty query_params map caused the acme key to be ignored
    // and the request fell through to the default tenant instead.
    let metrics = ok_response
        .extensions()
        .get::<TenantRequestMetrics>()
        .cloned()
        .expect("echo response must carry tenant metrics");
    match metrics {
        TenantRequestMetrics::Accepted { tenant } => {
            assert_eq!(
                tenant, "acme",
                "query key on echo endpoint must resolve to acme, not the default tenant"
            );
        }
        TenantRequestMetrics::Rejected { reason } => {
            panic!("expected accepted tenant, got rejection: {}", reason);
        }
    }

    // Confirm the body was echoed back unmodified.
    assert_eq!(response_text(ok_response).await, body_content);

    fs::remove_dir_all(temp_base).unwrap();
}

/// Regression: reload_requires_restart must detect tenancy config changes so
/// that operators cannot silently apply a new tenant_header, mode, or admin_auth
/// config through a live reload. Previously the tenancy field was not included
/// in the restart-required detection.
#[tokio::test]
async fn test_admin_reload_rejects_tenancy_config_changes() {
    let temp_base = management_test_base("test_admin_reload_rejects_tenancy_changes");
    write_provider(
        &temp_base.join("tenants/default/providers/openai.yaml"),
        r#"{"tenant":"default"}"#,
    );
    write_provider(
        &temp_base.join("tenants/acme/providers/openai.yaml"),
        r#"{"tenant":"acme"}"#,
    );

    let config_path = write_file_backed_management_config(
        &temp_base,
        "global-admin-secret",
        "secret-acme",
        false,
    );
    let config = load_file_backed_management_config(&config_path);
    let app = create_app(
        config.clone(),
        None,
        Arc::new(TenantStoreHandle::new(
            build_runtime_store(&config).unwrap(),
        )),
    )
    .await;

    // Rewrite the config file with a different tenant_header ("x-org" instead
    // of "x-tenant"). Everything else stays the same so this is the only
    // change that should trigger the restart-required response.
    fs::write(
        &config_path,
        format!(
            r#"
port = 8100
workers = 1
log_level = "debug"
config_dir = "{config_dir}"

[tenancy]
mode = "multi"
tenants_dir = "{tenants_dir}"
tenant_header = "x-org"

[tenancy.admin_auth]
header = "x-admin-key"
value = "global-admin-secret"
"#,
            config_dir = toml_path(&temp_base.join("config")),
            tenants_dir = toml_path(&temp_base.join("tenants")),
        ),
    )
    .unwrap();

    let reload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/reload")
                .header("x-admin-key", "global-admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        reload.status(),
        StatusCode::CONFLICT,
        "a tenancy config change must be rejected with CONFLICT to require restart"
    );
    let body = response_text(reload).await;
    assert!(
        body.contains("tenancy"),
        "CONFLICT response must mention 'tenancy'; got: {}",
        body
    );

    fs::remove_dir_all(temp_base).unwrap();
}
