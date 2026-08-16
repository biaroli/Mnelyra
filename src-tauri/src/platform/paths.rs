use std::env;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "rootrelay";

pub fn app_config_dir(base: impl AsRef<Path>) -> PathBuf {
    app_config_dir_with_override(base, env::var_os("MNELYRA_DATA_DIR"))
}

fn app_config_dir_with_override(
    base: impl AsRef<Path>,
    configured: Option<impl Into<PathBuf>>,
) -> PathBuf {
    if let Some(configured) = configured {
        let configured = configured.into();
        if configured.is_absolute() {
            return configured;
        }
    }
    let base = base.as_ref();
    base.join(APP_DIR)
}

pub fn resolve_from_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let paths = env::split_paths(&path_var);
    let windows = cfg!(windows);
    let candidates = if windows {
        vec![name.to_string(), format!("{name}.exe")]
    } else {
        vec![name.to_string()]
    };

    for dir in paths {
        for candidate in &candidates {
            let full = dir.join(candidate);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

pub fn append_if_exists(paths: &mut Vec<PathBuf>, candidate: impl AsRef<Path>) {
    let candidate = candidate.as_ref();
    if candidate.is_file() {
        paths.push(candidate.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_absolute_mnelyra_data_dir_wins_without_mutating_process_env() {
        let expected = if cfg!(windows) {
            PathBuf::from(r"C:\mnelyra-test-data")
        } else {
            PathBuf::from("/tmp/mnelyra-test-data")
        };
        assert_eq!(
            app_config_dir_with_override(Path::new("fallback"), Some(expected.clone())),
            expected
        );
    }
}
