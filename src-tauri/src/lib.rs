use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::AsyncWriteExt;
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

const API_ORIGIN: &str = "https://unchartable.site";
const MAX_ARCHIVE_BYTES: u64 = 250 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 10_000;

#[derive(Default)]
struct InstallRuntime {
    cancelled: Mutex<HashSet<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppState {
    custom_songs_path: String,
    directory_exists: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InstallProgress {
    chart_id: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    stage: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallResult {
    chart_id: String,
    install_path: String,
    archive_sha256: String,
}

#[derive(Deserialize)]
struct DownloadTicket {
    url: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct InstallMetadata {
    chart_id: String,
    title: String,
    artist: String,
    charter: String,
    archive_sha256: String,
    source: String,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    installed_at: Option<String>,
    #[serde(default = "default_updates_enabled")]
    updates_enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledChart {
    chart_id: Option<String>,
    title: String,
    artist: Option<String>,
    charter: Option<String>,
    folder_name: String,
    path: String,
    managed: bool,
    playable: bool,
    size_bytes: u64,
    updated_at: Option<String>,
    installed_at: Option<String>,
    updates_enabled: bool,
    #[serde(skip)]
    manual_identity: Option<ManualChartIdentity>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrashMetadata {
    trash_id: String,
    original_folder_name: String,
    deleted_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrashItem {
    trash_id: String,
    chart_id: Option<String>,
    title: String,
    original_folder_name: String,
    deleted_at: String,
    size_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCandidate {
    chart: serde_json::Value,
    installed_version: Option<String>,
    latest_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManualChartMatch {
    installed_path: String,
    chart: serde_json::Value,
}

#[derive(Default)]
struct ManualChartIdentity {
    title: String,
    artist: String,
    creators: HashSet<String>,
    creator_label: Option<String>,
    duration_seconds: Option<f64>,
}

#[derive(Default)]
struct LocalChartInspection {
    identity: ManualChartIdentity,
    has_chart: bool,
    has_audio: bool,
    size_bytes: u64,
}

fn default_updates_enabled() -> bool {
    true
}

fn default_custom_songs_path() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local_app_data);
        if let Some(app_data) = local.parent() {
            return app_data
                .join("LocalLow")
                .join("D-CELL GAMES")
                .join("UNBEATABLE")
                .join("CustomSongs");
        }
    }
    PathBuf::from(r"C:\Users\Public\D-CELL GAMES\UNBEATABLE\CustomSongs")
}

fn validate_chart_id(chart_id: &str) -> Result<(), String> {
    Uuid::parse_str(chart_id)
        .map(|_| ())
        .map_err(|_| "invalid chart id".to_string())
}

fn validate_target_directory(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("choose the UNBEATABLE CustomSongs folder first.".to_string());
    }
    fs::create_dir_all(path)
        .map_err(|error| format!("could not create the CustomSongs folder: {error}"))?;
    if !path.is_dir() {
        return Err("the selected CustomSongs path is not a directory.".to_string());
    }
    Ok(())
}

fn parse_allowed_external_url(url: &str) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|_| "invalid UNCHARTABLE URL.".to_string())?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("unchartable.site") {
        return Err("only unchartable.site links can be opened by the app.".to_string());
    }
    Ok(parsed)
}

fn is_direct_child(parent: &Path, child: &Path) -> bool {
    child.parent() == Some(parent)
}

fn directory_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(metadata) = entry.metadata() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

fn inspect_chart_structure(path: &Path) -> (bool, bool) {
    const AUDIO_EXTENSIONS: &[&str] = &["mp3", "ogg", "wav", "flac", "m4a", "aac"];
    let mut has_chart = false;
    let mut has_audio = false;
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            has_chart |= extension == "txt";
            has_audio |= AUDIO_EXTENSIONS.contains(&extension.as_str());
        }
    }
    (has_chart, has_audio)
}

fn read_install_metadata(path: &Path) -> Option<InstallMetadata> {
    let body = fs::read(path.join(".unchartable.json")).ok()?;
    serde_json::from_slice(&body).ok()
}

fn normalized_match_value(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn chart_text_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(key)
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
    })
}

fn inspect_manual_chart_identity(path: &Path) -> Option<ManualChartIdentity> {
    let inspection = inspect_local_chart(path, true);
    (!inspection.identity.title.is_empty() && !inspection.identity.artist.is_empty())
        .then_some(inspection.identity)
}

fn inspect_local_chart(path: &Path, read_identity: bool) -> LocalChartInspection {
    const AUDIO_EXTENSIONS: &[&str] = &["mp3", "ogg", "wav", "flac", "m4a", "aac"];
    let mut inspection = LocalChartInspection::default();
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                pending.push(entry_path);
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                inspection.size_bytes = inspection.size_bytes.saturating_add(metadata.len());
            }
            let extension = entry_path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            inspection.has_chart |= extension == "txt";
            inspection.has_audio |= AUDIO_EXTENSIONS.contains(&extension.as_str());
            if !read_identity || extension != "txt" {
                continue;
            }
            let Ok(body) = fs::read_to_string(&entry_path) else {
                continue;
            };
            if inspection.identity.title.is_empty() {
                inspection.identity.title = chart_text_value(&body, "Title")
                    .or_else(|| chart_text_value(&body, "TitleUnicode"))
                    .unwrap_or_default()
                    .to_string();
            }
            if inspection.identity.artist.is_empty() {
                inspection.identity.artist = chart_text_value(&body, "Artist")
                    .or_else(|| chart_text_value(&body, "ArtistUnicode"))
                    .unwrap_or_default()
                    .to_string();
            }
            if let Some(creator) = chart_text_value(&body, "Creator") {
                if inspection.identity.creator_label.is_none() {
                    inspection.identity.creator_label = Some(creator.to_string());
                }
                inspection
                    .identity
                    .creators
                    .insert(normalized_match_value(creator));
            }
            if inspection.identity.duration_seconds.is_none() {
                inspection.identity.duration_seconds = chart_text_value(&body, "Tags")
                    .and_then(|tags| serde_json::from_str::<serde_json::Value>(tags).ok())
                    .and_then(|tags| tags.get("SongLength")?.as_f64());
            }
        }
    }
    inspection
}

fn scan_installed(target: &Path) -> Result<Vec<InstalledChart>, String> {
    validate_target_directory(target)?;
    let mut charts = Vec::new();
    let entries =
        fs::read_dir(target).map_err(|error| format!("could not scan CustomSongs: {error}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder_name = entry.file_name().to_string_lossy().to_string();
        if folder_name.starts_with(".unchartable-") {
            continue;
        }
        let metadata = read_install_metadata(&path);
        let inspection = inspect_local_chart(&path, metadata.is_none());
        let manual_identity = (!inspection.identity.title.is_empty()
            && !inspection.identity.artist.is_empty())
        .then_some(inspection.identity);
        charts.push(InstalledChart {
            chart_id: metadata.as_ref().map(|value| value.chart_id.clone()),
            title: metadata
                .as_ref()
                .map(|value| value.title.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| manual_identity.as_ref().map(|value| value.title.clone()))
                .unwrap_or_else(|| folder_name.clone()),
            artist: metadata
                .as_ref()
                .map(|value| value.artist.clone())
                .or_else(|| manual_identity.as_ref().map(|value| value.artist.clone())),
            charter: metadata
                .as_ref()
                .map(|value| value.charter.clone())
                .or_else(|| manual_identity.as_ref()?.creator_label.clone()),
            folder_name,
            path: path.to_string_lossy().to_string(),
            managed: metadata.is_some(),
            playable: inspection.has_chart && inspection.has_audio,
            size_bytes: inspection.size_bytes,
            updated_at: metadata.as_ref().and_then(|value| value.updated_at.clone()),
            installed_at: metadata
                .as_ref()
                .and_then(|value| value.installed_at.clone()),
            updates_enabled: metadata
                .as_ref()
                .map(|value| value.updates_enabled)
                .unwrap_or(false),
            manual_identity,
        });
    }
    charts.sort_by(|left, right| left.title.to_lowercase().cmp(&right.title.to_lowercase()));
    Ok(charts)
}

fn trash_directory(_target: &Path) -> PathBuf {
    #[cfg(test)]
    {
        _target.join(".unchartable-trash")
    }

    #[cfg(not(test))]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("UNCHARTABLE")
            .join("Trash")
    }
}

fn migrate_legacy_trash(legacy: &Path, trash: &Path) -> Result<(), String> {
    if !legacy.is_dir() || legacy == trash {
        return Ok(());
    }
    fs::create_dir_all(trash)
        .map_err(|error| format!("could not create the private UNCHARTABLE trash: {error}"))?;
    for entry in fs::read_dir(legacy)
        .map_err(|error| format!("could not read the old CustomSongs trash: {error}"))?
        .flatten()
    {
        let source = entry.path();
        let destination = trash.join(entry.file_name());
        if destination.exists() {
            return Err(format!(
                "could not migrate the old trash because {} already exists.",
                destination.display()
            ));
        }
        fs::rename(&source, &destination).map_err(|error| {
            format!("could not move the old trash outside CustomSongs: {error}")
        })?;
    }
    fs::remove_dir(legacy)
        .map_err(|error| format!("could not remove the old CustomSongs trash folder: {error}"))
}

fn prepare_trash_directory(target: &Path) -> Result<PathBuf, String> {
    let trash = trash_directory(target);
    let legacy = target.join(".unchartable-trash");
    migrate_legacy_trash(&legacy, &trash)?;
    fs::create_dir_all(&trash)
        .map_err(|error| format!("could not create the UNCHARTABLE trash: {error}"))?;
    Ok(trash)
}

fn app_state_for_path(custom_songs_path: PathBuf) -> Result<AppState, String> {
    validate_target_directory(&custom_songs_path)?;
    Ok(AppState {
        directory_exists: true,
        custom_songs_path: custom_songs_path.to_string_lossy().to_string(),
    })
}

fn sanitize_folder_name(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'..='\u{1f}' => ' ',
            _ => character,
        })
        .collect();
    let sanitized = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    let sanitized = sanitized.trim_matches([' ', '.']).to_string();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if sanitized.is_empty()
        || reserved
            .iter()
            .any(|name| sanitized.eq_ignore_ascii_case(name))
    {
        "Unchartable Chart".to_string()
    } else {
        sanitized.chars().take(100).collect()
    }
}

fn generated_install_folder_name(title: &str) -> String {
    sanitize_folder_name(title.trim())
}

fn conflicting_install_folder_name(title: &str, charter: &str) -> String {
    let charter = if charter.trim().is_empty() {
        "Unknown Charter"
    } else {
        charter.trim()
    };
    sanitize_folder_name(&format!("{} by {}", title.trim(), charter))
}

fn install_folder_name(
    source_path: &Path,
    staging_path: &Path,
    title: &str,
    _charter: &str,
) -> String {
    if source_path != staging_path {
        if let Some(existing_name) = source_path.file_name().and_then(|name| name.to_str()) {
            return sanitize_folder_name(existing_name);
        }
    }
    generated_install_folder_name(title)
}

fn contains_blocked_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "bat" | "cmd" | "com" | "dll" | "exe" | "lnk" | "msi" | "ps1" | "scr"
    )
}

fn extract_zip_safely(archive_path: &Path, staging_path: &Path) -> Result<PathBuf, String> {
    let archive_file = fs::File::open(archive_path)
        .map_err(|error| format!("could not open the chart archive: {error}"))?;
    let mut archive = ZipArchive::new(archive_file)
        .map_err(|error| format!("the downloaded file is not a valid ZIP: {error}"))?;
    if archive.len() > MAX_ARCHIVE_FILES {
        return Err("the chart archive contains too many files.".to_string());
    }

    let mut extracted_bytes = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("could not read ZIP entry: {error}"))?;
        let relative_path = entry
            .enclosed_name()
            .ok_or_else(|| "the ZIP contains an unsafe file path.".to_string())?;
        if relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err("the ZIP contains an unsafe file path.".to_string());
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("symbolic links are not allowed in chart archives.".to_string());
        }
        if contains_blocked_file(&relative_path) {
            return Err(format!(
                "blocked executable file in archive: {}",
                relative_path.display()
            ));
        }

        extracted_bytes = extracted_bytes
            .checked_add(entry.size())
            .ok_or_else(|| "the extracted chart size is invalid.".to_string())?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err("the chart expands beyond the safe extraction limit.".to_string());
        }

        let output_path = staging_path.join(relative_path);
        if entry.is_dir() {
            fs::create_dir_all(&output_path)
                .map_err(|error| format!("could not create chart directory: {error}"))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create chart directory: {error}"))?;
        }
        let mut output = fs::File::create(&output_path)
            .map_err(|error| format!("could not create chart file: {error}"))?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| format!("could not extract chart file: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("could not finish chart file: {error}"))?;
    }

    let entries = fs::read_dir(staging_path)
        .map_err(|error| format!("could not inspect extracted chart: {error}"))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    if entries.len() == 1 && entries[0].path().is_dir() {
        Ok(entries[0].path())
    } else {
        Ok(staging_path.to_path_buf())
    }
}

fn find_existing_install(target: &Path, chart_id: &str) -> Option<PathBuf> {
    fs::read_dir(target)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let directory = entry.path();
            if !directory.is_dir() {
                return None;
            }
            let metadata = fs::read_to_string(directory.join(".unchartable.json")).ok()?;
            let value: serde_json::Value = serde_json::from_str(&metadata).ok()?;
            (value.get("chartId")?.as_str()? == chart_id).then_some(directory)
        })
}

fn unique_install_path(target: &Path, folder_name: &str) -> PathBuf {
    let preferred = target.join(folder_name);
    if !preferred.exists() {
        return preferred;
    }
    (2..=999)
        .map(|suffix| target.join(format!("{folder_name} ({suffix})")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| target.join(format!("{folder_name}-{}", Uuid::new_v4().simple())))
}

fn new_install_path(target: &Path, folder_name: &str, title: &str, charter: &str) -> PathBuf {
    let preferred = target.join(folder_name);
    if !preferred.exists() {
        return preferred;
    }
    unique_install_path(target, &conflicting_install_folder_name(title, charter))
}

fn finalize_install(
    source_path: &Path,
    staging_path: &Path,
    destination: &Path,
    metadata: &InstallMetadata,
) -> Result<(), String> {
    let metadata_body = serde_json::to_vec_pretty(metadata)
        .map_err(|error| format!("could not create install metadata: {error}"))?;
    fs::write(source_path.join(".unchartable.json"), metadata_body)
        .map_err(|error| format!("could not write install metadata: {error}"))?;

    let backup =
        destination.with_extension(format!("unchartable-backup-{}", Uuid::new_v4().simple()));
    if destination.exists() {
        fs::rename(destination, &backup)
            .map_err(|error| format!("could not back up the existing chart: {error}"))?;
    }
    if let Err(error) = fs::rename(source_path, destination) {
        if backup.exists() {
            let _ = fs::rename(&backup, destination);
        }
        return Err(format!(
            "could not move the chart into CustomSongs: {error}"
        ));
    }
    if staging_path.exists() {
        let _ = fs::remove_dir_all(staging_path);
    }
    if backup.exists() {
        fs::remove_dir_all(backup).map_err(|error| {
            format!("chart installed, but its previous backup could not be removed: {error}")
        })?;
    }
    Ok(())
}

#[tauri::command]
fn get_app_state() -> Result<AppState, String> {
    app_state_for_path(default_custom_songs_path())
}

#[tauri::command]
fn validate_custom_songs_path(path: String) -> Result<AppState, String> {
    app_state_for_path(PathBuf::from(path))
}

#[tauri::command]
fn open_custom_songs_folder(path: String) -> Result<(), String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(&target)
            .spawn()
            .map_err(|error| format!("could not launch Windows Explorer: {error}"))?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = target;
        Err("opening the game folder is currently supported on Windows only.".to_string())
    }
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let parsed = parse_allowed_external_url(&url)?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(parsed.as_str())
            .spawn()
            .map_err(|error| format!("could not open the UNCHARTABLE website: {error}"))?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        Err("opening website links is currently supported on Windows only.".to_string())
    }
}

#[tauri::command]
fn list_installed_charts(path: String) -> Result<Vec<InstalledChart>, String> {
    scan_installed(&PathBuf::from(path))
}

#[tauri::command]
fn trash_installed_chart(path: String, chart_id: String) -> Result<(), String> {
    validate_chart_id(&chart_id)?;
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let source = find_existing_install(&target, &chart_id)
        .ok_or_else(|| "this managed chart is no longer installed.".to_string())?;
    if !is_direct_child(&target, &source) {
        return Err("refusing to remove a chart outside CustomSongs.".to_string());
    }

    let trash = prepare_trash_directory(&target)?;
    let original_folder_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "the chart folder name is invalid.".to_string())?
        .to_string();
    let trash_id = Uuid::new_v4().to_string();
    let destination = trash.join(format!("{trash_id}-{original_folder_name}"));
    fs::rename(&source, &destination)
        .map_err(|error| format!("could not move the chart to trash: {error}"))?;
    let trash_metadata = TrashMetadata {
        trash_id,
        original_folder_name,
        deleted_at: chrono::Utc::now().to_rfc3339(),
    };
    let body = serde_json::to_vec_pretty(&trash_metadata)
        .map_err(|error| format!("could not create trash metadata: {error}"))?;
    fs::write(destination.join(".unchartable-trash.json"), body)
        .map_err(|error| format!("could not finish moving the chart to trash: {error}"))
}

#[tauri::command]
fn list_trashed_charts(path: String) -> Result<Vec<TrashItem>, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let trash = prepare_trash_directory(&target)?;
    if !trash.is_dir() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let entries =
        fs::read_dir(&trash).map_err(|error| format!("could not scan the trash: {error}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(trash_body) = fs::read(path.join(".unchartable-trash.json")) else {
            continue;
        };
        let Ok(trash_metadata) = serde_json::from_slice::<TrashMetadata>(&trash_body) else {
            continue;
        };
        let install_metadata = read_install_metadata(&path);
        items.push(TrashItem {
            trash_id: trash_metadata.trash_id,
            chart_id: install_metadata
                .as_ref()
                .map(|value| value.chart_id.clone()),
            title: install_metadata
                .map(|value| value.title)
                .unwrap_or_else(|| trash_metadata.original_folder_name.clone()),
            original_folder_name: trash_metadata.original_folder_name,
            deleted_at: trash_metadata.deleted_at,
            size_bytes: directory_size(&path),
        });
    }
    items.sort_by(|left, right| right.deleted_at.cmp(&left.deleted_at));
    Ok(items)
}

#[tauri::command]
fn restore_trashed_chart(path: String, trash_id: String) -> Result<String, String> {
    validate_chart_id(&trash_id)?;
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let trash = prepare_trash_directory(&target)?;
    let entries =
        fs::read_dir(&trash).map_err(|error| format!("could not scan the trash: {error}"))?;
    for entry in entries.flatten() {
        let source = entry.path();
        let Ok(body) = fs::read(source.join(".unchartable-trash.json")) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_slice::<TrashMetadata>(&body) else {
            continue;
        };
        if metadata.trash_id != trash_id {
            continue;
        }
        let destination = unique_install_path(&target, &metadata.original_folder_name);
        let _ = fs::remove_file(source.join(".unchartable-trash.json"));
        fs::rename(&source, &destination)
            .map_err(|error| format!("could not restore the chart: {error}"))?;
        return Ok(destination.to_string_lossy().to_string());
    }
    Err("the trashed chart could not be found.".to_string())
}

#[tauri::command]
fn empty_chart_trash(path: String) -> Result<usize, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let trash = prepare_trash_directory(&target)?;
    let entries =
        fs::read_dir(&trash).map_err(|error| format!("could not scan the trash: {error}"))?;
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let item_path = entry.path();
        if !item_path.is_dir() {
            continue;
        }
        let Ok(body) = fs::read(item_path.join(".unchartable-trash.json")) else {
            continue;
        };
        if serde_json::from_slice::<TrashMetadata>(&body).is_err() {
            continue;
        }
        fs::remove_dir_all(&item_path)
            .map_err(|error| format!("could not permanently remove a trashed chart: {error}"))?;
        removed += 1;
    }
    Ok(removed)
}

fn chart_matches_manual_identity(
    chart: &serde_json::Value,
    identity: &ManualChartIdentity,
) -> bool {
    let title_matches = chart
        .get("title")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            normalized_match_value(value) == normalized_match_value(&identity.title)
        });
    let artist_matches = chart
        .get("artist")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            normalized_match_value(value) == normalized_match_value(&identity.artist)
        });
    if !title_matches || !artist_matches || identity.creators.is_empty() {
        return false;
    }

    let mut site_charters = HashSet::new();
    for field in ["charterName"] {
        if let Some(value) = chart.get(field).and_then(serde_json::Value::as_str) {
            site_charters.insert(normalized_match_value(value));
        }
    }
    if let Some(submitter) = chart.get("submitter") {
        for field in ["displayName", "discordUsername"] {
            if let Some(value) = submitter.get(field).and_then(serde_json::Value::as_str) {
                site_charters.insert(normalized_match_value(value));
            }
        }
    }
    if let Some(levels) = chart
        .get("difficultyLevels")
        .and_then(serde_json::Value::as_array)
    {
        for level in levels {
            if let Some(value) = level.get("charterName").and_then(serde_json::Value::as_str) {
                site_charters.insert(normalized_match_value(value));
            }
        }
    }
    if identity.creators.is_disjoint(&site_charters) {
        return false;
    }

    match (
        identity.duration_seconds,
        chart
            .get("audioDurationSeconds")
            .and_then(serde_json::Value::as_f64),
    ) {
        (Some(local), Some(remote)) => (local - remote).abs() <= 3.0,
        _ => true,
    }
}

fn unique_manual_match(
    candidates: &[serde_json::Value],
    identity: &ManualChartIdentity,
) -> Option<serde_json::Value> {
    let mut matching = candidates
        .iter()
        .filter(|chart| chart_matches_manual_identity(chart, identity));
    let matched = matching.next()?.clone();
    matching.next().is_none().then_some(matched)
}

#[tauri::command]
async fn find_manual_chart_matches(path: String) -> Result<Vec<ManualChartMatch>, String> {
    let target = PathBuf::from(path);
    let installed = scan_installed(&target)?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("could not prepare manual chart matching: {error}"))?;
    let candidates = installed
        .into_iter()
        .filter(|item| !item.managed && item.playable)
        .filter_map(|item| Some((item.path, item.manual_identity?)));

    Ok(
        futures_util::stream::iter(candidates.map(|(installed_path, identity)| {
            let client = client.clone();
            async move {
                let mut url = Url::parse(&format!("{API_ORIGIN}/api/charts")).ok()?;
                url.query_pairs_mut()
                    .append_pair("page", "0")
                    .append_pair("pageSize", "24")
                    .append_pair("q", &format!("{} {}", identity.title, identity.artist));
                let response = client.get(url).send().await.ok()?;
                if !response.status().is_success() {
                    return None;
                }
                let body = response.json::<serde_json::Value>().await.ok()?;
                let candidates = body
                    .get("charts")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                unique_manual_match(candidates, &identity).map(|chart| ManualChartMatch {
                    installed_path,
                    chart,
                })
            }
        }))
        .buffer_unordered(6)
        .filter_map(async move |item| item)
        .collect()
        .await,
    )
}

#[tauri::command]
async fn adopt_manual_chart(
    path: String,
    installed_path: String,
    chart_id: String,
) -> Result<(), String> {
    validate_chart_id(&chart_id)?;
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let installed_path = PathBuf::from(installed_path);
    if !is_direct_child(&target, &installed_path)
        || read_install_metadata(&installed_path).is_some()
    {
        return Err("this folder cannot be adopted as a manual chart.".to_string());
    }
    let identity = inspect_manual_chart_identity(&installed_path)
        .ok_or_else(|| "could not read this manual chart metadata.".to_string())?;
    let body = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("could not prepare chart verification: {error}"))?
        .get(format!("{API_ORIGIN}/api/charts/{chart_id}"))
        .send()
        .await
        .map_err(|error| format!("could not verify this chart: {error}"))?
        .error_for_status()
        .map_err(|error| format!("could not verify this chart: {error}"))?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("could not read chart verification: {error}"))?;
    let chart = body
        .get("chart")
        .ok_or_else(|| "UNCHARTABLE did not return this chart.".to_string())?;
    if !chart_matches_manual_identity(chart, &identity) {
        return Err("the installed files no longer match this UNCHARTABLE chart.".to_string());
    }
    let metadata = InstallMetadata {
        chart_id,
        title: chart
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&identity.title)
            .to_string(),
        artist: chart
            .get("artist")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&identity.artist)
            .to_string(),
        charter: chart
            .get("charterName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        archive_sha256: String::new(),
        source: API_ORIGIN.to_string(),
        updated_at: chart
            .get("contentUpdatedAt")
            .or_else(|| chart.get("updatedAt"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        installed_at: Some(chrono::Utc::now().to_rfc3339()),
        updates_enabled: true,
    };
    let body = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("could not create chart metadata: {error}"))?;
    fs::write(installed_path.join(".unchartable.json"), body)
        .map_err(|error| format!("could not enable updates for this chart: {error}"))
}

#[tauri::command]
fn set_chart_updates(
    path: String,
    installed_path: String,
    chart_id: String,
    enabled: bool,
) -> Result<(), String> {
    validate_chart_id(&chart_id)?;
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let installed_path = PathBuf::from(installed_path);
    if !is_direct_child(&target, &installed_path) {
        return Err("this chart is outside the selected CustomSongs folder.".to_string());
    }
    let mut metadata = read_install_metadata(&installed_path)
        .ok_or_else(|| "this chart is missing UNCHARTABLE metadata.".to_string())?;
    if metadata.chart_id != chart_id {
        return Err("this chart metadata no longer matches the selected chart.".to_string());
    }
    metadata.updates_enabled = enabled;
    let body = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("could not update chart preferences: {error}"))?;
    fs::write(installed_path.join(".unchartable.json"), body)
        .map_err(|error| format!("could not save chart preferences: {error}"))
}

#[tauri::command]
async fn check_installed_updates(path: String) -> Result<Vec<UpdateCandidate>, String> {
    let target = PathBuf::from(path);
    let installed = scan_installed(&target)?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("could not prepare update checks: {error}"))?;
    let mut updates = Vec::new();
    for item in installed
        .into_iter()
        .filter(|item| item.managed && item.updates_enabled)
    {
        let Some(chart_id) = item.chart_id else {
            continue;
        };
        let response = match client
            .get(format!("{API_ORIGIN}/api/charts/{chart_id}"))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response,
            _ => continue,
        };
        let Ok(body) = response.json::<serde_json::Value>().await else {
            continue;
        };
        let Some(chart) = body.get("chart").cloned() else {
            continue;
        };
        if !chart
            .get("hasDirectDownload")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let latest_version = chart
            .get("contentUpdatedAt")
            .and_then(serde_json::Value::as_str)
            .or_else(|| chart.get("updatedAt").and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string();
        if latest_version.is_empty() || item.updated_at.as_deref() == Some(&latest_version) {
            continue;
        }
        updates.push(UpdateCandidate {
            chart,
            installed_version: item.updated_at,
            latest_version,
        });
    }
    Ok(updates)
}

#[tauri::command]
fn launch_unbeatable() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg("steam://run/2240620")
            .spawn()
            .map_err(|error| format!("could not ask Steam to launch UNBEATABLE: {error}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("launching UNBEATABLE is currently supported on Windows only.".to_string())
    }
}

#[tauri::command]
async fn fetch_charts(
    query: String,
    page: u32,
    difficulty: String,
    ranked_only: bool,
) -> Result<serde_json::Value, String> {
    let mut url =
        Url::parse(&format!("{API_ORIGIN}/api/charts")).map_err(|error| error.to_string())?;
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("page", &page.to_string());
        params.append_pair("pageSize", "24");
        params.append_pair("sort", "newest");
        if !query.trim().is_empty() {
            params.append_pair("q", query.trim());
        }
        if !difficulty.is_empty() {
            params.append_pair("difficulty", &difficulty);
        }
        if ranked_only {
            params.append_pair("ranked", "1");
        }
    }
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not prepare the catalog request: {error}"))?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("could not reach unchartable.site: {error}"))?
        .error_for_status()
        .map_err(|error| format!("UNCHARTABLE returned an error: {error}"))?
        .json()
        .await
        .map_err(|error| format!("could not read the chart catalog: {error}"))
}

#[tauri::command]
async fn fetch_chart(chart_id: String) -> Result<serde_json::Value, String> {
    validate_chart_id(&chart_id)?;
    let payload = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not prepare the chart request: {error}"))?
        .get(format!("{API_ORIGIN}/api/charts/{chart_id}"))
        .send()
        .await
        .map_err(|error| format!("could not reach unchartable.site: {error}"))?
        .error_for_status()
        .map_err(|error| format!("UNCHARTABLE returned an error: {error}"))?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("could not read the chart: {error}"))?;

    payload
        .get("chart")
        .cloned()
        .ok_or_else(|| "UNCHARTABLE did not return a chart.".to_string())
}

#[tauri::command]
fn cancel_install(runtime: State<'_, InstallRuntime>, chart_id: String) -> Result<(), String> {
    validate_chart_id(&chart_id)?;
    runtime
        .cancelled
        .lock()
        .map_err(|_| "the install queue is unavailable.".to_string())?
        .insert(chart_id);
    Ok(())
}

#[tauri::command]
async fn install_chart(
    app: AppHandle,
    runtime: State<'_, InstallRuntime>,
    chart_id: String,
    title: String,
    artist: String,
    charter: String,
    updated_at: String,
    target_directory: String,
) -> Result<InstallResult, String> {
    validate_chart_id(&chart_id)?;
    runtime
        .cancelled
        .lock()
        .map_err(|_| "the install queue is unavailable.".to_string())?
        .remove(&chart_id);
    let target = PathBuf::from(target_directory);
    validate_target_directory(&target)?;

    app.emit(
        "install-progress",
        InstallProgress {
            chart_id: chart_id.clone(),
            downloaded_bytes: 0,
            total_bytes: None,
            stage: "requesting".to_string(),
        },
    )
    .map_err(|error| error.to_string())?;

    let ticket_url = format!("{API_ORIGIN}/api/charts/{chart_id}/download?format=json");
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(4))
        .build()
        .map_err(|error| format!("could not prepare secure download: {error}"))?;
    let ticket: DownloadTicket = client
        .get(ticket_url)
        .send()
        .await
        .map_err(|error| format!("could not request the chart download: {error}"))?
        .error_for_status()
        .map_err(|error| format!("chart download is unavailable: {error}"))?
        .json()
        .await
        .map_err(|error| format!("invalid chart download response: {error}"))?;
    let download_url = Url::parse(&ticket.url)
        .map_err(|_| "UNCHARTABLE returned an invalid download URL.".to_string())?;
    if download_url.scheme() != "https" {
        return Err("the chart download did not use HTTPS.".to_string());
    }
    if download_url
        .host_str()
        .is_some_and(|host| host.contains("google.com") || host.contains("googleusercontent.com"))
    {
        return Err(
            "Google Drive charts are not supported by this first app version yet.".to_string(),
        );
    }

    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|error| format!("chart download failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("chart download failed: {error}"))?;
    let is_rar = response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains(".rar"));
    if is_rar {
        return Err(
            "This chart uses a RAR archive. UNCHARTABLE currently installs ZIP charts only."
                .to_string(),
        );
    }
    let total_bytes = response.content_length();
    if total_bytes.is_some_and(|size| size > MAX_ARCHIVE_BYTES) {
        return Err("the chart archive exceeds the 250 MB download limit.".to_string());
    }

    let operation_id = Uuid::new_v4();
    let archive_path = target.join(format!(".unchartable-{operation_id}.zip"));
    let staging_path = target.join(format!(".unchartable-{operation_id}"));
    let mut archive_file = tokio::fs::File::create(&archive_path)
        .await
        .map_err(|error| format!("could not create the temporary download: {error}"))?;
    let mut stream = response.bytes_stream();
    let mut downloaded_bytes = 0u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = stream.next().await {
        if runtime
            .cancelled
            .lock()
            .map_err(|_| "the install queue is unavailable.".to_string())?
            .remove(&chart_id)
        {
            drop(archive_file);
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err("installation cancelled.".to_string());
        }
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(archive_file);
                let _ = tokio::fs::remove_file(&archive_path).await;
                return Err(format!("chart download was interrupted: {error}"));
            }
        };
        downloaded_bytes = downloaded_bytes
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| "download size overflow.".to_string())?;
        if downloaded_bytes > MAX_ARCHIVE_BYTES {
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err("the chart archive exceeds the 250 MB download limit.".to_string());
        }
        hash.update(&chunk);
        archive_file
            .write_all(&chunk)
            .await
            .map_err(|error| format!("could not save the chart download: {error}"))?;
        app.emit(
            "install-progress",
            InstallProgress {
                chart_id: chart_id.clone(),
                downloaded_bytes,
                total_bytes,
                stage: "downloading".to_string(),
            },
        )
        .map_err(|error| error.to_string())?;
    }
    archive_file
        .flush()
        .await
        .map_err(|error| format!("could not finish the chart download: {error}"))?;
    drop(archive_file);

    app.emit(
        "install-progress",
        InstallProgress {
            chart_id: chart_id.clone(),
            downloaded_bytes,
            total_bytes,
            stage: "installing".to_string(),
        },
    )
    .map_err(|error| error.to_string())?;

    let archive_sha256 = format!("{:x}", hash.finalize());
    let existing = find_existing_install(&target, &chart_id);
    fs::create_dir_all(&staging_path)
        .map_err(|error| format!("could not create the temporary install directory: {error}"))?;
    let source_path = extract_zip_safely(&archive_path, &staging_path);
    let _ = fs::remove_file(&archive_path);
    let source_path = match source_path {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_path);
            return Err(error);
        }
    };
    let (has_chart, has_audio) = inspect_chart_structure(&source_path);
    if !has_chart || !has_audio {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(match (has_chart, has_audio) {
            (false, false) => {
                "the archive does not contain a recognizable chart file or audio file.".to_string()
            }
            (false, true) => "the archive contains audio but no chart .txt file.".to_string(),
            (true, false) => {
                "the archive contains chart data but no supported audio file.".to_string()
            }
            (true, true) => unreachable!(),
        });
    }
    let folder_name = install_folder_name(&source_path, &staging_path, &title, &charter);
    let destination =
        existing.unwrap_or_else(|| new_install_path(&target, &folder_name, &title, &charter));
    let metadata = InstallMetadata {
        chart_id: chart_id.clone(),
        title: title.clone(),
        artist,
        charter,
        archive_sha256: archive_sha256.clone(),
        source: API_ORIGIN.to_string(),
        updated_at: Some(updated_at),
        installed_at: Some(chrono::Utc::now().to_rfc3339()),
        updates_enabled: true,
    };
    if let Err(error) = finalize_install(&source_path, &staging_path, &destination, &metadata) {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(error);
    }

    app.emit(
        "install-progress",
        InstallProgress {
            chart_id: chart_id.clone(),
            downloaded_bytes,
            total_bytes,
            stage: "complete".to_string(),
        },
    )
    .map_err(|error| error.to_string())?;

    Ok(InstallResult {
        chart_id,
        install_path: destination.to_string_lossy().to_string(),
        archive_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        InstallMetadata, app_state_for_path, conflicting_install_folder_name,
        contains_blocked_file, empty_chart_trash, extract_zip_safely, finalize_install,
        generated_install_folder_name, inspect_manual_chart_identity, install_folder_name,
        list_trashed_charts, migrate_legacy_trash, new_install_path, parse_allowed_external_url,
        restore_trashed_chart, sanitize_folder_name, scan_installed, trash_installed_chart,
        unique_manual_match,
    };
    use std::{
        fs::File,
        io::Write,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn sanitizes_windows_folder_names() {
        assert_eq!(sanitize_folder_name("Artist: Song?"), "Artist Song");
        assert_eq!(sanitize_folder_name("CON"), "Unchartable Chart");
        assert_eq!(generated_install_folder_name("Same Song"), "Same Song");
        assert_eq!(
            conflicting_install_folder_name("Same Song", "alice"),
            "Same Song by alice"
        );
    }

    #[test]
    fn preserves_an_archive_root_folder_name() {
        let staging = Path::new(r"C:\temp\staging");
        let source = staging.join("Creator Folder");
        assert_eq!(
            install_folder_name(&source, staging, "Song", "charter"),
            "Creator Folder"
        );
    }

    #[test]
    fn names_a_rootless_archive_from_the_chart_title() {
        let staging = Path::new(r"C:\temp\staging");
        assert_eq!(
            install_folder_name(staging, staging, "Song", "charter"),
            "Song"
        );
    }

    #[test]
    fn adds_the_charter_only_when_the_title_folder_is_taken() {
        let temporary = tempdir().expect("temporary directory");
        std::fs::create_dir(temporary.path().join("Same Song")).expect("existing chart");

        assert_eq!(
            new_install_path(temporary.path(), "Same Song", "Same Song", "alice"),
            temporary.path().join("Same Song by alice")
        );

        std::fs::create_dir(temporary.path().join("Same Song by alice"))
            .expect("existing conflicting chart");
        assert_eq!(
            new_install_path(temporary.path(), "Same Song", "Same Song", "alice"),
            temporary.path().join("Same Song by alice (2)")
        );
    }

    #[test]
    fn blocks_executable_payloads() {
        assert!(contains_blocked_file(Path::new("chart/setup.exe")));
        assert!(contains_blocked_file(Path::new("chart/script.PS1")));
        assert!(!contains_blocked_file(Path::new("chart/song.wav")));
    }

    #[test]
    fn opens_only_secure_unchartable_links() {
        assert!(parse_allowed_external_url("https://unchartable.site/charts/example").is_ok());
        assert!(parse_allowed_external_url("http://unchartable.site/charts/example").is_err());
        assert!(
            parse_allowed_external_url("https://unchartable.site.example/charts/example").is_err()
        );
        assert!(parse_allowed_external_url("file:///C:/Windows/System32").is_err());
    }

    #[test]
    fn extracts_a_safe_single_root_archive() {
        let temporary = tempdir().expect("temporary directory");
        let archive_path = temporary.path().join("chart.zip");
        let staging_path = temporary.path().join("staging");
        std::fs::create_dir_all(&staging_path).expect("staging directory");

        let archive_file = File::create(&archive_path).expect("archive file");
        let mut archive = ZipWriter::new(archive_file);
        archive
            .start_file("Chart/song.wav", SimpleFileOptions::default())
            .expect("zip entry");
        archive.write_all(b"audio").expect("zip contents");
        archive.finish().expect("finish archive");

        let extracted = extract_zip_safely(&archive_path, &staging_path).expect("safe extraction");
        assert_eq!(extracted, staging_path.join("Chart"));
        assert_eq!(
            std::fs::read(extracted.join("song.wav")).expect("extracted file"),
            b"audio"
        );
    }

    #[test]
    fn rejects_parent_directory_entries() {
        let temporary = tempdir().expect("temporary directory");
        let archive_path = temporary.path().join("chart.zip");
        let staging_path = temporary.path().join("staging");
        std::fs::create_dir_all(&staging_path).expect("staging directory");

        let archive_file = File::create(&archive_path).expect("archive file");
        let mut archive = ZipWriter::new(archive_file);
        archive
            .start_file("../outside.txt", SimpleFileOptions::default())
            .expect("zip entry");
        archive.write_all(b"unsafe").expect("zip contents");
        archive.finish().expect("finish archive");

        let result = extract_zip_safely(&archive_path, &staging_path);
        assert!(result.is_err());
        assert!(!temporary.path().join("outside.txt").exists());
    }

    #[test]
    fn creates_a_missing_custom_songs_directory() {
        let temporary = tempdir().expect("temporary directory");
        let custom_songs = temporary
            .path()
            .join("D-CELL GAMES")
            .join("UNBEATABLE")
            .join("CustomSongs");

        let state = app_state_for_path(custom_songs.clone()).expect("app state");

        assert!(custom_songs.is_dir());
        assert!(state.directory_exists);
        assert_eq!(PathBuf::from(state.custom_songs_path), custom_songs);
    }

    #[test]
    fn scans_managed_and_manual_charts_without_claiming_manual_folders() {
        let temporary = tempdir().expect("temporary directory");
        let managed = temporary.path().join("Managed Song");
        let manual = temporary.path().join("Manual Song");
        std::fs::create_dir_all(&managed).expect("managed directory");
        std::fs::create_dir_all(&manual).expect("manual directory");
        std::fs::write(managed.join("chart.txt"), b"chart").expect("managed chart");
        std::fs::write(managed.join("audio.ogg"), b"audio").expect("managed audio");
        std::fs::write(manual.join("chart.txt"), b"chart").expect("manual chart");
        std::fs::write(manual.join("audio.wav"), b"audio").expect("manual audio");
        let metadata = InstallMetadata {
            chart_id: "503c0c85-d4b6-4366-b18b-bbcc6fb44f63".to_string(),
            title: "Managed Song".to_string(),
            artist: "Artist".to_string(),
            charter: "Charter".to_string(),
            archive_sha256: "hash".to_string(),
            source: "https://unchartable.site".to_string(),
            updated_at: Some("2026-07-23T12:00:00Z".to_string()),
            installed_at: Some("2026-07-23T12:01:00Z".to_string()),
            updates_enabled: true,
        };
        std::fs::write(
            managed.join(".unchartable.json"),
            serde_json::to_vec(&metadata).expect("metadata"),
        )
        .expect("metadata file");

        let installed = scan_installed(temporary.path()).expect("installed charts");

        assert_eq!(installed.len(), 2);
        assert!(installed.iter().all(|item| item.playable));
        assert!(
            installed
                .iter()
                .any(|item| item.title == "Managed Song" && item.managed)
        );
        assert!(
            installed
                .iter()
                .any(|item| item.title == "Manual Song" && !item.managed)
        );
    }

    #[test]
    fn matches_same_song_to_the_correct_charter_only() {
        let temporary = tempdir().expect("temporary directory");
        std::fs::write(
            temporary.path().join("chart.txt"),
            "[Metadata]\nTitle:Same Song\nArtist:Same Artist\nCreator:Alice\nTags:{\"SongLength\":120.5}",
        )
        .expect("manual chart");
        let identity =
            inspect_manual_chart_identity(temporary.path()).expect("manual chart identity");
        let candidates = vec![
            serde_json::json!({
                "id": "alice-chart",
                "title": "Same Song",
                "artist": "Same Artist",
                "charterName": "Alice",
                "audioDurationSeconds": 120.0
            }),
            serde_json::json!({
                "id": "bob-chart",
                "title": "Same Song",
                "artist": "Same Artist",
                "charterName": "Bob",
                "audioDurationSeconds": 120.0
            }),
        ];

        let matched = unique_manual_match(&candidates, &identity).expect("unique Alice chart");

        assert_eq!(matched["id"], "alice-chart");
        assert_eq!(identity.creator_label.as_deref(), Some("Alice"));
    }

    #[test]
    fn refuses_an_ambiguous_manual_chart_match() {
        let temporary = tempdir().expect("temporary directory");
        std::fs::write(
            temporary.path().join("chart.txt"),
            "[Metadata]\nTitle:Same Song\nArtist:Same Artist\nCreator:Alice\nTags:{\"SongLength\":120}",
        )
        .expect("manual chart");
        let identity =
            inspect_manual_chart_identity(temporary.path()).expect("manual chart identity");
        let candidates = vec![
            serde_json::json!({
                "id": "first-upload",
                "title": "Same Song",
                "artist": "Same Artist",
                "charterName": "Alice",
                "audioDurationSeconds": 120.0
            }),
            serde_json::json!({
                "id": "second-upload",
                "title": "Same Song",
                "artist": "Same Artist",
                "charterName": "Alice",
                "audioDurationSeconds": 121.0
            }),
        ];

        assert!(unique_manual_match(&candidates, &identity).is_none());
    }

    #[test]
    fn rejects_a_manual_match_with_a_different_duration() {
        let temporary = tempdir().expect("temporary directory");
        std::fs::write(
            temporary.path().join("chart.txt"),
            "[Metadata]\nTitle:Same Song\nArtist:Same Artist\nCreator:Alice\nTags:{\"SongLength\":120}",
        )
        .expect("manual chart");
        let identity =
            inspect_manual_chart_identity(temporary.path()).expect("manual chart identity");
        let candidates = vec![serde_json::json!({
            "id": "wrong-audio",
            "title": "Same Song",
            "artist": "Same Artist",
            "charterName": "Alice",
            "audioDurationSeconds": 180.0
        })];

        assert!(unique_manual_match(&candidates, &identity).is_none());
    }

    #[test]
    fn moves_only_managed_charts_to_trash_and_restores_them() {
        let temporary = tempdir().expect("temporary directory");
        let chart_id = "503c0c85-d4b6-4366-b18b-bbcc6fb44f63";
        let managed = temporary.path().join("Managed Song");
        std::fs::create_dir_all(&managed).expect("managed directory");
        std::fs::write(managed.join("chart.txt"), b"chart").expect("chart");
        std::fs::write(managed.join("audio.ogg"), b"audio").expect("audio");
        let metadata = InstallMetadata {
            chart_id: chart_id.to_string(),
            title: "Managed Song".to_string(),
            artist: "Artist".to_string(),
            charter: "Charter".to_string(),
            archive_sha256: "hash".to_string(),
            source: "https://unchartable.site".to_string(),
            updated_at: Some("2026-07-23T12:00:00Z".to_string()),
            installed_at: Some("2026-07-23T12:01:00Z".to_string()),
            updates_enabled: true,
        };
        std::fs::write(
            managed.join(".unchartable.json"),
            serde_json::to_vec(&metadata).expect("metadata"),
        )
        .expect("metadata file");

        trash_installed_chart(
            temporary.path().to_string_lossy().to_string(),
            chart_id.to_string(),
        )
        .expect("move to trash");
        assert!(!managed.exists());

        let trash = list_trashed_charts(temporary.path().to_string_lossy().to_string())
            .expect("trash contents");
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].title, "Managed Song");

        let restored = restore_trashed_chart(
            temporary.path().to_string_lossy().to_string(),
            trash[0].trash_id.clone(),
        )
        .expect("restore chart");
        let restored = PathBuf::from(restored);
        assert!(restored.join("chart.txt").is_file());
        assert!(restored.join(".unchartable.json").is_file());
        assert!(
            list_trashed_charts(temporary.path().to_string_lossy().to_string())
                .expect("empty trash")
                .is_empty()
        );

        trash_installed_chart(
            temporary.path().to_string_lossy().to_string(),
            chart_id.to_string(),
        )
        .expect("move restored chart to trash");
        assert_eq!(
            empty_chart_trash(temporary.path().to_string_lossy().to_string())
                .expect("empty chart trash"),
            1
        );
        assert!(
            list_trashed_charts(temporary.path().to_string_lossy().to_string())
                .expect("empty trash")
                .is_empty()
        );
    }

    #[test]
    fn migrates_the_legacy_trash_outside_custom_songs() {
        let temporary = tempdir().expect("temporary directory");
        let legacy = temporary
            .path()
            .join("CustomSongs")
            .join(".unchartable-trash");
        let private_trash = temporary.path().join("AppData").join("Trash");
        let trashed_chart = legacy.join("trash-id-Chart");
        std::fs::create_dir_all(&trashed_chart).expect("legacy trash chart");
        std::fs::write(trashed_chart.join("chart.txt"), b"chart").expect("legacy chart");

        migrate_legacy_trash(&legacy, &private_trash).expect("migrate legacy trash");

        assert!(!legacy.exists());
        assert!(
            private_trash
                .join("trash-id-Chart")
                .join("chart.txt")
                .is_file()
        );
    }

    #[test]
    fn atomically_replaces_a_managed_chart() {
        let temporary = tempdir().expect("temporary directory");
        let destination = temporary.path().join("Installed Song");
        let staging = temporary.path().join(".unchartable-test");
        let source = staging.join("Updated Song");
        std::fs::create_dir_all(&destination).expect("old install");
        std::fs::create_dir_all(&source).expect("updated install");
        std::fs::write(destination.join("old.txt"), b"old").expect("old file");
        std::fs::write(source.join("chart.txt"), b"new").expect("new chart");
        std::fs::write(source.join("audio.wav"), b"audio").expect("new audio");
        let metadata = InstallMetadata {
            chart_id: "503c0c85-d4b6-4366-b18b-bbcc6fb44f63".to_string(),
            title: "Updated Song".to_string(),
            artist: "Artist".to_string(),
            charter: "Charter".to_string(),
            archive_sha256: "new-hash".to_string(),
            source: "https://unchartable.site".to_string(),
            updated_at: Some("2026-07-23T13:00:00Z".to_string()),
            installed_at: Some("2026-07-23T13:01:00Z".to_string()),
            updates_enabled: true,
        };

        finalize_install(&source, &staging, &destination, &metadata).expect("replace installation");

        assert!(!destination.join("old.txt").exists());
        assert_eq!(
            std::fs::read(destination.join("chart.txt")).expect("updated chart"),
            b"new"
        );
        assert!(destination.join(".unchartable.json").is_file());
        assert!(!staging.exists());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(InstallRuntime::default())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().register_all()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            adopt_manual_chart,
            cancel_install,
            check_installed_updates,
            fetch_charts,
            find_manual_chart_matches,
            get_app_state,
            empty_chart_trash,
            fetch_chart,
            install_chart,
            launch_unbeatable,
            list_installed_charts,
            list_trashed_charts,
            open_custom_songs_folder,
            open_external_url,
            restore_trashed_chart,
            set_chart_updates,
            trash_installed_chart,
            validate_custom_songs_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running UNCHARTABLE");
}
