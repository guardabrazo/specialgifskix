use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use tauri::{AppHandle, Emitter, Manager};

fn silent_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = silent_command(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}

#[derive(Deserialize)]
struct ConversionSettings {
    files: Vec<String>,
    output_folder: Option<String>,
    width: u32,
    fps: u32,
    quality: u32,
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    current: usize,
    total: usize,
    file: String,
}

fn gifski_binary(app: &AppHandle) -> Result<PathBuf, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let name = "gifski-mac-arm64";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    let name = "gifski-mac-x64";
    #[cfg(target_os = "windows")]
    let name = "gifski-win-x64.exe";
    #[cfg(target_os = "linux")]
    let name = "gifski-linux-x64";

    let path = [resource_dir.join(name), resource_dir.join("resources").join(name)]
        .into_iter()
        .find(|p| p.exists())
        .ok_or_else(|| format!("gifski binary not found (looked in {})", resource_dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).map_err(|e| e.to_string())?;
    }

    Ok(path)
}

#[tauri::command]
fn validate_gifski(app: AppHandle) -> Result<String, String> {
    let bin = gifski_binary(&app)?;
    let out = silent_command(&bin)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if version.is_empty() { "gifski ready".into() } else { version })
}

#[tauri::command]
fn scan_folder(folder: String) -> Vec<String> {
    let path = PathBuf::from(&folder);
    if !path.is_dir() {
        return vec![];
    }
    std::fs::read_dir(&path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| {
                            let lower = ext.to_lowercase();
                            matches!(lower.as_str(), "mp4" | "avi" | "mov" | "mkv" | "webm")
                        })
                        .unwrap_or(false)
                })
                .map(|e| e.path().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

// On macOS: gifski from Homebrew has built-in FFmpeg video support — call it directly.
// On Windows: cargo-compiled gifski has no video support — use ffmpeg to extract
//             frames to a temp dir first, then feed them to gifski.
#[tauri::command]
async fn convert_videos(app: AppHandle, settings: ConversionSettings) -> Result<(), String> {
    let gifski = gifski_binary(&app)?;
    let total = settings.files.len();

    for (i, file_str) in settings.files.iter().enumerate() {
        let input = PathBuf::from(file_str);
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .ok_or("Invalid filename")?;

        let output = if let Some(ref folder) = settings.output_folder {
            PathBuf::from(folder).join(format!("{}.gif", stem))
        } else {
            input
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(format!("{}.gif", stem))
        };

        let file_name = input
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        app.emit(
            "conversion-progress",
            ProgressPayload { current: i + 1, total, file: file_name.clone() },
        )
        .map_err(|e| e.to_string())?;

        #[cfg(target_os = "windows")]
        convert_via_ffmpeg(&gifski, &input, &output, settings.fps, settings.width, settings.quality)
            .map_err(|e| {
                let msg = format!("Failed to convert {}: {}", file_name, e);
                let _ = app.emit("conversion-error", msg.clone());
                msg
            })?;

        #[cfg(not(target_os = "windows"))]
        {
            let status = silent_command(&gifski)
                .arg("--fps").arg(settings.fps.to_string())
                .arg("--width").arg(settings.width.to_string())
                .arg("--quality").arg(settings.quality.to_string())
                .arg("-o").arg(&output)
                .arg(&input)
                .status()
                .map_err(|e| e.to_string())?;

            if !status.success() {
                let msg = format!("Failed to convert: {}", file_name);
                app.emit("conversion-error", msg.clone()).map_err(|e| e.to_string())?;
                return Err(msg);
            }
        }
    }

    app.emit("conversion-complete", ()).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn convert_via_ffmpeg(
    gifski: &PathBuf,
    input: &PathBuf,
    output: &PathBuf,
    fps: u32,
    width: u32,
    quality: u32,
) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!(
        "sgx_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

    let frames_pattern = tmp.join("frame%06d.png");

    // Step 1: extract frames with ffmpeg
    let ffmpeg_status = silent_command("ffmpeg")
        .arg("-i").arg(input)
        .arg("-vf").arg(format!("fps={fps},scale={width}:-1:flags=lanczos"))
        .arg("-y")
        .arg(&frames_pattern)
        .status()
        .map_err(|e| format!("ffmpeg not found: {e}"))?;

    if !ffmpeg_status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("ffmpeg frame extraction failed".into());
    }

    // Step 2: collect frame paths in order
    let mut frames: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "png").unwrap_or(false))
        .collect();

    frames.sort();

    if frames.is_empty() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("no frames extracted by ffmpeg".into());
    }

    // Step 3: convert frames to GIF with gifski
    let status = silent_command(gifski)
        .arg("--fps").arg(fps.to_string())
        .arg("--width").arg(width.to_string())
        .arg("--quality").arg(quality.to_string())
        .arg("-o").arg(output)
        .args(&frames)
        .status()
        .map_err(|e| e.to_string())?;

    let _ = std::fs::remove_dir_all(&tmp);

    if !status.success() {
        return Err("gifski conversion failed".into());
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            validate_gifski,
            scan_folder,
            convert_videos
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
