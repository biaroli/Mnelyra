use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::codex;
use crate::error::{AppError, AppResult};

const ROUTE_URL: &str = "http://127.0.0.1:17841/v1";
const MANAGED_COMMENT: &str = "# Managed by Mnelyra Web Models";
const LEGACY_MODEL_CATALOG_KEY: &str = "model_catalog_json";
const LEGACY_MODEL_CATALOG_FILE: &str = "model-catalog.json";
const NATIVE_WEB_MODEL: &str = "gpt-5.6-sol";
const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviousAssignment {
    present: bool,
    raw_line: Option<String>,
    index: Option<usize>,
}

fn migrate_legacy_model_selection(lines: &mut [String]) -> bool {
    let Some((index, line)) = top_level_assignment(lines, "model") else {
        return false;
    };
    let Some(model) = assignment_value(&line, "model") else {
        return false;
    };
    if !model.starts_with("mnelyra-web/") {
        return false;
    }

    let replacement = Regex::new(r#"^(\s*model\s*=\s*)\"mnelyra-web/[^\"]*\"(\s*(?:#.*)?)$"#)
        .expect("valid legacy model regex")
        .replace(&line, format!("${{1}}\"{NATIVE_WEB_MODEL}\"${{2}}"));
    lines[index] = replacement.into_owned();
    true
}

fn install_assignment(
    lines: &mut Vec<String>,
    key: &str,
    installed_line: &str,
) -> PreviousAssignment {
    if let Some((index, raw_line)) = top_level_assignment(lines, key) {
        lines[index] = installed_line.to_string();
        PreviousAssignment {
            present: true,
            raw_line: Some(raw_line),
            index: Some(index),
        }
    } else {
        let index = first_table_index(lines);
        lines.insert(index, installed_line.to_string());
        PreviousAssignment {
            present: false,
            raw_line: None,
            index: None,
        }
    }
}

fn restore_assignment(
    lines: &mut Vec<String>,
    key: &str,
    installed_line: &str,
    previous: &PreviousAssignment,
) -> AppResult<()> {
    let managed_index = top_level_assignment(lines, key)
        .filter(|(_, line)| line.trim() == installed_line.trim())
        .map(|(index, _)| index)
        .ok_or_else(|| {
            AppError::Message(format!(
                "Codex {key} changed after Mnelyra installed Web Models; refusing to overwrite the user's newer config"
            ))
        })?;
    if previous.present {
        let raw = previous.raw_line.clone().ok_or_else(|| {
            AppError::Message(format!(
                "Mnelyra route journal is missing the previous Codex {key} assignment"
            ))
        })?;
        lines[managed_index] = raw;
    } else {
        lines.remove(managed_index);
        if managed_index < lines.len()
            && lines[managed_index].trim().is_empty()
            && managed_index > 0
            && lines[managed_index - 1].trim().is_empty()
        {
            lines.remove(managed_index);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteJournal {
    version: u32,
    config_path: String,
    installed_line: String,
    previous: PreviousAssignment,
    #[serde(default)]
    catalog_installed_line: Option<String>,
    #[serde(default)]
    catalog_previous: Option<PreviousAssignment>,
    #[serde(default)]
    catalog_path: Option<String>,
    line_ending: String,
    trailing_newline: bool,
}

#[derive(Debug, Clone)]
pub(super) struct RouteStatus {
    pub installed: bool,
    pub active: bool,
    pub route_url: String,
    pub errors: Vec<String>,
}

fn route_root(app: &AppHandle) -> AppResult<PathBuf> {
    let root = app.path().app_data_dir().map_err(|error| {
        AppError::Message(format!("could not resolve Mnelyra app data: {error}"))
    })?;
    Ok(root.join("web-models").join("codex-route"))
}

fn journal_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(route_root(app)?.join("journal.json"))
}

fn recovery_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(route_root(app)?.join("journal.recovery.json"))
}

fn legacy_model_catalog_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(route_root(app)?.join(LEGACY_MODEL_CATALOG_FILE))
}

fn managed_line() -> String {
    format!(
        "openai_base_url = {} {MANAGED_COMMENT}",
        serde_json::to_string(ROUTE_URL).expect("static route URL is valid JSON")
    )
}

fn read_text(path: &Path) -> AppResult<String> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| AppError::Message(format!("{} is not UTF-8", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(AppError::Io(error)),
    }
}

fn split_config(text: &str) -> (Vec<String>, String, bool) {
    let line_ending = if text.contains("\r\n") { "\r\n" } else { "\n" }.to_string();
    let trailing_newline = text.ends_with('\n');
    let lines = text
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect::<Vec<_>>();
    (lines, line_ending, trailing_newline)
}

fn serialize_config(lines: &[String], line_ending: &str, trailing_newline: bool) -> String {
    let mut text = lines.join(line_ending);
    if trailing_newline && !lines.is_empty() {
        text.push_str(line_ending);
    }
    text
}

fn first_table_index(lines: &[String]) -> usize {
    lines
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('[') && !trimmed.starts_with("#")
        })
        .unwrap_or(lines.len())
}

fn top_level_assignment(lines: &[String], key: &str) -> Option<(usize, String)> {
    let pattern = Regex::new(&format!(r"^\s*{}\s*=", regex::escape(key))).expect("valid key regex");
    lines
        .iter()
        .take(first_table_index(lines))
        .enumerate()
        .find(|(_, line)| pattern.is_match(line))
        .map(|(index, line)| (index, line.clone()))
}

fn assignment_value(line: &str, key: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r"^\s*{}\s*=\s*(.+?)\s*(?:#.*)?$",
        regex::escape(key)
    ))
    .ok()?;
    let captures = pattern.captures(line)?;
    let raw = captures.get(1)?.as_str().trim();
    serde_json::from_str::<String>(raw).ok()
}

fn load_journal(path: &Path) -> Option<RouteJournal> {
    let raw = fs::read(path).ok()?;
    let journal = serde_json::from_slice::<RouteJournal>(&raw).ok()?;
    (journal.version == JOURNAL_VERSION).then_some(journal)
}

fn current_journal(app: &AppHandle) -> AppResult<Option<RouteJournal>> {
    let primary = journal_path(app)?;
    let recovery = recovery_path(app)?;
    Ok(load_journal(&primary).or_else(|| load_journal(&recovery)))
}

fn write_journal(path: &Path, journal: &RouteJournal) -> AppResult<()> {
    let bytes = serde_json::to_vec_pretty(journal)?;
    atomic_write(path, &bytes)
}

fn remove_journal_pair(app: &AppHandle) -> AppResult<()> {
    for path in [journal_path(app)?, recovery_path(app)?] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::Io(error)),
        }
    }
    Ok(())
}

pub(super) fn status(app: &AppHandle) -> AppResult<RouteStatus> {
    let config_path = codex::config_path().map_err(AppError::Message)?;
    let text = read_text(&config_path)?;
    let (lines, _, _) = split_config(&text);
    let current = top_level_assignment(&lines, "openai_base_url")
        .and_then(|(_, line)| assignment_value(&line, "openai_base_url"));
    let route_active = current.as_deref() == Some(ROUTE_URL);
    let journal = current_journal(app)?;
    let mut errors = Vec::new();
    let installed = if let Some(journal) = journal {
        let same_path = Path::new(&journal.config_path) == config_path;
        if !same_path {
            errors.push("Codex home changed after the Mnelyra route was installed".into());
        }
        same_path
    } else {
        false
    };

    if route_active && !installed {
        errors.push(
            "Codex already points at Mnelyra's local Responses URL, but no Mnelyra route journal exists"
                .into(),
        );
    }
    Ok(RouteStatus {
        installed,
        active: route_active,
        route_url: ROUTE_URL.into(),
        errors,
    })
}

pub(super) fn install(app: &AppHandle) -> AppResult<RouteStatus> {
    let config_path = codex::config_path().map_err(AppError::Message)?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let text = read_text(&config_path)?;
    let (mut lines, line_ending, trailing_newline) = split_config(&text);
    let existing = top_level_assignment(&lines, "openai_base_url");
    let mut existing_journal = current_journal(app)?;

    if let Some(journal) = existing_journal.as_mut() {
        if Path::new(&journal.config_path) != config_path {
            return Err(AppError::Message(
                "CODEX_HOME changed after the Mnelyra route was installed; refusing to modify a second Codex config until the first route is restored"
                    .into(),
            ));
        }
        let route_owned = existing
            .as_ref()
            .is_some_and(|(_, line)| line.trim() == journal.installed_line.trim());
        if !route_owned {
            return Err(AppError::Message(
                "Codex routing changed after Mnelyra installed Web Models; refusing to overwrite the user's newer config"
                    .into(),
            ));
        }

        let mut changed = migrate_legacy_model_selection(&mut lines);
        let mut legacy_catalog_path = None;
        if let (Some(installed_line), Some(previous)) = (
            journal.catalog_installed_line.as_deref(),
            journal.catalog_previous.as_ref(),
        ) {
            restore_assignment(
                &mut lines,
                LEGACY_MODEL_CATALOG_KEY,
                installed_line,
                previous,
            )?;
            legacy_catalog_path = journal.catalog_path.clone();
            journal.catalog_installed_line = None;
            journal.catalog_previous = None;
            journal.catalog_path = None;
            changed = true;
        }
        if changed {
            write_journal(&recovery_path(app)?, journal)?;
            let config =
                serialize_config(&lines, &line_ending, trailing_newline || !lines.is_empty());
            atomic_write(&config_path, config.as_bytes())?;
            write_journal(&journal_path(app)?, journal)?;
            if let Some(path) = legacy_catalog_path {
                let path = PathBuf::from(path);
                if path == legacy_model_catalog_path(app)? {
                    let _ = fs::remove_file(path);
                }
                if let Err(error) = codex::clear_model_cache() {
                    eprintln!("Mnelyra could not invalidate the legacy Codex model cache: {error}");
                }
            }
        }
        return status(app);
    }

    if let Some(current) = existing
        .as_ref()
        .and_then(|(_, line)| assignment_value(line, "openai_base_url"))
    {
        if current == ROUTE_URL && existing_journal.is_none() {
            return Err(AppError::Message(
                "Codex already points at Mnelyra's local Responses URL, but Mnelyra does not own that change. Restore the Codex config manually before installing Web Models."
                    .into(),
            ));
        }
    }

    migrate_legacy_model_selection(&mut lines);
    let installed_line = managed_line();
    let previous = install_assignment(&mut lines, "openai_base_url", &installed_line);

    let journal = RouteJournal {
        version: JOURNAL_VERSION,
        config_path: config_path.to_string_lossy().to_string(),
        installed_line,
        previous,
        catalog_installed_line: None,
        catalog_previous: None,
        catalog_path: None,
        line_ending: line_ending.clone(),
        trailing_newline,
    };

    // Recovery is written before the Codex config so an interrupted install can
    // still be restored deterministically.
    write_journal(&recovery_path(app)?, &journal)?;
    let config = serialize_config(&lines, &line_ending, trailing_newline || !lines.is_empty());
    atomic_write(&config_path, config.as_bytes())?;
    write_journal(&journal_path(app)?, &journal)?;
    status(app)
}

pub(super) fn restore(app: &AppHandle) -> AppResult<RouteStatus> {
    let Some(journal) = current_journal(app)? else {
        return status(app);
    };
    let config_path = PathBuf::from(&journal.config_path);
    let text = read_text(&config_path)?;
    let (mut lines, current_line_ending, current_trailing_newline) = split_config(&text);
    let had_legacy_catalog = journal.catalog_installed_line.is_some();
    if let (Some(installed_line), Some(previous)) = (
        journal.catalog_installed_line.as_deref(),
        journal.catalog_previous.as_ref(),
    ) {
        restore_assignment(
            &mut lines,
            LEGACY_MODEL_CATALOG_KEY,
            installed_line,
            previous,
        )?;
    }
    migrate_legacy_model_selection(&mut lines);
    restore_assignment(
        &mut lines,
        "openai_base_url",
        &journal.installed_line,
        &journal.previous,
    )?;

    let line_ending = if current_line_ending.is_empty() {
        journal.line_ending.as_str()
    } else {
        current_line_ending.as_str()
    };
    let config = serialize_config(
        &lines,
        line_ending,
        current_trailing_newline || journal.trailing_newline,
    );
    atomic_write(&config_path, config.as_bytes())?;
    remove_journal_pair(app)?;
    if let Some(path) = journal.catalog_path.as_deref() {
        let path = PathBuf::from(path);
        if path == legacy_model_catalog_path(app)? {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::Io(error)),
            }
        }
    }
    if had_legacy_catalog {
        if let Err(error) = codex::clear_model_cache() {
            eprintln!("Mnelyra could not invalidate the legacy Codex model cache: {error}");
        }
    }
    status(app)
}

fn atomic_write(target: &Path, content: &[u8]) -> AppResult<()> {
    let parent = target.parent().ok_or_else(|| {
        AppError::Message(format!("{} has no parent directory", target.display()))
    })?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".mnelyra-route-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        atomic_replace(&temp, target)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(AppError::Io)
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_top_level_base_url_assignment() {
        let lines = vec![
            "model = \"gpt-5.6-sol\"".to_string(),
            "openai_base_url = \"https://example.test/v1\"".to_string(),
            "[model_providers.test]".to_string(),
            "openai_base_url = \"https://ignored.test/v1\"".to_string(),
        ];
        let (index, line) = top_level_assignment(&lines, "openai_base_url").expect("assignment");
        assert_eq!(index, 1);
        assert_eq!(
            assignment_value(&line, "openai_base_url").as_deref(),
            Some("https://example.test/v1")
        );
    }

    #[test]
    fn preserves_line_endings_and_trailing_newline() {
        let source = "model = \"a\"\r\n[features]\r\nfoo = true\r\n";
        let (lines, ending, trailing) = split_config(source);
        assert_eq!(ending, "\r\n");
        assert!(trailing);
        assert_eq!(serialize_config(&lines, &ending, trailing), source);
    }

    #[test]
    fn managed_line_is_parseable_and_exact() {
        let line = managed_line();
        assert_eq!(
            assignment_value(&line, "openai_base_url").as_deref(),
            Some(ROUTE_URL)
        );
        assert!(line.ends_with(MANAGED_COMMENT));
    }

    #[test]
    fn route_assignment_round_trips_when_absent() {
        let original = vec![
            "model = \"gpt-5.6-sol\"".to_string(),
            "[features]".to_string(),
            "foo = true".to_string(),
        ];
        let mut lines = original.clone();
        let route_line = managed_line();

        let route_previous = install_assignment(&mut lines, "openai_base_url", &route_line);

        assert_eq!(
            assignment_value(
                &top_level_assignment(&lines, "openai_base_url")
                    .expect("route assignment")
                    .1,
                "openai_base_url"
            )
            .as_deref(),
            Some(ROUTE_URL)
        );
        restore_assignment(&mut lines, "openai_base_url", &route_line, &route_previous)
            .expect("restore route");
        assert_eq!(lines, original);
    }

    #[test]
    fn legacy_web_model_selection_migrates_to_native_sol() {
        let mut lines = vec![
            "model = \"mnelyra-web/high\" # selected by old Web Models".to_string(),
            "model_reasoning_effort = \"high\"".to_string(),
        ];
        assert!(migrate_legacy_model_selection(&mut lines));
        assert_eq!(
            lines[0],
            "model = \"gpt-5.6-sol\" # selected by old Web Models"
        );
        assert_eq!(lines[1], "model_reasoning_effort = \"high\"");
        assert!(!migrate_legacy_model_selection(&mut lines));
    }
}
