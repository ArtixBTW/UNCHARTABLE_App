use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncWriteExt;
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

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
    audio_duration_seconds: Option<f64>,
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupMetadata {
    backup_id: String,
    chart_id: String,
    created_at: String,
    folder_name: String,
    title: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupItem {
    backup_id: String,
    chart_id: String,
    created_at: String,
    folder_name: String,
    size_bytes: u64,
    title: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticReport {
    backup_count: usize,
    backup_size_bytes: u64,
    free_space_bytes: u64,
    invalid_charts: usize,
    managed_charts: usize,
    manual_charts: usize,
    path: String,
    path_writable: bool,
    total_charts: usize,
    total_size_bytes: u64,
    trash_count: usize,
    trash_size_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportArchiveInspection {
    archive_format: String,
    archive_path: String,
    archive_size_bytes: u64,
    artist: String,
    charter: String,
    conflict_folder_name: Option<String>,
    conflict_path: Option<String>,
    title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveFormat {
    Zip,
    SevenZip,
    Rar,
}

impl ArchiveFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
            Self::Rar => "rar",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepairReport {
    invalid_chart_paths: Vec<String>,
    removed_temporary_items: usize,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OperationRecord {
    action: String,
    created_at: String,
    detail: String,
    title: String,
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
    #[cfg(target_os = "windows")]
    {
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

    #[cfg(not(target_os = "windows"))]
    {
        app_handle()
            .path()
            .local_data_dir()
            .unwrap()
            .join("Steam")
            .join("steamapps")
            .join("compatdata")
            .join("2240620")
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("AppData")
            .join("LocalLow")
            .join("D-CELL GAMES")
            .join("UNBEATABLE")
            .join("CustomSongs")
    }
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
            audio_duration_seconds: manual_identity
                .as_ref()
                .and_then(|value| value.duration_seconds),
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

fn app_handle<'a>() -> &'a AppHandle {
    APP_HANDLE.get().unwrap()
}

fn local_data_dir() -> PathBuf {
    app_handle()
        .path()
        .local_data_dir()
        .unwrap_or_else(|_| PathBuf::from(std::env::temp_dir()))
        .join("UNCHARTABLE")
}

fn trash_directory(_target: &Path) -> PathBuf {
    #[cfg(test)]
    {
        _target.join(".unchartable-trash")
    }

    #[cfg(not(test))]
    {
        local_data_dir().join("Trash")
    }
}

fn backup_directory(_target: &Path) -> PathBuf {
    #[cfg(test)]
    {
        _target.join(".unchartable-backups")
    }

    #[cfg(not(test))]
    {
        local_data_dir().join("Backups")
    }
}

fn history_path(_target: &Path) -> PathBuf {
    #[cfg(test)]
    {
        _target.join(".unchartable-history.json")
    }

    #[cfg(not(test))]
    {
        local_data_dir().join("history.json")
    }
}

fn read_operation_history(target: &Path) -> Vec<OperationRecord> {
    fs::read(history_path(target))
        .ok()
        .and_then(|body| serde_json::from_slice(&body).ok())
        .unwrap_or_default()
}

fn append_operation(target: &Path, action: &str, title: &str, detail: &str) {
    let path = history_path(target);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut records = read_operation_history(target);
    records.insert(
        0,
        OperationRecord {
            action: action.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            detail: detail.to_string(),
            title: title.to_string(),
        },
    );
    records.truncate(500);
    if let Ok(body) = serde_json::to_vec_pretty(&records) {
        let _ = fs::write(path, body);
    }
}

fn prepare_backup_directory(target: &Path) -> Result<PathBuf, String> {
    let backups = backup_directory(target);
    fs::create_dir_all(&backups)
        .map_err(|error| format!("could not create the UNCHARTABLE backup folder: {error}"))?;
    Ok(backups)
}

fn backup_metadata(path: &Path) -> Option<BackupMetadata> {
    let body = fs::read(path.join(".unchartable-backup.json")).ok()?;
    serde_json::from_slice(&body).ok()
}

fn prune_chart_backups(backups: &Path, chart_id: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(backups) else {
        return;
    };
    let mut matching = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = backup_metadata(&path)?;
            (metadata.chart_id == chart_id).then_some((metadata.created_at, path))
        })
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, path) in matching.into_iter().skip(keep) {
        let _ = fs::remove_dir_all(path);
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

fn is_nested_archive(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "zip" | "7z" | "rar"
    )
}

fn safe_archive_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("the archive contains an unsafe file path.".to_string());
    }
    Ok(path.to_path_buf())
}

fn extracted_source(staging_path: &Path) -> Result<PathBuf, String> {
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
        let relative_path = safe_archive_path(&relative_path)?;
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
        if is_nested_archive(&relative_path) {
            return Err(format!(
                "nested archives are not installed into CustomSongs: {}",
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

    extracted_source(staging_path)
}

fn sevenz_error(message: impl Into<String>) -> sevenz_rust::Error {
    sevenz_rust::Error::io(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}

fn extract_7z_safely(archive_path: &Path, staging_path: &Path) -> Result<PathBuf, String> {
    let mut file_count = 0usize;
    let mut extracted_bytes = 0u64;
    sevenz_rust::decompress_file_with_extract_fn(
        archive_path,
        staging_path,
        |entry, reader, destination| {
            file_count = file_count
                .checked_add(1)
                .ok_or_else(|| sevenz_error("the archive file count is invalid."))?;
            if file_count > MAX_ARCHIVE_FILES {
                return Err(sevenz_error("the chart archive contains too many files."));
            }

            let relative_path = safe_archive_path(Path::new(entry.name())).map_err(sevenz_error)?;
            if contains_blocked_file(&relative_path) {
                return Err(sevenz_error(format!(
                    "blocked executable file in archive: {}",
                    relative_path.display()
                )));
            }
            if is_nested_archive(&relative_path) {
                return Err(sevenz_error(format!(
                    "nested archives are not installed into CustomSongs: {}",
                    relative_path.display()
                )));
            }
            extracted_bytes = extracted_bytes
                .checked_add(entry.size())
                .ok_or_else(|| sevenz_error("the extracted chart size is invalid."))?;
            if extracted_bytes > MAX_EXTRACTED_BYTES {
                return Err(sevenz_error(
                    "the chart expands beyond the safe extraction limit.",
                ));
            }

            let output_path = staging_path.join(&relative_path);
            if output_path != *destination {
                return Err(sevenz_error("the 7Z contains an unsafe file path."));
            }
            if entry.is_directory() {
                fs::create_dir_all(&output_path).map_err(sevenz_rust::Error::io)?;
                return Ok(true);
            }
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(sevenz_rust::Error::io)?;
            }
            let mut output = fs::File::create(&output_path).map_err(sevenz_rust::Error::io)?;
            io::copy(reader, &mut output).map_err(sevenz_rust::Error::io)?;
            output.flush().map_err(sevenz_rust::Error::io)?;
            Ok(true)
        },
    )
    .map_err(|error| format!("the downloaded file is not a valid 7Z archive: {error}"))?;
    extracted_source(staging_path)
}

fn extract_rar_safely(archive_path: &Path, staging_path: &Path) -> Result<PathBuf, String> {
    let mut archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .map_err(|error| format!("the downloaded file is not a valid RAR archive: {error}"))?;
    let mut file_count = 0usize;
    let mut extracted_bytes = 0u64;

    while let Some(header) = archive
        .read_header()
        .map_err(|error| format!("could not read the RAR archive: {error}"))?
    {
        let entry = header.entry();
        let relative_path = safe_archive_path(&entry.filename)?;
        if entry.is_encrypted() {
            return Err("password-protected RAR archives are not supported.".to_string());
        }
        if entry.is_split() {
            return Err("multi-part RAR archives are not supported.".to_string());
        }
        if contains_blocked_file(&relative_path) {
            return Err(format!(
                "blocked executable file in archive: {}",
                relative_path.display()
            ));
        }
        if is_nested_archive(&relative_path) {
            return Err(format!(
                "nested archives are not installed into CustomSongs: {}",
                relative_path.display()
            ));
        }
        file_count = file_count
            .checked_add(1)
            .ok_or_else(|| "the archive file count is invalid.".to_string())?;
        if file_count > MAX_ARCHIVE_FILES {
            return Err("the chart archive contains too many files.".to_string());
        }
        extracted_bytes = extracted_bytes
            .checked_add(entry.unpacked_size)
            .ok_or_else(|| "the extracted chart size is invalid.".to_string())?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err("the chart expands beyond the safe extraction limit.".to_string());
        }

        let output_path = staging_path.join(&relative_path);
        if entry.is_directory() {
            fs::create_dir_all(&output_path)
                .map_err(|error| format!("could not create chart directory: {error}"))?;
            archive = header
                .skip()
                .map_err(|error| format!("could not read the RAR archive: {error}"))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create chart directory: {error}"))?;
        }
        archive = header
            .extract_to(&output_path)
            .map_err(|error| format!("could not extract the RAR archive: {error}"))?;
        let metadata = fs::symlink_metadata(&output_path)
            .map_err(|error| format!("could not verify extracted chart file: {error}"))?;
        if metadata.file_type().is_symlink() {
            let _ = fs::remove_file(&output_path);
            return Err("symbolic links are not allowed in chart archives.".to_string());
        }
    }

    extracted_source(staging_path)
}

fn detect_archive_format(archive_path: &Path) -> Result<ArchiveFormat, String> {
    let mut file = fs::File::open(archive_path)
        .map_err(|error| format!("could not open the chart archive: {error}"))?;
    let mut signature = [0u8; 8];
    let bytes_read = file
        .read(&mut signature)
        .map_err(|error| format!("could not inspect the chart archive: {error}"))?;
    let signature = &signature[..bytes_read];
    if signature.starts_with(b"PK\x03\x04")
        || signature.starts_with(b"PK\x05\x06")
        || signature.starts_with(b"PK\x07\x08")
    {
        return Ok(ArchiveFormat::Zip);
    }
    if signature.starts_with(b"7z\xBC\xAF\x27\x1C") {
        return Ok(ArchiveFormat::SevenZip);
    }
    if signature.starts_with(b"Rar!\x1A\x07\x00") || signature.starts_with(b"Rar!\x1A\x07\x01\x00")
    {
        return Ok(ArchiveFormat::Rar);
    }
    Err("unsupported archive. Use a ZIP, 7Z, or RAR chart archive.".to_string())
}

fn extract_archive_safely(
    archive_path: &Path,
    staging_path: &Path,
    format: ArchiveFormat,
) -> Result<PathBuf, String> {
    match format {
        ArchiveFormat::Zip => extract_zip_safely(archive_path, staging_path),
        ArchiveFormat::SevenZip => extract_7z_safely(archive_path, staging_path),
        ArchiveFormat::Rar => extract_rar_safely(archive_path, staging_path),
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
        let target = destination
            .parent()
            .ok_or_else(|| "the chart destination has no parent folder.".to_string())?;
        let backups = prepare_backup_directory(target)?;
        let backup_id = Uuid::new_v4().to_string();
        let backup_destination = backups.join(&backup_id);
        fs::rename(&backup, &backup_destination).map_err(|error| {
            format!("chart installed, but its backup could not be saved: {error}")
        })?;
        let previous = read_install_metadata(&backup_destination);
        let backup_metadata = BackupMetadata {
            backup_id,
            chart_id: metadata.chart_id.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            folder_name: destination
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Restored Chart")
                .to_string(),
            title: previous
                .map(|value| value.title)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| metadata.title.clone()),
        };
        let body = serde_json::to_vec_pretty(&backup_metadata)
            .map_err(|error| format!("could not create backup metadata: {error}"))?;
        fs::write(backup_destination.join(".unchartable-backup.json"), body)
            .map_err(|error| format!("could not finish the chart backup: {error}"))?;
        prune_chart_backups(&backups, &metadata.chart_id, 3);
    }
    Ok(())
}

fn validate_local_archive(archive_path: &Path) -> Result<(u64, ArchiveFormat), String> {
    if !archive_path.is_file() {
        return Err("choose an existing chart archive.".to_string());
    }
    let size = fs::metadata(archive_path)
        .map_err(|error| format!("could not inspect the archive: {error}"))?
        .len();
    if size > MAX_ARCHIVE_BYTES {
        return Err("the chart archive exceeds the 250 MB limit.".to_string());
    }
    Ok((size, detect_archive_format(archive_path)?))
}

fn matching_local_chart(target: &Path, identity: &ManualChartIdentity) -> Option<PathBuf> {
    let title = normalized_match_value(&identity.title);
    let artist = normalized_match_value(&identity.artist);
    scan_installed(target).ok()?.into_iter().find_map(|chart| {
        if normalized_match_value(&chart.title) != title
            || normalized_match_value(chart.artist.as_deref().unwrap_or_default()) != artist
        {
            return None;
        }
        let local_creators = chart
            .manual_identity
            .as_ref()
            .map(|value| value.creators.clone())
            .unwrap_or_else(|| {
                chart
                    .charter
                    .as_deref()
                    .map(normalized_match_value)
                    .into_iter()
                    .filter(|value| !value.is_empty())
                    .collect()
            });
        let creators_overlap = identity.creators.is_empty()
            || local_creators.is_empty()
            || identity
                .creators
                .iter()
                .any(|creator| local_creators.contains(creator));
        creators_overlap.then(|| PathBuf::from(chart.path))
    })
}

fn inspect_archive_for_import(
    archive_path: &Path,
    target: &Path,
) -> Result<(ImportArchiveInspection, PathBuf, PathBuf), String> {
    validate_target_directory(target)?;
    let (archive_size_bytes, archive_format) = validate_local_archive(archive_path)?;
    let staging = target.join(format!(".unchartable-import-{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&staging)
        .map_err(|error| format!("could not create the import workspace: {error}"))?;
    let source = match extract_archive_safely(archive_path, &staging, archive_format) {
        Ok(source) => source,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let inspection = inspect_local_chart(&source, true);
    if !inspection.has_chart || !inspection.has_audio {
        let _ = fs::remove_dir_all(&staging);
        return Err(match (inspection.has_chart, inspection.has_audio) {
            (false, false) => "the archive contains neither chart .txt files nor supported audio.",
            (false, true) => "the archive contains audio but no chart .txt file.",
            (true, false) => "the archive contains chart data but no supported audio file.",
            (true, true) => unreachable!(),
        }
        .to_string());
    }
    if inspection.identity.title.is_empty() || inspection.identity.artist.is_empty() {
        let _ = fs::remove_dir_all(&staging);
        return Err("the chart TXT is missing a title or artist.".to_string());
    }
    let conflict = matching_local_chart(target, &inspection.identity);
    let conflict_folder_name = conflict
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned);
    let result = ImportArchiveInspection {
        archive_format: archive_format.label().to_string(),
        archive_path: archive_path.to_string_lossy().to_string(),
        archive_size_bytes,
        artist: inspection.identity.artist.clone(),
        charter: inspection
            .identity
            .creator_label
            .clone()
            .unwrap_or_else(|| "unknown charter".to_string()),
        conflict_folder_name,
        conflict_path: conflict.map(|path| path.to_string_lossy().to_string()),
        title: inspection.identity.title.clone(),
    };
    Ok((result, staging, source))
}

#[tauri::command]
fn inspect_chart_archive(
    archive_path: String,
    target_directory: String,
) -> Result<ImportArchiveInspection, String> {
    let target = PathBuf::from(target_directory);
    let (inspection, staging, _) = inspect_archive_for_import(Path::new(&archive_path), &target)?;
    let _ = fs::remove_dir_all(staging);
    Ok(inspection)
}

#[tauri::command]
fn import_chart_archive(
    archive_path: String,
    target_directory: String,
    allow_duplicate: bool,
) -> Result<String, String> {
    let target = PathBuf::from(target_directory);
    let (inspection, staging, source) =
        inspect_archive_for_import(Path::new(&archive_path), &target)?;
    if inspection.conflict_path.is_some() && !allow_duplicate {
        let _ = fs::remove_dir_all(staging);
        return Err("a matching chart is already installed.".to_string());
    }
    let folder_name =
        install_folder_name(&source, &staging, &inspection.title, &inspection.charter);
    let destination = new_install_path(
        &target,
        &folder_name,
        &inspection.title,
        &inspection.charter,
    );
    if let Err(error) = fs::rename(&source, &destination) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("could not install the imported chart: {error}"));
    }
    let _ = fs::remove_dir_all(staging);
    append_operation(
        &target,
        "import",
        &inspection.title,
        &format!("Imported from {}", inspection.archive_path),
    );
    Ok(destination.to_string_lossy().to_string())
}

fn is_repairable_temporary_item(name: &str) -> bool {
    if name.starts_with(".unchartable-import-") || name.starts_with(".unchartable-write-test-") {
        return true;
    }
    let Some(operation_id) = name.strip_prefix(".unchartable-").map(|value| {
        value
            .strip_suffix(".zip")
            .or_else(|| value.strip_suffix(".archive"))
            .unwrap_or(value)
    }) else {
        return false;
    };
    Uuid::parse_str(operation_id).is_ok()
}

#[tauri::command]
fn repair_library(path: String) -> Result<RepairReport, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let mut removed_temporary_items = 0;
    for entry in fs::read_dir(&target)
        .map_err(|error| format!("could not inspect CustomSongs: {error}"))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_repairable_temporary_item(&name) {
            continue;
        }
        let result = if entry.path().is_dir() {
            fs::remove_dir_all(entry.path())
        } else {
            fs::remove_file(entry.path())
        };
        if result.is_ok() {
            removed_temporary_items += 1;
        }
    }
    let invalid_chart_paths = scan_installed(&target)?
        .into_iter()
        .filter(|chart| !chart.playable)
        .map(|chart| chart.path)
        .collect::<Vec<_>>();
    append_operation(
        &target,
        "repair",
        "Library repair",
        &format!(
            "Removed {removed_temporary_items} temporary items; found {} invalid chart folders",
            invalid_chart_paths.len()
        ),
    );
    Ok(RepairReport {
        invalid_chart_paths,
        removed_temporary_items,
    })
}

#[tauri::command]
fn list_operation_history(path: String) -> Result<Vec<OperationRecord>, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    Ok(read_operation_history(&target))
}

fn add_directory_to_zip(
    writer: &mut zip::ZipWriter<File>,
    root: &Path,
    current: &Path,
    prefix: &str,
) -> Result<(), String> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for entry in fs::read_dir(current)
        .map_err(|error| format!("could not read a chart folder: {error}"))?
        .flatten()
    {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "could not build the pack archive path.".to_string())?;
        let name = format!("{prefix}/{}", relative.to_string_lossy().replace('\\', "/"));
        if path.is_dir() {
            writer
                .add_directory(format!("{name}/"), options)
                .map_err(|error| format!("could not add a pack folder: {error}"))?;
            add_directory_to_zip(writer, root, &path, prefix)?;
        } else {
            writer
                .start_file(name, options)
                .map_err(|error| format!("could not add a pack file: {error}"))?;
            let mut source = File::open(&path)
                .map_err(|error| format!("could not read a chart file: {error}"))?;
            io::copy(&mut source, writer)
                .map_err(|error| format!("could not write a pack file: {error}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
fn export_local_pack(
    path: String,
    output_path: String,
    chart_paths: Vec<String>,
    name: String,
) -> Result<String, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    if chart_paths.is_empty() {
        return Err("select at least one installed chart.".to_string());
    }
    let output = PathBuf::from(output_path);
    if !output
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("zip"))
    {
        return Err("local packs must be exported as ZIP files.".to_string());
    }
    let file = File::create(&output)
        .map_err(|error| format!("could not create the pack archive: {error}"))?;
    let mut writer = zip::ZipWriter::new(file);
    let chart_count = chart_paths.len();
    for chart_path in chart_paths {
        let chart = PathBuf::from(chart_path);
        if !is_direct_child(&target, &chart) || !chart.is_dir() {
            return Err("refusing to export a folder outside CustomSongs.".to_string());
        }
        let folder_name = chart
            .file_name()
            .and_then(|value| value.to_str())
            .map(sanitize_folder_name)
            .unwrap_or_else(|| "Chart".to_string());
        add_directory_to_zip(&mut writer, &chart, &chart, &folder_name)?;
    }
    writer
        .finish()
        .map_err(|error| format!("could not finish the pack archive: {error}"))?;
    append_operation(
        &target,
        "export",
        &name,
        &format!("Exported {chart_count} installed charts"),
    );
    Ok(output.to_string_lossy().to_string())
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
    let target = PathBuf::from(&path);
    validate_target_directory(&target)?;

    app_handle()
        .opener()
        .open_path(&path, None::<&str>)
        .map_err(|error| {
            format!(
                "could not open the custom songs directory: {}",
                error.to_string()
            )
        })
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let parsed = parse_allowed_external_url(&url)?;

    app_handle()
        .opener()
        .open_path(parsed, None::<&str>)
        .map_err(|error| {
            format!(
                "could not open the UNCHARTABLE website: {}",
                error.to_string()
            )
        })
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
    let title = read_install_metadata(&source)
        .map(|metadata| metadata.title)
        .unwrap_or_else(|| "Chart".to_string());
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
        .map_err(|error| format!("could not finish moving the chart to trash: {error}"))?;
    append_operation(&target, "remove", &title, "Moved to the UNCHARTABLE trash");
    Ok(())
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
        append_operation(
            &target,
            "restore",
            &metadata.original_folder_name,
            "Restored from trash",
        );
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

#[tauri::command]
fn list_chart_backups(path: String) -> Result<Vec<BackupItem>, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let backups = prepare_backup_directory(&target)?;
    let mut items = fs::read_dir(&backups)
        .map_err(|error| format!("could not scan chart backups: {error}"))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = backup_metadata(&path)?;
            Some(BackupItem {
                backup_id: metadata.backup_id,
                chart_id: metadata.chart_id,
                created_at: metadata.created_at,
                folder_name: metadata.folder_name,
                size_bytes: directory_size(&path),
                title: metadata.title,
            })
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(items)
}

#[tauri::command]
fn restore_chart_backup(path: String, backup_id: String) -> Result<String, String> {
    validate_chart_id(&backup_id)?;
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let backups = prepare_backup_directory(&target)?;
    let source = fs::read_dir(&backups)
        .map_err(|error| format!("could not scan chart backups: {error}"))?
        .flatten()
        .find_map(|entry| {
            let path = entry.path();
            backup_metadata(&path)
                .filter(|metadata| metadata.backup_id == backup_id)
                .map(|metadata| (path, metadata))
        })
        .ok_or_else(|| "this backup could not be found.".to_string())?;
    let (source_path, metadata) = source;
    let current = find_existing_install(&target, &metadata.chart_id);
    let destination = current
        .clone()
        .unwrap_or_else(|| unique_install_path(&target, &metadata.folder_name));

    if let Some(current_path) = current {
        let replacement_id = Uuid::new_v4().to_string();
        let replacement = backups.join(&replacement_id);
        fs::rename(&current_path, &replacement)
            .map_err(|error| format!("could not preserve the current chart version: {error}"))?;
        let current_metadata = read_install_metadata(&replacement);
        let body = serde_json::to_vec_pretty(&BackupMetadata {
            backup_id: replacement_id,
            chart_id: metadata.chart_id.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            folder_name: metadata.folder_name.clone(),
            title: current_metadata
                .map(|value| value.title)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| metadata.title.clone()),
        })
        .map_err(|error| format!("could not create replacement backup metadata: {error}"))?;
        fs::write(replacement.join(".unchartable-backup.json"), body)
            .map_err(|error| format!("could not preserve the current chart version: {error}"))?;
    }

    let _ = fs::remove_file(source_path.join(".unchartable-backup.json"));
    if let Err(error) = fs::rename(&source_path, &destination) {
        return Err(format!(
            "could not restore the selected chart version: {error}"
        ));
    }
    prune_chart_backups(&backups, &metadata.chart_id, 3);
    append_operation(
        &target,
        "rollback",
        &metadata.title,
        "Restored a previous chart version",
    );
    Ok(destination.to_string_lossy().to_string())
}

#[tauri::command]
fn delete_chart_backup(path: String, backup_id: String) -> Result<(), String> {
    validate_chart_id(&backup_id)?;
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let backups = prepare_backup_directory(&target)?;
    let item = fs::read_dir(&backups)
        .map_err(|error| format!("could not scan chart backups: {error}"))?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| backup_metadata(path).is_some_and(|metadata| metadata.backup_id == backup_id))
        .ok_or_else(|| "this backup could not be found.".to_string())?;
    fs::remove_dir_all(item).map_err(|error| format!("could not delete this backup: {error}"))
}

#[tauri::command]
fn diagnose_library(path: String) -> Result<DiagnosticReport, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let installed = scan_installed(&target)?;
    let trash = list_trashed_charts(target.to_string_lossy().to_string())?;
    let backups = list_chart_backups(target.to_string_lossy().to_string())?;
    let write_probe = target.join(format!(".unchartable-write-test-{}", Uuid::new_v4()));
    let path_writable = fs::write(&write_probe, b"ok").is_ok();
    let _ = fs::remove_file(write_probe);
    Ok(DiagnosticReport {
        backup_count: backups.len(),
        backup_size_bytes: backups.iter().map(|item| item.size_bytes).sum(),
        free_space_bytes: fs2::available_space(&target).unwrap_or(0),
        invalid_charts: installed.iter().filter(|item| !item.playable).count(),
        managed_charts: installed.iter().filter(|item| item.managed).count(),
        manual_charts: installed.iter().filter(|item| !item.managed).count(),
        path: target.to_string_lossy().to_string(),
        path_writable,
        total_charts: installed.len(),
        total_size_bytes: installed.iter().map(|item| item.size_bytes).sum(),
        trash_count: trash.len(),
        trash_size_bytes: trash.iter().map(|item| item.size_bytes).sum(),
    })
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
                    .append_pair("pageSize", "100")
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
fn set_all_chart_updates(path: String, enabled: bool) -> Result<usize, String> {
    let target = PathBuf::from(path);
    validate_target_directory(&target)?;
    let entries =
        fs::read_dir(&target).map_err(|error| format!("could not scan CustomSongs: {error}"))?;
    let mut managed = 0usize;

    for entry in entries.flatten() {
        let installed_path = entry.path();
        if !installed_path.is_dir() || !is_direct_child(&target, &installed_path) {
            continue;
        }
        let Some(mut metadata) = read_install_metadata(&installed_path) else {
            continue;
        };
        managed += 1;
        if metadata.updates_enabled == enabled {
            continue;
        }
        metadata.updates_enabled = enabled;
        let body = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| format!("could not update chart preferences: {error}"))?;
        fs::write(installed_path.join(".unchartable.json"), body)
            .map_err(|error| format!("could not save chart preferences: {error}"))?;
    }

    Ok(managed)
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
    app_handle()
        .opener()
        .open_path("steam://run/2240620", None::<&str>)
        .map_err(|error| {
            format!(
                "could not ask Steam to launch UNBEATABLE: {}",
                error.to_string()
            )
        })
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
async fn fetch_packs(page: u32, query: String) -> Result<serde_json::Value, String> {
    let mut url =
        Url::parse(&format!("{API_ORIGIN}/api/packs")).map_err(|error| error.to_string())?;
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("page", &page.to_string());
        params.append_pair("pageSize", "12");
        params.append_pair("charts", "all");
        if !query.trim().is_empty() {
            params.append_pair("q", query.trim());
        }
    }
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|error| format!("could not prepare the pack request: {error}"))?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("could not reach unchartable.site: {error}"))?
        .error_for_status()
        .map_err(|error| format!("UNCHARTABLE returned an error: {error}"))?
        .json()
        .await
        .map_err(|error| format!("could not read the pack catalog: {error}"))
}

#[tauri::command]
async fn fetch_pack(pack_id: String) -> Result<serde_json::Value, String> {
    validate_chart_id(&pack_id)?;
    let payload = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|error| format!("could not prepare the pack request: {error}"))?
        .get(format!("{API_ORIGIN}/api/packs/{pack_id}"))
        .send()
        .await
        .map_err(|error| format!("could not reach unchartable.site: {error}"))?
        .error_for_status()
        .map_err(|error| format!("UNCHARTABLE returned an error: {error}"))?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("could not read the pack: {error}"))?;

    payload
        .get("pack")
        .cloned()
        .ok_or_else(|| "UNCHARTABLE did not return a pack.".to_string())
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

fn get_or_create_installation_id(app: &AppHandle) -> Result<String, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not locate app data: {error}"))?;
    fs::create_dir_all(&app_data)
        .map_err(|error| format!("could not prepare app data: {error}"))?;
    let path = app_data.join("installation-id");
    if let Ok(value) = fs::read_to_string(&path) {
        let value = value.trim();
        if Uuid::parse_str(value).is_ok() {
            return Ok(value.to_string());
        }
    }

    let installation_id = Uuid::new_v4().to_string();
    fs::write(&path, &installation_id)
        .map_err(|error| format!("could not save the app installation id: {error}"))?;
    Ok(installation_id)
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
    let installation_id = get_or_create_installation_id(&app)?;

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

    let ticket_url = format!("{API_ORIGIN}/api/charts/{chart_id}/download?format=json&source=app");
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(4))
        .user_agent(format!(
            "UNCHARTABLE-App/{} (+https://unchartable.site)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| format!("could not prepare secure download: {error}"))?;
    let ticket: DownloadTicket = client
        .get(ticket_url)
        .header("x-unchartable-device-id", installation_id)
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
    let total_bytes = response.content_length();
    if total_bytes.is_some_and(|size| size > MAX_ARCHIVE_BYTES) {
        return Err("the chart archive exceeds the 250 MB download limit.".to_string());
    }

    let operation_id = Uuid::new_v4();
    let archive_path = target.join(format!(".unchartable-{operation_id}.archive"));
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
    let archive_format = match detect_archive_format(&archive_path) {
        Ok(format) => format,
        Err(error) => {
            let _ = fs::remove_file(&archive_path);
            return Err(error);
        }
    };
    fs::create_dir_all(&staging_path)
        .map_err(|error| format!("could not create the temporary install directory: {error}"))?;
    let source_path = extract_archive_safely(&archive_path, &staging_path, archive_format);
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
    let is_update = existing.is_some();
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
    append_operation(
        &target,
        if is_update { "update" } else { "install" },
        &title,
        if is_update {
            "Installed a newer version from unchartable.site"
        } else {
            "Installed from unchartable.site"
        },
    );

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
        ArchiveFormat, InstallMetadata, MAX_ARCHIVE_BYTES, app_state_for_path, backup_directory,
        conflicting_install_folder_name, contains_blocked_file, detect_archive_format,
        empty_chart_trash, export_local_pack, extract_archive_safely, extract_zip_safely,
        finalize_install, generated_install_folder_name, import_chart_archive,
        inspect_chart_archive, inspect_chart_structure, inspect_manual_chart_identity,
        install_folder_name, is_nested_archive, is_repairable_temporary_item, list_chart_backups,
        list_operation_history, list_trashed_charts, migrate_legacy_trash, new_install_path,
        parse_allowed_external_url, repair_library, restore_chart_backup, restore_trashed_chart,
        sanitize_folder_name, scan_installed, set_all_chart_updates, trash_installed_chart,
        unique_manual_match, validate_local_archive,
    };
    use std::{
        fs::File,
        io::Write,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;
    use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

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
        assert!(is_nested_archive(Path::new("chart/original.zip")));
        assert!(is_nested_archive(Path::new("chart/original.7Z")));
        assert!(is_nested_archive(Path::new("chart/original.rar")));
        assert!(!is_nested_archive(Path::new("chart/song.wav")));
    }

    #[test]
    fn detects_supported_archives_and_enforces_the_import_size_limit() {
        let temporary = tempdir().expect("temporary directory");
        let wrong_contents = temporary.path().join("chart.rar");
        std::fs::write(&wrong_contents, b"archive").expect("fake archive");
        assert!(
            validate_local_archive(&wrong_contents)
                .expect_err("invalid archive should be rejected")
                .contains("unsupported archive")
        );

        let oversized = temporary.path().join("oversized.zip");
        let oversized_file = File::create(&oversized).expect("oversized fixture");
        oversized_file
            .set_len(MAX_ARCHIVE_BYTES + 1)
            .expect("sparse oversized fixture");
        assert!(
            validate_local_archive(&oversized)
                .expect_err("oversized archive should be rejected")
                .contains("250 MB")
        );

        let zip_path = temporary.path().join("chart.data");
        let zip_file = File::create(&zip_path).expect("zip file");
        ZipWriter::new(zip_file).finish().expect("empty zip");
        assert_eq!(
            validate_local_archive(&zip_path).expect("ZIP signature").1,
            ArchiveFormat::Zip
        );

        let seven_path = temporary.path().join("chart.7z");
        std::fs::write(&seven_path, b"7z\xBC\xAF\x27\x1C\0\0").expect("7Z signature");
        assert_eq!(
            detect_archive_format(&seven_path).expect("7Z signature"),
            ArchiveFormat::SevenZip
        );

        let rar_path = temporary.path().join("chart.rar");
        std::fs::write(&rar_path, b"Rar!\x1A\x07\x01\x00").expect("RAR signature");
        assert_eq!(
            detect_archive_format(&rar_path).expect("RAR signature"),
            ArchiveFormat::Rar
        );
    }

    #[test]
    #[ignore = "requires UNCHARTABLE_ARCHIVE_FIXTURE"]
    fn extracts_a_real_supported_archive_fixture() {
        let fixture = std::env::var("UNCHARTABLE_ARCHIVE_FIXTURE")
            .expect("UNCHARTABLE_ARCHIVE_FIXTURE must point to an archive");
        let temporary = tempdir().expect("temporary directory");
        let staging = temporary.path().join("staging");
        std::fs::create_dir_all(&staging).expect("staging directory");
        let format = detect_archive_format(Path::new(&fixture)).expect("supported archive");
        let source = extract_archive_safely(Path::new(&fixture), &staging, format)
            .expect("safe archive extraction");
        let (has_chart, has_audio) = inspect_chart_structure(&source);
        assert!(has_chart, "fixture should contain chart TXT data");
        assert!(has_audio, "fixture should contain supported audio");
    }

    #[test]
    fn recognizes_only_owned_temporary_items_for_repair() {
        assert!(is_repairable_temporary_item(
            ".unchartable-import-abandoned"
        ));
        assert!(is_repairable_temporary_item(
            ".unchartable-503c0c85-d4b6-4366-b18b-bbcc6fb44f63.zip"
        ));
        assert!(is_repairable_temporary_item(
            ".unchartable-503c0c85-d4b6-4366-b18b-bbcc6fb44f63.archive"
        ));
        assert!(!is_repairable_temporary_item(".unchartable-history.json"));
        assert!(!is_repairable_temporary_item(".unchartable-backups"));
        assert!(!is_repairable_temporary_item(".unchartable-user-folder"));
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
    fn rejects_imports_with_executables_or_incomplete_chart_contents() {
        let temporary = tempdir().expect("temporary directory");
        let custom_songs = temporary.path().join("CustomSongs");
        std::fs::create_dir_all(&custom_songs).expect("CustomSongs");

        let unsafe_archive = temporary.path().join("unsafe.zip");
        let unsafe_file = File::create(&unsafe_archive).expect("unsafe archive");
        let mut unsafe_zip = ZipWriter::new(unsafe_file);
        unsafe_zip
            .start_file("chart.txt", SimpleFileOptions::default())
            .expect("chart entry");
        unsafe_zip
            .write_all(b"Title:Unsafe\nArtist:Test\nCreator:Alice")
            .expect("chart data");
        unsafe_zip
            .start_file("audio.wav", SimpleFileOptions::default())
            .expect("audio entry");
        unsafe_zip.write_all(b"audio").expect("audio data");
        unsafe_zip
            .start_file("setup.exe", SimpleFileOptions::default())
            .expect("executable entry");
        unsafe_zip
            .write_all(b"executable")
            .expect("executable data");
        unsafe_zip.finish().expect("finish unsafe archive");
        assert!(
            inspect_chart_archive(
                unsafe_archive.to_string_lossy().to_string(),
                custom_songs.to_string_lossy().to_string(),
            )
            .expect_err("executable payload should be rejected")
            .contains("blocked executable")
        );

        let chart_only_archive = temporary.path().join("chart-only.zip");
        let chart_only_file = File::create(&chart_only_archive).expect("chart-only archive");
        let mut chart_only_zip = ZipWriter::new(chart_only_file);
        chart_only_zip
            .start_file("chart.txt", SimpleFileOptions::default())
            .expect("chart entry");
        chart_only_zip
            .write_all(b"Title:No Audio\nArtist:Test\nCreator:Alice")
            .expect("chart data");
        chart_only_zip.finish().expect("finish chart-only archive");
        assert!(
            inspect_chart_archive(
                chart_only_archive.to_string_lossy().to_string(),
                custom_songs.to_string_lossy().to_string(),
            )
            .expect_err("chart without audio should be rejected")
            .contains("no supported audio")
        );

        let audio_only_archive = temporary.path().join("audio-only.zip");
        let audio_only_file = File::create(&audio_only_archive).expect("audio-only archive");
        let mut audio_only_zip = ZipWriter::new(audio_only_file);
        audio_only_zip
            .start_file("audio.wav", SimpleFileOptions::default())
            .expect("audio entry");
        audio_only_zip.write_all(b"audio").expect("audio data");
        audio_only_zip.finish().expect("finish audio-only archive");
        assert!(
            inspect_chart_archive(
                audio_only_archive.to_string_lossy().to_string(),
                custom_songs.to_string_lossy().to_string(),
            )
            .expect_err("audio without chart should be rejected")
            .contains("no chart .txt")
        );
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
    fn enables_updates_for_every_managed_chart_without_claiming_manual_charts() {
        let temporary = tempdir().expect("temporary directory");
        let managed = temporary.path().join("Managed Song");
        let manual = temporary.path().join("Manual Song");
        std::fs::create_dir_all(&managed).expect("managed directory");
        std::fs::create_dir_all(&manual).expect("manual directory");
        let metadata = InstallMetadata {
            chart_id: "503c0c85-d4b6-4366-b18b-bbcc6fb44f63".to_string(),
            title: "Managed Song".to_string(),
            artist: "Artist".to_string(),
            charter: "Charter".to_string(),
            archive_sha256: "hash".to_string(),
            source: "https://unchartable.site".to_string(),
            updated_at: None,
            installed_at: None,
            updates_enabled: false,
        };
        std::fs::write(
            managed.join(".unchartable.json"),
            serde_json::to_vec(&metadata).expect("metadata"),
        )
        .expect("metadata file");

        assert_eq!(
            set_all_chart_updates(temporary.path().to_string_lossy().to_string(), true)
                .expect("bulk updates"),
            1
        );
        assert!(
            super::read_install_metadata(&managed)
                .expect("updated metadata")
                .updates_enabled
        );
        assert!(!manual.join(".unchartable.json").exists());
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

        let backups = list_chart_backups(temporary.path().to_string_lossy().to_string())
            .expect("list saved versions");
        assert_eq!(backups.len(), 1);
        let saved = &backups[0];
        assert!(
            backup_directory(temporary.path())
                .join(&saved.backup_id)
                .join("old.txt")
                .is_file()
        );

        restore_chart_backup(
            temporary.path().to_string_lossy().to_string(),
            saved.backup_id.clone(),
        )
        .expect("restore previous version");
        assert!(destination.join("old.txt").is_file());
        assert!(!destination.join("chart.txt").exists());
    }

    #[test]
    fn imports_detects_conflicts_exports_and_repairs_local_charts() {
        let temporary = tempdir().expect("temporary directory");
        let custom_songs = temporary.path().join("CustomSongs");
        std::fs::create_dir_all(&custom_songs).expect("CustomSongs");
        let archive_path = temporary.path().join("manual-chart.zip");
        let archive_file = File::create(&archive_path).expect("archive file");
        let mut archive = ZipWriter::new(archive_file);
        archive
            .start_file("Manual Song/chart.txt", SimpleFileOptions::default())
            .expect("chart entry");
        archive
            .write_all(
                b"[Metadata]\nTitle:Manual Song\nArtist:Manual Artist\nCreator:Alice\nTags:{\"SongLength\":120}",
            )
            .expect("chart contents");
        archive
            .start_file("Manual Song/audio.wav", SimpleFileOptions::default())
            .expect("audio entry");
        archive.write_all(b"audio").expect("audio contents");
        archive.finish().expect("finish archive");

        let target = custom_songs.to_string_lossy().to_string();
        let first_inspection =
            inspect_chart_archive(archive_path.to_string_lossy().to_string(), target.clone())
                .expect("inspect import");
        assert_eq!(first_inspection.title, "Manual Song");
        assert!(first_inspection.conflict_path.is_none());

        let installed = import_chart_archive(
            archive_path.to_string_lossy().to_string(),
            target.clone(),
            false,
        )
        .expect("import chart");
        let installed = PathBuf::from(installed);
        assert!(installed.join("chart.txt").is_file());
        assert!(installed.join("audio.wav").is_file());
        assert_eq!(
            installed.file_name().and_then(|name| name.to_str()),
            Some("Manual Song")
        );
        assert!(
            archive_path.is_file(),
            "the original archive should remain at its source"
        );
        assert!(
            std::fs::read_dir(&installed)
                .expect("installed chart directory")
                .filter_map(Result::ok)
                .all(|entry| !is_nested_archive(&entry.path())),
            "the installed chart should contain extracted files only"
        );

        let conflict =
            inspect_chart_archive(archive_path.to_string_lossy().to_string(), target.clone())
                .expect("inspect conflict");
        assert_eq!(
            conflict.conflict_path.as_deref(),
            Some(installed.to_string_lossy().as_ref())
        );
        assert!(
            import_chart_archive(
                archive_path.to_string_lossy().to_string(),
                target.clone(),
                false,
            )
            .expect_err("matching chart should require an explicit choice")
            .contains("already installed")
        );
        let duplicate = import_chart_archive(
            archive_path.to_string_lossy().to_string(),
            target.clone(),
            true,
        )
        .expect("keep both");
        assert_ne!(PathBuf::from(&duplicate), installed);
        assert!(PathBuf::from(duplicate).join("chart.txt").is_file());

        let export_path = temporary.path().join("local-pack.zip");
        export_local_pack(
            target.clone(),
            export_path.to_string_lossy().to_string(),
            vec![installed.to_string_lossy().to_string()],
            "Test Pack".to_string(),
        )
        .expect("export local pack");
        assert!(export_path.is_file());
        let exported = ZipArchive::new(File::open(export_path).expect("pack archive"))
            .expect("valid pack ZIP");
        assert!(exported.len() >= 2);

        let abandoned = custom_songs.join(".unchartable-import-abandoned");
        let preserved = custom_songs.join(".unchartable-user-folder");
        let invalid = custom_songs.join("Incomplete Song");
        std::fs::create_dir_all(&abandoned).expect("abandoned temporary directory");
        std::fs::create_dir_all(&preserved).expect("preserved hidden directory");
        std::fs::create_dir_all(&invalid).expect("invalid chart directory");
        std::fs::write(invalid.join("chart.txt"), b"chart").expect("invalid chart");

        let report = repair_library(target.clone()).expect("repair library");
        assert_eq!(report.removed_temporary_items, 1);
        assert!(!abandoned.exists());
        assert!(preserved.exists());
        assert!(
            report
                .invalid_chart_paths
                .iter()
                .any(|path| Path::new(path) == invalid)
        );

        let history = list_operation_history(target).expect("operation history");
        assert!(history.iter().any(|record| record.action == "import"));
        assert!(history.iter().any(|record| record.action == "export"));
        assert!(history.iter().any(|record| record.action == "repair"));
    }

    #[test]
    fn imports_rootless_archive_into_a_folder_named_from_chart_metadata() {
        let temporary = tempdir().expect("temporary directory");
        let custom_songs = temporary.path().join("CustomSongs");
        std::fs::create_dir_all(&custom_songs).expect("CustomSongs");
        let archive_path = temporary.path().join("rootless.zip");
        let archive_file = File::create(&archive_path).expect("archive file");
        let mut archive = ZipWriter::new(archive_file);
        archive
            .start_file("chart.txt", SimpleFileOptions::default())
            .expect("chart entry");
        archive
            .write_all(
                b"[Metadata]\nTitle:Rootless Song\nArtist:Manual Artist\nCreator:Alice\nTags:{\"SongLength\":120}",
            )
            .expect("chart contents");
        archive
            .start_file("audio.wav", SimpleFileOptions::default())
            .expect("audio entry");
        archive.write_all(b"audio").expect("audio contents");
        archive.finish().expect("finish archive");

        let installed = PathBuf::from(
            import_chart_archive(
                archive_path.to_string_lossy().to_string(),
                custom_songs.to_string_lossy().to_string(),
                false,
            )
            .expect("import rootless chart"),
        );

        assert_eq!(
            installed.file_name().and_then(|name| name.to_str()),
            Some("Rootless Song")
        );
        assert!(installed.join("chart.txt").is_file());
        assert!(installed.join("audio.wav").is_file());
        assert!(
            archive_path.is_file(),
            "the original archive should remain at its source"
        );
        assert!(
            std::fs::read_dir(&installed)
                .expect("installed chart directory")
                .filter_map(Result::ok)
                .all(|entry| !is_nested_archive(&entry.path())),
            "the installed chart should contain extracted files only"
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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

            let _ = APP_HANDLE.set(app.app_handle().to_owned());

            let show = MenuItem::with_id(app, "show", "Open UNCHARTABLE", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .ok_or("missing app icon")?,
                )
                .tooltip("UNCHARTABLE")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            adopt_manual_chart,
            cancel_install,
            check_installed_updates,
            fetch_charts,
            find_manual_chart_matches,
            get_app_state,
            empty_chart_trash,
            delete_chart_backup,
            diagnose_library,
            export_local_pack,
            fetch_chart,
            fetch_pack,
            install_chart,
            import_chart_archive,
            inspect_chart_archive,
            fetch_packs,
            launch_unbeatable,
            list_chart_backups,
            list_installed_charts,
            list_operation_history,
            list_trashed_charts,
            open_custom_songs_folder,
            open_external_url,
            repair_library,
            restore_trashed_chart,
            restore_chart_backup,
            set_all_chart_updates,
            set_chart_updates,
            trash_installed_chart,
            validate_custom_songs_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running UNCHARTABLE");
}
