//! Downloading, cancelling and removing the analysis engine.

use super::jobs::run_blocking_command;
use super::updates::http_client;
use crate::commands::BackendCommands;
use crate::models::ApiResponse;
use tauri::{AppHandle, Emitter, State};

pub(super) const ESSENTIA_DOWNLOAD_EVENT: &str = "essentia_download_progress";

/// Cancellation flag for in-progress Essentia download. Managed Tauri state.
pub struct EssentiaDownloadCancel(pub std::sync::Arc<std::sync::atomic::AtomicBool>);

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct EssentiaDownloadProgress {
    bytes_received: u64,
    total_bytes: Option<u64>,
    percent: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tauri::command]
pub async fn download_essentia(
    app: AppHandle,
    state: State<'_, BackendCommands>,
    cancel: State<'_, EssentiaDownloadCancel>,
) -> Result<ApiResponse<()>, String> {
    use futures_util::StreamExt;
    use std::sync::atomic::Ordering;

    fn extract_npm_package_from_tgz(
        tgz_path: &std::path::Path,
        node_modules_dir: &std::path::Path,
        package_name: &str,
    ) -> Result<(), String> {
        let extract_dir = node_modules_dir
            .parent()
            .unwrap_or(node_modules_dir)
            .join(format!("extract_tmp_{package_name}"));
        if extract_dir.exists() {
            std::fs::remove_dir_all(&extract_dir)
                .map_err(|e| format!("cleanup extract dir ({package_name}): {e}"))?;
        }
        std::fs::create_dir_all(&extract_dir)
            .map_err(|e| format!("create extract dir ({package_name}): {e}"))?;

        let tgz_file =
            std::fs::File::open(tgz_path).map_err(|e| format!("open tgz ({package_name}): {e}"))?;
        let gz = flate2::read::GzDecoder::new(tgz_file);
        let mut archive = tar::Archive::new(gz);
        archive
            .unpack(&extract_dir)
            .map_err(|e| format!("extract failed ({package_name}): {e}"))?;

        let package_dir = extract_dir.join("package");
        let dest = node_modules_dir.join(package_name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("remove old package ({package_name}): {e}"))?;
        }
        std::fs::rename(&package_dir, &dest)
            .map_err(|e| format!("install package ({package_name}): {e}"))?;

        let _ = std::fs::remove_dir_all(&extract_dir);
        Ok(())
    }

    cancel.0.store(false, Ordering::SeqCst);

    let data_dir = state.data_dir().to_path_buf();
    let cancel_flag = cancel.0.clone();

    let result: Result<(), String> = async {
        let url = "https://registry.npmjs.org/essentia.js/-/essentia.js-0.1.3.tgz";
        let response = http_client()?
            .get(url)
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        let total_bytes = response.content_length();
        let mut stream = response.bytes_stream();

        let tmp_tgz = data_dir.join("essentia_download.tmp.tgz");
        let mut file = std::fs::File::create(&tmp_tgz)
            .map_err(|e| format!("failed to create temp file: {e}"))?;

        let mut bytes_received: u64 = 0;
        while let Some(chunk) = stream.next().await {
            if cancel_flag.load(Ordering::SeqCst) {
                drop(file);
                let _ = std::fs::remove_file(&tmp_tgz);
                return Err("cancelled".to_string());
            }
            let chunk = chunk.map_err(|e| format!("download error: {e}"))?;
            std::io::Write::write_all(&mut file, &chunk)
                .map_err(|e| format!("write error: {e}"))?;
            bytes_received += chunk.len() as u64;
            let percent = total_bytes
                .map(|t| (bytes_received as f32 / t as f32 * 100.0).min(99.0))
                .unwrap_or(0.0);
            let _ = app.emit(
                ESSENTIA_DOWNLOAD_EVENT,
                EssentiaDownloadProgress {
                    bytes_received,
                    total_bytes,
                    percent,
                    done: None,
                    error: None,
                },
            );
        }
        drop(file);

        // Install package tarball to app-data node_modules.
        let node_modules_dir = data_dir.join("essentia").join("node_modules");
        std::fs::create_dir_all(&node_modules_dir)
            .map_err(|e| format!("create node_modules dir: {e}"))?;
        extract_npm_package_from_tgz(&tmp_tgz, &node_modules_dir, "essentia.js")?;

        // Install known dependency required by essentia.js package.
        let node_wav_tgz = data_dir.join("node_wav_download.tmp.tgz");
        let dep_url = "https://registry.npmjs.org/node-wav/-/node-wav-0.0.2.tgz";
        let dep_bytes = http_client()?
            .get(dep_url)
            .send()
            .await
            .map_err(|e| format!("download node-wav failed: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("read node-wav payload failed: {e}"))?;
        std::fs::write(&node_wav_tgz, &dep_bytes)
            .map_err(|e| format!("write node-wav temp file failed: {e}"))?;
        extract_npm_package_from_tgz(&node_wav_tgz, &node_modules_dir, "node-wav")?;
        let _ = std::fs::remove_file(&node_wav_tgz);

        if !node_modules_dir
            .join("essentia.js")
            .join("dist")
            .join("essentia-wasm.umd.js")
            .is_file()
        {
            return Err(
                "essentia install incomplete: missing dist/essentia-wasm.umd.js".to_string(),
            );
        }
        if !node_modules_dir
            .join("node-wav")
            .join("package.json")
            .is_file()
        {
            return Err("essentia install incomplete: missing node-wav dependency".to_string());
        }

        // Clean up temp files
        let _ = std::fs::remove_file(&tmp_tgz);

        // Make the new install visible to analysis in this session without restart.
        crate::essentia_runtime::set_node_modules(node_modules_dir.clone());

        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            let _ = app.emit(
                ESSENTIA_DOWNLOAD_EVENT,
                EssentiaDownloadProgress {
                    bytes_received: 0,
                    total_bytes: None,
                    percent: 100.0,
                    done: Some(true),
                    error: None,
                },
            );
            Ok(ApiResponse {
                ok: true,
                data: Some(()),
                error: None,
            })
        }
        Err(msg) => {
            let _ = app.emit(
                ESSENTIA_DOWNLOAD_EVENT,
                EssentiaDownloadProgress {
                    bytes_received: 0,
                    total_bytes: None,
                    percent: 0.0,
                    done: None,
                    error: Some(msg.clone()),
                },
            );
            Ok(ApiResponse {
                ok: false,
                data: None,
                error: Some(crate::error::ErrorPayload::new(
                    crate::error::ErrorCode::InternalError,
                    msg,
                )),
            })
        }
    }
}

#[tauri::command]
pub fn cancel_essentia_download(cancel: State<'_, EssentiaDownloadCancel>) -> ApiResponse<()> {
    use std::sync::atomic::Ordering;
    cancel.0.store(true, Ordering::SeqCst);
    ApiResponse {
        ok: true,
        data: Some(()),
        error: None,
    }
}

#[tauri::command]
pub async fn remove_essentia(state: State<'_, BackendCommands>) -> Result<ApiResponse<()>, String> {
    let commands = state.inner().clone();
    Ok(run_blocking_command("remove_essentia", move || commands.remove_essentia()).await)
}
