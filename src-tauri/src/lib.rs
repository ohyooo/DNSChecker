use serde::{Deserialize, Deserializer, Serialize};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};
use tauri::Manager;

#[derive(Debug, Deserialize, Serialize)]
struct CheckResult {
    #[serde(default)]
    line: u32,
    server: String,
    protocol: Option<String>,
    domain: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    duration_ms: f64,
    #[serde(default, deserialize_with = "null_vec")]
    answers: Vec<String>,
    #[serde(default, deserialize_with = "null_vec")]
    expected: Vec<String>,
    matched: Option<bool>,
    status: String,
    error: Option<String>,
    response_code: Option<String>,
}

fn null_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize, Serialize)]
struct BatchCheckResponse {
    results: Vec<CheckResult>,
    total: u32,
    ok: u32,
    failed: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct ExpandServersResponse {
    servers: String,
    changed: bool,
    error: Option<String>,
}

#[tauri::command]
async fn check_servers(
    app: tauri::AppHandle,
    servers: String,
    domain: String,
    type_name: String,
    expected: String,
    bootstrap: String,
    timeout: String,
) -> Result<BatchCheckResponse, String> {
    let output = tauri::async_runtime::spawn_blocking(move || {
        run_helper(
            &app,
            vec![
                "batch".to_string(),
                "-servers".to_string(),
                servers,
                "-domain".to_string(),
                domain,
                "-type".to_string(),
                type_name,
                "-expected".to_string(),
                expected,
                "-bootstrap".to_string(),
                bootstrap,
                "-timeout".to_string(),
                timeout,
            ],
        )
    })
    .await
    .map_err(|err| format!("DNS helper task failed: {err}"))??;
    serde_json::from_str(&output).map_err(|err| format!("invalid helper JSON: {err}; output: {output}"))
}

#[tauri::command]
async fn expand_servers(
    app: tauri::AppHandle,
    servers: String,
    bootstrap: String,
    timeout: String,
    domain: String,
    type_name: String,
    expected: String,
) -> Result<ExpandServersResponse, String> {
    let _ = (domain, type_name, expected);
    let output = tauri::async_runtime::spawn_blocking(move || {
        run_helper(
            &app,
            vec![
                "expand".to_string(),
                "-servers".to_string(),
                servers,
                "-bootstrap".to_string(),
                bootstrap,
                "-timeout".to_string(),
                timeout,
            ],
        )
    })
    .await
    .map_err(|err| format!("DNS helper task failed: {err}"))??;
    serde_json::from_str(&output).map_err(|err| format!("invalid helper JSON: {err}; output: {output}"))
}

fn run_helper(app: &tauri::AppHandle, args: Vec<String>) -> Result<String, String> {
    let root = project_root()?;
    let helper = helper_path(app, &root);
    let output = if helper.exists() {
        Command::new(helper).args(&args).current_dir(&root).output()
    } else {
        let mut go_args = vec!["run", "."];
        go_args.extend(args.iter().map(String::as_str));
        Command::new("go").args(go_args).current_dir(&root).output()
    }
    .map_err(|err| format!("failed to run DNS helper: {err}"))?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|err| format!("helper output is not UTF-8: {err}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!("DNS helper failed: {stderr}{stdout}"))
    }
}

fn project_root() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "failed to locate project root".to_string())
}

fn helper_path(app: &tauri::AppHandle, root: &Path) -> PathBuf {
    let exe = if cfg!(windows) {
        "dnschecker-helper.exe"
    } else {
        "dnschecker-helper"
    };
    let dev_helper = root.join("bin").join(exe);
    if dev_helper.exists() {
        return dev_helper;
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled_helper = resource_dir.join(exe);
        if bundled_helper.exists() {
            return bundled_helper;
        }
        let nested_helper = resource_dir.join("bin").join(exe);
        if nested_helper.exists() {
            return nested_helper;
        }
        let tauri_resource_helper = resource_dir.join("_up_").join("bin").join(exe);
        if tauri_resource_helper.exists() {
            return tauri_resource_helper;
        }
    }
    dev_helper
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![check_servers, expand_servers])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
